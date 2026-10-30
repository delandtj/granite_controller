#!/usr/bin/env python3
"""Netlist equivalence checker (ADR 0002, component 5).

  schcheck.py <reference> <candidate> [--map out.txt]
  schcheck.py --regen <command> <out.kicad_sch>

Each netlist argument is a kicadxml netlist (.xml) or a schematic
(.kicad_sch), which is first exported with kicad-cli into a temp dir.

Equivalence: (ref, pin) -> net is built for both sides, the net rename
map reference -> candidate is derived from the shared (ref, pin) pairs
and must be a bijection; every (ref, pin) must exist on both sides and
agree under the map. Per ref, Value, Footprint, LCSC, MPN, DNP and
Description must match. Prints a summary and the rename map as
"old<TAB>new" lines (changed names only; tab because hierarchical net
names may contain spaces), also written to --map.
Exit 0 only when fully equivalent.

--regen runs <command> (through the shell) twice, copying
<out.kicad_sch> after each run, and fails unless both copies are
byte-identical.

Python 3 stdlib only.
"""

import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

FIELDS = ("value", "footprint", "LCSC", "MPN", "DNP", "description")


def die(msg):
    print("schcheck: " + msg, file=sys.stderr)
    sys.exit(2)


def export_netlist(sch, tmpdir):
    out = os.path.join(tmpdir, os.path.basename(sch) + ".xml")
    cmd = ["kicad-cli", "sch", "export", "netlist", "--format", "kicadxml",
           "--output", out, sch]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(out):
        die("netlist export failed for %s:\n%s%s" % (sch, r.stdout, r.stderr))
    return out


def load(path, tmpdir):
    """Return (pins, parts) for a netlist or schematic path.

    pins: {(ref, pin): net}
    parts: {ref: {field: value}}
    """
    if path.endswith(".kicad_sch"):
        path = export_netlist(path, tmpdir)
    elif not path.endswith(".xml"):
        die("unsupported input (want .xml or .kicad_sch): " + path)
    try:
        root = ET.parse(path).getroot()
    except (OSError, ET.ParseError) as e:
        die("cannot read %s: %s" % (path, e))

    parts = {}
    for comp in root.iter("comp"):
        ref = comp.get("ref")
        fields = {f.get("name"): (f.text or "")
                  for f in comp.findall("fields/field")}
        props = {p.get("name"): p.get("value")
                 for p in comp.findall("property")}
        rec = {
            "value": comp.findtext("value", ""),
            "footprint": comp.findtext("footprint", ""),
            "LCSC": fields.get("LCSC", props.get("LCSC", "")) or "",
            "MPN": fields.get("MPN", props.get("MPN", "")) or "",
            "DNP": "yes" if "dnp" in props else "no",
            "description": comp.findtext("description", ""),
        }
        if ref in parts:
            die("%s: duplicate component ref %s" % (path, ref))
        parts[ref] = rec

    pins = {}
    nets = root.find("nets")
    if nets is None:
        die("%s: no <nets> section" % path)
    for net in nets.findall("net"):
        name = net.get("name")
        for node in net.findall("node"):
            key = (node.get("ref"), node.get("pin"))
            if key in pins and pins[key] != name:
                die("%s: %s pin %s on two nets: %s, %s"
                    % (path, key[0], key[1], pins[key], name))
            pins[key] = name
    return pins, parts


def pinkey(k):
    return "%s.%s" % k


def compare(ref_path, cand_path, map_path):
    tmpdir = tempfile.mkdtemp(prefix="schcheck-")
    try:
        rpins, rparts = load(ref_path, tmpdir)
        cpins, cparts = load(cand_path, tmpdir)
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)

    errors = []

    # (ref, pin) presence
    only_r = sorted(set(rpins) - set(cpins))
    only_c = sorted(set(cpins) - set(rpins))
    for k in only_r:
        errors.append("pin %s (net %s) only in reference"
                      % (pinkey(k), rpins[k]))
    for k in only_c:
        errors.append("pin %s (net %s) only in candidate"
                      % (pinkey(k), cpins[k]))

    # Net map from shared pins
    fwd = {}   # ref net -> {cand net: [pins]}
    rev = {}   # cand net -> {ref net: [pins]}
    for k in sorted(set(rpins) & set(cpins)):
        a, b = rpins[k], cpins[k]
        fwd.setdefault(a, {}).setdefault(b, []).append(k)
        rev.setdefault(b, {}).setdefault(a, []).append(k)

    def fmt(group):
        return "; ".join("%s <- %s" % (n, " ".join(pinkey(k) for k in ks))
                         for n, ks in sorted(group.items()))

    moved = set()
    for a in sorted(fwd):
        if len(fwd[a]) > 1:
            errors.append("reference net %s split across candidate nets: %s"
                          % (a, fmt(fwd[a])))
            main = max(fwd[a], key=lambda n: (len(fwd[a][n]), n))
            moved.update(k for n, ks in fwd[a].items() if n != main
                         for k in ks)
    for b in sorted(rev):
        if len(rev[b]) > 1:
            errors.append("candidate net %s merges reference nets: %s"
                          % (b, fmt(rev[b])))
            main = max(rev[b], key=lambda n: (len(rev[b][n]), n))
            moved.update(k for n, ks in rev[b].items() if n != main
                         for k in ks)
    for k in sorted(moved):
        errors.append("pin %s disagrees: reference net %s, candidate net %s"
                      % (pinkey(k), rpins[k], cpins[k]))

    # Rename map: only the clean one-to-one pairs.
    rename = {}
    for a, targets in fwd.items():
        if len(targets) == 1:
            b = next(iter(targets))
            if len(rev[b]) == 1:
                rename[a] = b

    # Part fields
    for r in sorted(set(rparts) - set(cparts)):
        errors.append("part %s only in reference" % r)
    for r in sorted(set(cparts) - set(rparts)):
        errors.append("part %s only in candidate" % r)
    for r in sorted(set(rparts) & set(cparts)):
        for f in FIELDS:
            if rparts[r][f] != cparts[r][f]:
                errors.append("part %s field %s: %r -> %r"
                              % (r, f, rparts[r][f], cparts[r][f]))

    changed = sorted((a, b) for a, b in rename.items() if a != b)
    rnets = set(rpins.values())
    cnets = set(cpins.values())
    print("reference: %s" % ref_path)
    print("candidate: %s" % cand_path)
    print("parts: %d reference, %d candidate" % (len(rparts), len(cparts)))
    print("nets: %d reference, %d candidate" % (len(rnets), len(cnets)))
    print("pins: %d reference, %d candidate" % (len(rpins), len(cpins)))
    print("renamed nets: %d" % len(changed))
    # Tab-separated: hierarchical net names may contain spaces.
    lines = ["%s\t%s" % ab for ab in changed]
    if lines:
        print("rename map (old<TAB>new):")
        for ln in lines:
            print("  " + ln)
    if map_path:
        with open(map_path, "w") as fh:
            fh.write("".join(ln + "\n" for ln in lines))

    if errors:
        print("NOT EQUIVALENT: %d difference(s)" % len(errors))
        for e in errors:
            print("  " + e)
        return 1
    print("EQUIVALENT")
    return 0


def regen(command, out):
    tmpdir = tempfile.mkdtemp(prefix="schcheck-regen-")
    copies = []
    try:
        for i in (1, 2):
            r = subprocess.run(command, shell=True)
            if r.returncode != 0:
                print("regen: run %d of %r failed (exit %d)"
                      % (i, command, r.returncode))
                return 1
            if not os.path.exists(out):
                print("regen: run %d did not produce %s" % (i, out))
                return 1
            dst = os.path.join(tmpdir, "run%d" % i)
            shutil.copyfile(out, dst)
            copies.append(dst)
        with open(copies[0], "rb") as f1, open(copies[1], "rb") as f2:
            a, b = f1.read(), f2.read()
        if a != b:
            al, bl = a.splitlines(), b.splitlines()
            n = next((i for i, (x, y) in enumerate(zip(al, bl)) if x != y),
                     min(len(al), len(bl)))
            print("regen: NOT DETERMINISTIC: %s differs between runs "
                  "(first difference at line %d)" % (out, n + 1))
            return 1
        print("regen: deterministic, %s byte-identical across 2 runs "
              "(%d bytes)" % (out, len(a)))
        return 0
    finally:
        shutil.rmtree(tmpdir, ignore_errors=True)


def main():
    if len(sys.argv) >= 2 and sys.argv[1] == "--regen":
        if len(sys.argv) != 4:
            die("usage: schcheck.py --regen <command> <out.kicad_sch>")
        return regen(sys.argv[2], sys.argv[3])
    ap = argparse.ArgumentParser(description="KiCad netlist equivalence "
                                 "checker")
    ap.add_argument("reference")
    ap.add_argument("candidate")
    ap.add_argument("--map", help="write the rename map (old<TAB>new) here")
    a = ap.parse_args()
    return compare(a.reference, a.candidate, a.map)


if __name__ == "__main__":
    sys.exit(main())
