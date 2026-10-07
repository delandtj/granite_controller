#!/usr/bin/env python3
"""First grouped placement for the 230 x 40 mm controller strip.

Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Zones along x (y = 0 is the frame-top edge):
  0-28 power | 28-60 Ethernet | 60-100 MCU/USB | 100-130 probes + expansion
  | 130-160 expanders + optos | 160-220 node channels (4 columns, ch1-4 top
  row, ch5-8 bottom row).
Big parts are anchored explicitly; the rest of each group is packed
greedily (bottom-left fill, 0.4 mm courtyard gap) into the group's regions.
This is a starting point for hand placement, not a final layout.
Same scratch-dir rule as mkboard/outline.
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
GAP = 0.4
STEP = 0.5

board = pcbnew.LoadBoard(src)
_fps = board.Footprints()
FP = {_fps[i].GetReference(): _fps[i] for i in range(len(_fps))}


def mm(v):
    return pcbnew.ToMM(v)


def cbox(fp):
    """Courtyard (or body) box as (x0, y0, x1, y1) in mm."""
    cy = fp.GetCourtyard(pcbnew.F_CrtYd)
    b = cy.BBox() if cy.OutlineCount() else fp.GetBoundingBox(False)
    return mm(b.GetLeft()), mm(b.GetTop()), mm(b.GetRight()), mm(b.GetBottom())


placed = []      # obstacle boxes


def hit(b):
    return any(not (b[2] + GAP <= o[0] or o[2] + GAP <= b[0] or
                    b[3] + GAP <= o[1] or o[3] + GAP <= b[1]) for o in placed)


def put(ref, x, y, rot=0):
    fp = FP[ref]
    fp.SetOrientationDegrees(rot)
    fp.SetPosition(pcbnew.VECTOR2I_MM(x, y))
    placed.append(cbox(fp))


def pack(refs, regions, rot=0):
    """Greedy bottom-left fill of refs into the first region that fits."""
    left = []
    for ref in refs:
        fp = FP[ref]
        fp.SetOrientationDegrees(rot)
        fp.SetPosition(pcbnew.VECTOR2I_MM(0, 0))
        x0, y0, x1, y1 = cbox(fp)
        w, h = x1 - x0, y1 - y0
        done = False
        for rx0, ry0, rx1, ry1 in regions:
            y = ry0
            while not done and y + h <= ry1 + 1e-6:
                x = rx0
                while x + w <= rx1 + 1e-6:
                    b = (x, y, x + w, y + h)
                    if not hit(b):
                        fp.SetPosition(pcbnew.VECTOR2I_MM(x - x0, y - y0))
                        placed.append(b)
                        done = True
                        break
                    x += STEP
                y += STEP
            if done:
                break
        if not done:
            left.append(ref)
    return left


def sheet(name):
    return sorted((r for r, f in FP.items() if f.GetSheetname() == name),
                  key=lambda r: (r.rstrip("0123456789"), int("0" + r[len(r.rstrip("0123456789")):])))


for h in ("H1", "H2", "H3", "H4"):
    placed.append(cbox(FP[h]))

# ---------------------------------------------------------------- anchors
put("J3", 3.0, 18.0)                       # 19 V in, left edge
put("J1", 45.0, 17.8, 180)                 # RJ45, opening flush with the top edge
put("U3", 38.0, 31.0)                      # W5500
put("U1", 80.0, 33.25, 180)                # WROOM, antenna over the bottom edge
put("J2", 66.5, 3.675, 180)                # USB-C, opening on the top edge
put("J4", 76.0, 3.0)
put("J5", 86.0, 3.0)
put("J6", 103.0, 3.0)
put("J7", 112.5, 3.0)
put("J9", 122.0, 3.0)
put("J8", 102.0, 23.5)
put("J10", 110.0, 36.2)
put("U14", 140.0, 10.0, 90)
put("U15", 140.0, 30.0, 90)
put("U16", 155.0, 10.0)
put("U17", 155.0, 30.0)
COLS = [160.0 + 15.0 * i for i in range(4)]
for i, x0 in enumerate(COLS):
    put(f"J{11 + i}", x0 + 3.5, 3.0)       # ch1-4 top row
    put(f"J{15 + i}", x0 + 3.5, 36.2)      # ch5-8 bottom row

anchored = {"J3", "J1", "U3", "U1", "J2", "J4", "J5", "J6", "J7", "J9", "J8", "J10",
            "U14", "U15", "U16", "U17", "H1", "H2", "H3", "H4"} | {f"J{n}" for n in range(11, 19)}

# ---------------------------------------------------------------- node channels
left = []
for ch in range(1, 9):
    i = (ch - 1) % 4
    x0 = COLS[i]
    band = (x0, 7.5, x0 + 15.0, 19.5) if ch <= 4 else (x0, 20.5, x0 + 15.0, 33.0)
    refs = [f"K{2 * ch - 1}", f"K{2 * ch}", f"R{100 + 2 * ch - 1}", f"R{100 + 2 * ch}",
            f"R{200 + 2 * ch - 1}", f"R{200 + 2 * ch}", f"R{46 + ch}", f"R{38 + ch}"]
    left += pack(refs, [band])
    anchored |= set(refs)

# ---------------------------------------------------------------- groups
groups = [
    ("Power", [(8, 1, 28, 39)]),
    ("Ethernet", [(51, 1, 60, 24), (28, 23, 60, 39.5)]),
    ("MCU", [(60, 9, 100, 20), (60, 20, 70, 39.5), (90, 20, 100, 39.5)]),
    ("Expansion header", [(100, 7, 130, 21.5), (107, 21.5, 130, 33.5)]),
    ("Relay channels", [(130, 16.5, 160, 23.5), (130, 0.5, 160, 39.5)]),
]
for name, regions in groups:
    refs = [r for r in sheet(name) if r not in anchored]
    left += pack(refs, regions)

board.Save(out)
print(f"placed; did not fit: {left}")
