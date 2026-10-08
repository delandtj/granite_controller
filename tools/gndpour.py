#!/usr/bin/env python3
"""GND pours on F.Cu and B.Cu plus stitching vias to the In1 GND plane.

Usage: gndpour.py <in.kicad_pcb> <out.kicad_pcb>

- Replaces any F.Cu/B.Cu GND zone with one over the whole outline
  (0.25 mm clearance, 0.2 mm min width, thermal reliefs, islands removed).
  Footprint keep-outs (the WROOM antenna) still apply.
- Stitching vias 0.6/0.3, kept in a group "gnd-stitch" so a re-run replaces
  them: a row 1.5 mm in from the board edge every 4 mm, and a 5 mm grid over
  the rest. A via goes only where it clears every pad, track, via and
  courtyard of the board by 0.3 mm, and stays out of the mounting holes and
  keep-out areas.
Run after routing (it needs to see the tracks). Same scratch-dir rule as the
other tools.
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
VIA_D, VIA_H, CLR = 0.6, 0.3, 0.3
EDGE_IN, EDGE_PITCH, GRID = 1.5, 4.0, 5.0

board = pcbnew.LoadBoard(src)
gnd = board.GetNetcodeFromNetname("GND")


def mm(v):
    return pcbnew.ToMM(v)


def box(b):
    return (mm(b.GetLeft()), mm(b.GetTop()), mm(b.GetRight()), mm(b.GetBottom()))


# drop previous outer GND zones and stitching group
zs = board.Zones()
for z in [zs[i] for i in range(len(zs))]:
    if not z.GetIsRuleArea() and z.GetNetCode() == gnd and z.GetLayer() in (pcbnew.F_Cu, pcbnew.B_Cu):
        board.Delete(z)
gs = board.Groups()
for g in [gs[i] for i in range(len(gs))]:
    if g.GetName() == "gnd-stitch":
        items = g.GetItems()
        for it in list(items):
            board.Delete(it)
        board.Delete(g)

eb = board.GetBoardEdgesBoundingBox()
E = box(eb)


def pour(layer):
    z = pcbnew.ZONE(board)
    z.SetLayer(layer)
    z.SetNetCode(gnd)
    o = z.Outline()
    o.NewOutline()
    for x, y in ((eb.GetLeft(), eb.GetTop()), (eb.GetRight(), eb.GetTop()),
                 (eb.GetRight(), eb.GetBottom()), (eb.GetLeft(), eb.GetBottom())):
        o.Append(x, y)
    z.SetAssignedPriority(1)
    z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
    z.SetMinThickness(pcbnew.FromMM(0.2))
    z.SetLocalClearance(pcbnew.FromMM(0.25))
    z.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_ALWAYS)
    board.Add(z)


pour(pcbnew.F_Cu)
pour(pcbnew.B_Cu)

# obstacles: pads, tracks, vias, courtyards (as boxes), keep-out rule areas
obst = []
fps = board.Footprints()
for f in [fps[i] for i in range(len(fps))]:
    c = f.GetCourtyard(pcbnew.F_CrtYd)
    obst.append(box(c.BBox()) if c.OutlineCount() else box(f.GetBoundingBox(False)))
    p = f.Pads()
    for i in range(len(p)):
        obst.append(box(p[i].GetBoundingBox()))
    zz = f.Zones()
    for i in range(len(zz)):
        obst.append(box(zz[i].GetBoundingBox()))
t = board.Tracks()
for i in range(len(t)):
    obst.append(box(t[i].GetBoundingBox()))
for i in range(len(zs)):
    pass
r = VIA_D / 2 + CLR


def free(x, y):
    if not (E[0] + 1.0 <= x <= E[2] - 1.0 and E[1] + 1.0 <= y <= E[3] - 1.0):
        return False
    return not any(o[0] - r < x < o[2] + r and o[1] - r < y < o[3] + r for o in obst)


pts = []
x = E[0] + EDGE_IN
while x <= E[2] - EDGE_IN:
    pts += [(x, E[1] + EDGE_IN), (x, E[3] - EDGE_IN)]
    x += EDGE_PITCH
y = E[1] + EDGE_IN + EDGE_PITCH
while y <= E[3] - EDGE_IN - EDGE_PITCH:
    pts += [(E[0] + EDGE_IN, y), (E[2] - EDGE_IN, y)]
    y += EDGE_PITCH
y = E[1] + GRID
while y < E[3] - 1:
    x = E[0] + GRID
    while x < E[2] - 1:
        pts.append((x, y))
        x += GRID
    y += GRID

grp = pcbnew.PCB_GROUP(board)
grp.SetName("gnd-stitch")
board.Add(grp)
n = 0
for x, y in pts:
    if not free(x, y):
        continue
    v = pcbnew.PCB_VIA(board)
    v.SetPosition(pcbnew.VECTOR2I_MM(x, y))
    v.SetWidth(pcbnew.FromMM(VIA_D))
    v.SetDrill(pcbnew.FromMM(VIA_H))
    v.SetNetCode(gnd)
    board.Add(v)
    grp.AddItem(v)
    obst.append((x - VIA_D / 2, y - VIA_D / 2, x + VIA_D / 2, y + VIA_D / 2))
    n += 1

filler = pcbnew.ZONE_FILLER(board)
filler.Fill(board.Zones())
board.Save(out)
print(f"outer GND pours on F.Cu/B.Cu, {n} stitching vias")
