#!/usr/bin/env python3
"""First grouped placement for the 250 x 50 mm controller strip.

Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Zones along x (y = 0 is the frame-top edge):
  0-31 power | 32-67 Ethernet | 68-112 MCU/USB | 112-146 probes + expansion
  | 146-180 expanders + optos | 180-244 node channels (4 columns, ch1-4 top
  row, ch5-8 bottom row).
Big parts are anchored explicitly; the rest of each group is packed
greedily (bottom-left fill, GAP mm courtyard gap) into the group's regions.
This is a starting point for hand placement, not a final layout.
Same scratch-dir rule as mkboard/outline.
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
GAP = 0.8         # courtyard-to-courtyard gap, leaves room to route
H = 50.0          # board height (y), see tools/outline.py
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
put("J3", 3.0, 23.0)                       # 19 V in, left edge
put("J1", 49.0, 17.8, 180)                 # RJ45, opening flush with the top edge
# W5500 rotated so its PHY pins (1, 2, 5, 6) face the RJ45's signal pins
put("U3", 46.0, 36.0, 270)
# Ethernet chain, in line from J1 (pins at y 17.8/20.3) down to U3 (pins at y 31.8):
# row A under J1: RX coupling caps, RX 49.9R to RCT, TX 49.9R pull-ups
for ref, x in (("C29", 42.6), ("R23", 43.9), ("R22", 45.2), ("C28", 46.5),
               ("R21", 47.8), ("R20", 49.1)):
    put(ref, x, 24.0, 90)
# row B: 33R series resistors (each pair crosses once, at the W5500 end)
for ref, x in (("R15", 43.2), ("R12", 44.6), ("R11", 46.0), ("R10", 47.4), ("R9", 48.8)):
    put(ref, x, 27.4, 90)
# centre taps and shield next to the jack's left pins
for ref, x, y in (("C35", 40.4, 24.0), ("C34", 39.1, 24.0), ("R26", 37.8, 24.0),
                  ("C36", 34.5, 28.0), ("R28", 37.6, 27.4)):
    put(ref, x, y, 90)
# crystal under XI/XO (pins 30/31, bottom edge of U3)
put("Y2", 46.0, 44.8)
put("C26", 42.6, 44.8, 90)
put("C27", 49.4, 44.8, 90)
put("R29", 46.0, 47.6)
# TOCAP / 1V2O on the left side, next to pins 20 / 22
put("C30", 39.2, 36.8)
put("C31", 39.2, 38.4)
put("U1", 90.0, H - 6.75, 180)             # WROOM, antenna over the bottom edge
put("J2", 74.5, 3.675, 180)                # USB-C, opening on the top edge
put("J4", 84.0, 3.0)
put("J5", 94.0, 3.0)
put("J6", 115.0, 3.0)
put("J7", 124.5, 3.0)
put("J9", 134.0, 3.0)
put("J8", 114.0, H - 16.5)
put("J10", 122.0, H - 3.8)
put("U14", 157.0, 11.0, 90)
put("U15", 157.0, H - 11.0, 90)
put("U16", 173.0, 11.0)
put("U17", 173.0, H - 11.0)
COLS = [180.0 + 16.0 * i for i in range(4)]
for i, x0 in enumerate(COLS):
    put(f"J{11 + i}", x0 + 3.5, 3.0)       # ch1-4 top row
    put(f"J{15 + i}", x0 + 3.5, H - 3.8)   # ch5-8 bottom row

anchored = {"C29", "R23", "R22", "C28", "R21", "R20", "R15", "R12", "R11", "R10", "R9",
            "C35", "C34", "R26", "C36", "R28", "Y2", "C26", "C27", "R29", "C30", "C31",
            "J3", "J1", "U3", "U1", "J2", "J4", "J5", "J6", "J7", "J9", "J8", "J10",
            "U14", "U15", "U16", "U17", "H1", "H2", "H3", "H4"} | {f"J{n}" for n in range(11, 19)}

# ---------------------------------------------------------------- node channels
left = []
for ch in range(1, 9):
    i = (ch - 1) % 4
    x0 = COLS[i]
    # label lanes: 6.3-8.6 under the top connectors, H-8.4..H-6 over the bottom ones
    band = (x0, 8.6, x0 + 16.0, H / 2 - 0.5) if ch <= 4 else (x0, H / 2 + 0.5, x0 + 16.0, H - 8.4)
    refs = [f"K{2 * ch - 1}", f"K{2 * ch}", f"R{100 + 2 * ch - 1}", f"R{100 + 2 * ch}",
            f"R{200 + 2 * ch - 1}", f"R{200 + 2 * ch}", f"R{46 + ch}", f"R{38 + ch}"]
    left += pack(refs, [band])
    anchored |= set(refs)

# ---------------------------------------------------------------- groups
groups = [
    ("Power", [(8, 1, 31, H - 1)]),
    ("Ethernet", [(32, 31.5, 40.0, H - 0.5), (51.8, 24, 67, H - 0.5), (54.6, 1, 67, 23)]),
    ("MCU", [(68, 9, 112, 28), (68, 28, 80, H - 0.5), (100, 28, 112, H - 0.5)]),
    ("Expansion header", [(112, 8.6, 146, H - 18.8), (119, H - 18.8, 146, H - 7.5)]),  # 7-8.6: label lane under J6/J7/J9
    ("Relay channels", [(146, 19, 180, H - 19), (146, 0.5, 180, H - 0.5)]),
]
for name, regions in groups:
    refs = [r for r in sheet(name) if r not in anchored]
    left += pack(refs, regions)

board.Save(out)
print(f"placed; did not fit: {left}")
