#!/usr/bin/env python3
"""Add the board outline and corner mounting holes to a .kicad_pcb.

Usage: outline.py <in.kicad_pcb> <out.kicad_pcb> [length_mm] [height_mm]

Rectangle from (0, 0) to (length, height) on Edge.Cuts, 1 mm corner radius,
plus four M3 (3.2 mm NPTH) holes inset 4 mm from each corner as board-only
footprints H1-H4. Existing Edge.Cuts items and H1-H4 are replaced, so it is
safe to re-run after tools/mkboard.py. Write <out> to a scratch directory and
copy only the .kicad_pcb back (pcbnew's Save rewrites the .kicad_pro).
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
L = float(sys.argv[3]) if len(sys.argv) > 3 else 180.0
H = float(sys.argv[4]) if len(sys.argv) > 4 else 40.0
R = 1.0           # corner radius
INSET = 4.0       # hole centre from each edge
HOLE_LIB = "/usr/share/kicad/footprints/MountingHole.pretty"
HOLE_FP = "MountingHole_3.2mm_M3"

board = pcbnew.LoadBoard(src)
# index the SWIG containers: their iterators break on Python 3.14
drw = board.Drawings()
for d in [drw[i] for i in range(len(drw))]:
    if d.GetLayer() == pcbnew.Edge_Cuts:
        board.Delete(d)
fps = board.Footprints()
for fp in [fps[i] for i in range(len(fps))]:
    if fp.GetReference() in ("H1", "H2", "H3", "H4"):
        board.Delete(fp)


def mm(x, y):
    return pcbnew.VECTOR2I_MM(x, y)


def seg(x1, y1, x2, y2):
    s = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_SEGMENT)
    s.SetStart(mm(x1, y1))
    s.SetEnd(mm(x2, y2))
    s.SetLayer(pcbnew.Edge_Cuts)
    s.SetWidth(pcbnew.FromMM(0.1))
    board.Add(s)


def arc(cx, cy, sx, sy, angle):
    a = pcbnew.PCB_SHAPE(board, pcbnew.SHAPE_T_ARC)
    a.SetCenter(mm(cx, cy))
    a.SetStart(mm(sx, sy))
    a.SetArcAngleAndEnd(pcbnew.EDA_ANGLE(angle, pcbnew.DEGREES_T), True)
    a.SetLayer(pcbnew.Edge_Cuts)
    a.SetWidth(pcbnew.FromMM(0.1))
    board.Add(a)


seg(R, 0, L - R, 0)
seg(L, R, L, H - R)
seg(L - R, H, R, H)
seg(0, H - R, 0, R)
arc(L - R, R, L - R, 0, 90)
arc(L - R, H - R, L, H - R, 90)
arc(R, H - R, R, H, 90)
arc(R, R, 0, R, 90)

for i, (x, y) in enumerate(((INSET, INSET), (L - INSET, INSET),
                            (L - INSET, H - INSET), (INSET, H - INSET))):
    fp = pcbnew.FootprintLoad(HOLE_LIB, HOLE_FP)
    fp.SetReference(f"H{i + 1}")
    fp.Reference().SetVisible(False)
    fp.SetFPID(pcbnew.LIB_ID("MountingHole", HOLE_FP))
    fp.SetPosition(mm(x, y))
    fp.SetAttributes(pcbnew.FP_BOARD_ONLY | pcbnew.FP_EXCLUDE_FROM_BOM
                     | pcbnew.FP_EXCLUDE_FROM_POS_FILES)
    board.Add(fp)

board.Save(out)
print(f"outline {L} x {H} mm, holes M3 at {INSET} mm inset -> {out}")
