#!/usr/bin/env python3
"""Capture screenshots and raw waveforms from the Siglent SDS1202X-E over LAN.

Talks raw SCPI on TCP port 5025 (no VISA needed). Each capture goes into its
own timestamped folder:

    screen.png    the scope screen (screen.bmp if Pillow is missing)
    c1.bin c2.bin raw 8-bit ADC codes, one signed byte per sample
    meta.json     scope settings and the scaling to turn codes into volts
    c1.csv ...    time,volts (only with --csv; large at deep memory)

Usage:
    tools/sds_capture.py                       grab what is on screen now
    tools/sds_capture.py --single -l preon     arm single, wait for trigger, grab
    tools/sds_capture.py --single --repeat 20 -l latch
                                               20 armed captures in a row, one
                                               folder each (power-cycle between)
    tools/sds_capture.py --setup "TDIV 1MS" --setup "C2:VDIV 5V" --single

Scaling (SDS1000X-E programming guide):
    volts = code * vdiv / 25 - offset
    time  = -tdiv * 14 / 2 - trdl + index / sara
The time formula is only checked at zero trigger delay.

load(path) reads a capture folder back as {channel: (time, volts)} lists.
"""

import argparse
import datetime
import json
import os
import re
import socket
import sys
import time
from pathlib import Path

DEFAULT_HOST = os.environ.get("SDS_HOST", "192.168.64.163")
PORT = 5025
REPO = Path(__file__).resolve().parent.parent
DEFAULT_OUT = REPO / "bench" / "captures"
GRID_X = 14       # horizontal divisions
CODES_PER_DIV = 25


class Scope:
    def __init__(self, host, timeout=10.0):
        self.sock = socket.create_connection((host, PORT), timeout=timeout)
        self.buf = b""

    def close(self):
        self.sock.close()

    def write(self, cmd):
        self.sock.sendall(cmd.encode() + b"\n")

    def _fill(self):
        chunk = self.sock.recv(1 << 20)
        if not chunk:
            raise ConnectionError("scope closed the connection")
        self.buf += chunk

    def _take(self, n):
        while len(self.buf) < n:
            self._fill()
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def readline(self):
        while b"\n" not in self.buf:
            self._fill()
        line, self.buf = self.buf.split(b"\n", 1)
        return line.decode(errors="replace").strip()

    def drain(self):
        """Drop anything already waiting on the socket."""
        self.buf = b""
        self.sock.setblocking(False)
        try:
            while self.sock.recv(1 << 20):
                pass
        except BlockingIOError:
            pass
        finally:
            self.sock.setblocking(True)

    def query(self, cmd):
        """Reply with the echoed header removed: 'C1:VDIV 2.00E-03V' -> '2.00E-03V'.

        Firmware 1.3.26 sometimes glues stale output in front of a reply
        (after reading an empty waveform), so the reply is taken after the
        last occurrence of the command's own header, skipping lines without it.
        """
        self.drain()
        self.write(cmd)
        if cmd.startswith("*"):
            return self.readline()
        key = cmd.split("?")[0].split(" ")[0].split(":")[-1] + " "
        while True:
            line = self.readline()
            if key in line:
                return line.rsplit(key, 1)[1].strip()

    def qfloat(self, cmd):
        m = re.match(r"[-+]?\d*\.?\d+(?:[eE][-+]?\d+)?", self.query(cmd))
        if not m:
            raise ValueError(f"{cmd}: no number in reply")
        return float(m.group(0))

    def screenshot(self):
        self.write("SCDP")
        head = self._take(6)
        if head[:2] != b"BM":
            raise ValueError(f"SCDP: not a BMP ({head!r})")
        size = int.from_bytes(head[2:6], "little")
        return head + self._take(size - 6)

    def waveform(self, ch):
        """Raw int8 codes of channel ch (1 or 2), all points in memory.

        Returns b"" when the scope holds no acquired frame (it answers
        '#9000000000'); after that the next reply carries stale output,
        which query() skips.
        """
        self.drain()
        self.write("WFSU SP,0,NP,0,FP,0")
        self.write(f"C{ch}:WF? DAT2")
        # Reply: 'C1:WF DAT2,#9000001400<data>\n\n'
        marker = b"WF DAT2,#9"
        while marker not in self.buf or len(self.buf) < self.buf.index(marker) + len(marker) + 9:
            self._fill()
        self.buf = self.buf[self.buf.index(marker) + len(marker):]
        n = int(self._take(9))
        data = self._take(n)
        self._take(2)  # trailing '\n\n'
        return data


def settings(sc, channels):
    meta = {
        "idn": sc.query("*IDN?"),
        "time": datetime.datetime.now().isoformat(timespec="seconds"),
        "tdiv": sc.qfloat("TDIV?"),
        "trdl": sc.qfloat("TRDL?"),
        "sara": sc.qfloat("SARA?"),
        "trigger_mode": sc.query("TRMD?"),
        "trigger": sc.query("TRSE?"),
        "channels": {},
    }
    for ch in channels:
        meta["channels"][f"c{ch}"] = {
            "vdiv": sc.qfloat(f"C{ch}:VDIV?"),
            "offset": sc.qfloat(f"C{ch}:OFST?"),
            "probe": sc.query(f"C{ch}:ATTN?"),
            "coupling": sc.query(f"C{ch}:CPL?"),
        }
    return meta


def scale(codes, ch_meta, meta):
    k = ch_meta["vdiv"] / CODES_PER_DIV
    t0 = -meta["tdiv"] * GRID_X / 2 - meta["trdl"]
    dt = 1.0 / meta["sara"]
    volts = [(c - 256 if c > 127 else c) * k - ch_meta["offset"] for c in codes]
    times = [t0 + i * dt for i in range(len(volts))]
    return times, volts


def load(path):
    """Read a capture folder back: {'c1': (times, volts), ...}."""
    path = Path(path)
    meta = json.loads((path / "meta.json").read_text())
    out = {}
    for name, ch_meta in meta["channels"].items():
        if not ch_meta.get("points"):
            continue
        codes = (path / f"{name}.bin").read_bytes()
        out[name] = scale(codes, ch_meta, meta)
    return out


def wait_trigger(sc, timeout):
    """Poll the acquisition status until the single shot has fired."""
    t_end = None if timeout is None else time.monotonic() + timeout
    while True:
        if sc.query("SAST?").lower().startswith("stop"):
            return True
        if t_end is not None and time.monotonic() > t_end:
            return False
        time.sleep(0.1)


def capture(sc, out_dir, label, want_csv):
    channels = [ch for ch in (1, 2) if sc.query(f"C{ch}:TRA?").upper() == "ON"]
    meta = settings(sc, channels)
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S")
    base = out_dir / (f"{stamp}-{label}" if label else stamp)
    folder, n = base, 1
    while folder.exists():
        folder = base.with_name(f"{base.name}_{n}")
        n += 1
    folder.mkdir(parents=True)

    bmp = sc.screenshot()
    try:
        import io
        from PIL import Image
        Image.open(io.BytesIO(bmp)).save(folder / "screen.png")
    except ImportError:
        (folder / "screen.bmp").write_bytes(bmp)

    for ch in channels:
        codes = sc.waveform(ch)
        meta["channels"][f"c{ch}"]["points"] = len(codes)
        if not codes:
            print(f"  c{ch}: scope returned no waveform data (no acquired frame?)",
                  file=sys.stderr)
            continue
        (folder / f"c{ch}.bin").write_bytes(codes)
        if want_csv:
            t, v = scale(codes, meta["channels"][f"c{ch}"], meta)
            with open(folder / f"c{ch}.csv", "w") as f:
                f.write("time_s,volts\n")
                f.writelines(f"{a:.9e},{b:.6e}\n" for a, b in zip(t, v))

    if label:
        meta["label"] = label
    (folder / "meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    pts = ", ".join(f"c{ch} {meta['channels'][f'c{ch}']['points']} pts" for ch in channels)
    print(f"saved {folder}  ({pts or 'no channels on'})")
    return folder


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--host", default=DEFAULT_HOST, help="scope IP (env SDS_HOST)")
    ap.add_argument("-o", "--out", type=Path, default=DEFAULT_OUT,
                    help=f"output root (default {DEFAULT_OUT.relative_to(REPO)})")
    ap.add_argument("-l", "--label", default="", help="suffix for the folder name")
    ap.add_argument("--single", action="store_true",
                    help="arm a single-shot trigger and wait for it before grabbing")
    ap.add_argument("--repeat", type=int, default=1, help="number of armed captures")
    ap.add_argument("--timeout", type=float, default=None,
                    help="seconds to wait for each trigger (default: forever)")
    ap.add_argument("--setup", action="append", default=[], metavar="SCPI",
                    help="raw SCPI command sent before the first capture (repeatable)")
    ap.add_argument("--csv", action="store_true", help="also write time,volts CSV per channel")
    args = ap.parse_args()

    if args.repeat > 1 and not args.single:
        ap.error("--repeat needs --single")
    label = re.sub(r"[^\w.-]+", "_", args.label)

    sc = Scope(args.host)
    try:
        for cmd in args.setup:
            sc.write(cmd)
        if args.setup:
            sc.query("*OPC?")
        for i in range(args.repeat):
            tag = f"{label}-{i + 1:03d}" if args.repeat > 1 else label
            if args.single:
                sc.write("TRMD SINGLE")
                sc.write("ARM")
                sc.query("*OPC?")
                print(f"[{i + 1}/{args.repeat}] armed, waiting for trigger ...", flush=True)
                if not wait_trigger(sc, args.timeout):
                    print("  no trigger within timeout, skipped", file=sys.stderr)
                    continue
            capture(sc, args.out, tag, args.csv)
    except KeyboardInterrupt:
        print("\ninterrupted", file=sys.stderr)
        return 130
    finally:
        sc.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
