import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
# Drop-in board mount for Frame.FCStd (run once, from anywhere):  freecadcmd tools/drop_in.py
#
# No screws and no heat-set inserts. The board hangs in four claws at its corners:
#   top claws (tank top = frame z 15 edge)     deep pocket: the board slides 4.2 mm up into them
#   bottom claws (tank bottom = z 195 edge)    shallow pocket: the board rests on their floor
# Fitting: tilt the board, push its top edge up into the top claws, swing the bottom edge in
# over the bottom lips, let it drop onto the bottom floors. Then thread a 3.6 mm zip tie
# along x through each top claw, in the gap above the board edge, and close it around the
# claw's front lip: the tie fills the gap, so the board cannot lift out of the bottom claws.
# The four standoffs stay as the back stop (no insert holes any more: PilotHoles suppressed).
#
# Claws only sit where the edges are free: the connectors and their plugs fill the rest of
# both edges (checked against MoboLink and PlugsLink). The parts #368..#371 of MoboLink are
# the vendor's screw stand-ins at the corner holes: nothing on the real board, so ignored.
#
# Frame axes: x across, y = stacking (0 = back, the print bed), z = frame height,
# z POINTS DOWN in the tank. Board nominal x 18..198, y 23..24.6, z 15..195; seated on the
# bottom floors it is at z 15.2..195.2.
import FreeCAD as App
import Part
V = App.Vector
F = FRAME + "Frame.FCStd"

PK_B, PK_F, LIP_F = 22.6, 25.0, 27.2   # pocket back / front face, lip front face (board 23..24.6)
SEAT = 0.2                             # bottom floor below the board's nominal edge
TOP_FLOOR = 11.0                       # top pocket floor: travel 15.2 - 11 = 4.2
TOP_LIP = 17.0                         # top lip covers the board face to here (2 mm, seated 1.8)
BOT_LIP = 192.5                        # bottom lip covers the board face from here (2.7 mm)
TIE_W = 3.6
X_CLR = 0.3                            # board edge to pocket end, in x
# claws: name, x0, x1, z0, z1 (block), pocket x0, x1, pocket z0, z1, tie channel x0, x1 (or None)
CLAWS = [
    ("TL", 16.0, 31.0, 7.0, 17.0, 18 - X_CLR, 31.0, TOP_FLOOR, 17.0, (12.0, 18 - X_CLR)),
    ("TR", 179.5, 200.0, 7.0, 17.0, 179.5, 198 + X_CLR, TOP_FLOOR, 17.0, (198 + X_CLR, 204.0)),
    ("BL", 15.0, 27.5, BOT_LIP - 1, 200.0, 18 - X_CLR, 27.5, BOT_LIP, 195 + SEAT, None),
    ("BR", 188.5, 201.0, BOT_LIP - 1, 200.0, 188.5, 198 + X_CLR, BOT_LIP, 195 + SEAT, None),
]
SCREW_CUBES = (368, 369, 370, 371)

d = App.openDocument(F)
b = d.getObject("FrameBody")
sh = d.getObject("Params")
pitch = float(sh.get("pitch"))


def box(kind, name, x0, x1, y0, y1, z0, z1):
    o = b.newObject("PartDesign::" + kind, name)
    o.Length, o.Width, o.Height = x1 - x0, y1 - y0, z1 - z0
    o.Placement = App.Placement(V(x0, y0, z0), App.Rotation())
    return o


if d.getObject("ClawTL") is None:
    d.getObject("PilotHoles").Suppressed = True
    d.recompute()
    v0 = b.Shape.Volume
    for nm, x0, x1, z0, z1, px0, px1, pz0, pz1, tie in CLAWS:
        box("AdditiveBox", "Claw" + nm, x0, x1, 0.0, LIP_F, z0, z1)
        # only the front lip touches the board; the back stop is the standoffs at y 23
        box("SubtractiveBox", "ClawPocket" + nm, px0, px1, PK_B, PK_F, pz0, pz1)
        if nm[0] == "B":
            # the block starts 1 mm above the lip so it overlaps the bottom standoff (whose
            # edge would otherwise just touch it along a line); trim the lip back to BOT_LIP
            box("SubtractiveBox", "ClawLipTrim" + nm, x0, x1, PK_B, LIP_F, z0, BOT_LIP)
        if tie:
            box("SubtractiveBox", "ClawTie" + nm, tie[0], tie[1], PK_B, PK_F, TOP_FLOOR, 15 + SEAT - 0.4)
        d.recompute()
    for r, (a, v, c) in enumerate([
            ("claw_pocket_y", "22.6", "Drop-in claws (tools/drop_in.py): pocket y 22.6..25.0, lip front 27.2"),
            ("claw_top_floor", "11", "Top claw pocket floor z: board travels 4.2 up to clear the bottom lips"),
            ("claw_bot_lip", "192.5", "Bottom lip starts at z 192.5 (2.7 over the board face); floor at z 195.2"),
            ("claw_tie_w", "3.6", "Zip tie in the top claws, in the gap above the board edge (0.6 play)")], start=53):
        sh.set("A%d" % r, "'" + a)
        sh.set("B%d" % r, v)
        sh.set("C%d" % r, "'" + c + " (reference only, the claws use plain numbers)")
    d.recompute()
    print("CHK claws added, volume change", round(b.Shape.Volume - v0, 1))

if d.getObject("ClawTopStandoffCut") is None:
    # The two top standoffs go: parts beside them run into them while the board is pushed
    # 4.2 mm up. The top claws' pocket back face (y 22.6) is the back stop at the top.
    v0 = b.Shape.Volume
    o = box("SubtractiveBox", "ClawTopStandoffCut", 18.0, 28.0, 5.0, 23.0, 17.0, 27.0)
    o2 = box("SubtractiveBox", "ClawTopStandoffCutR", 188.0, 198.0, 5.0, 23.0, 17.0, 27.0)
    d.recompute()
    print("CHK top standoffs removed, volume", round(v0 - b.Shape.Volume, 1))

s = b.Shape
print("CHK frame valid", s.isValid(), "solids", len(s.Solids), "vol", round(s.Volume), "bbox", s.BoundBox)
bad = [o.Name for o in b.Group if "Invalid" in o.State or "Error" in o.State]
print("CHK features in error", bad)
n = s.copy()
n.translate(V(0, pitch, 0))
print("CHK clash with the next frame", round(s.common(n).Volume, 3))

ms = d.getObject("MoboLink").Shape
ps = d.getObject("PlugsLink").Shape
pcb = ms.Solids[3645]


def clashes(shapes, dz, label, skip=()):
    out = []
    for i, so in enumerate(shapes):
        if i in skip:
            continue
        t = so.copy()
        t.translate(V(0, 0, dz))
        if not t.BoundBox.intersect(s.BoundBox):
            continue
        v = s.common(t).Volume
        if v > 0.01:
            out.append((i, round(v, 2), "x %.1f..%.1f z %.1f..%.1f" % (t.BoundBox.XMin, t.BoundBox.XMax, t.BoundBox.ZMin, t.BoundBox.ZMax)))
    print("CHK", label, out if out else "none")


clashes(ms.Solids, SEAT, "board parts vs frame, seated", SCREW_CUBES)
clashes(ps.Solids, SEAT, "plugs vs frame, seated")
lift = 15 + SEAT - TOP_FLOOR
clashes(ms.Solids, SEAT - lift + 0.05, "board parts vs frame, pushed up into the top claws", SCREW_CUBES)
# while pushed up, the bottom edge must be clear of the bottom lips
print("CHK pushed up: board bottom edge at z", round(195 + SEAT - lift + 0.05, 2), "bottom lips start at", BOT_LIP)
# lips really cover the board face (seated), and pockets are empty around the board
for nm, x0, x1, z0, z1, px0, px1, pz0, pz1, tie in CLAWS:
    xa, xb = max(px0, 18.0), min(px1, 198.0)
    za, zb = (15 + SEAT, pz1) if nm[0] == "T" else (pz0, 195 + SEAT)
    lip = s.common(Part.makeBox(xb - xa, 0.5, zb - za, V(xa, PK_F + 0.01, za))).Volume
    print("CHK claw", nm, "lip in front of the board face: %.1f of %.1f mm3" % (lip, (xb - xa) * 0.5 * (zb - za)),
          "| cover %.1f mm" % (zb - za))
    if tie:
        g = s.common(Part.makeBox(tie[1] - tie[0], PK_F - PK_B - 0.1, TIE_W,
                                  V(tie[0], PK_B + 0.05, TOP_FLOOR + 0.05))).Volume
        g2 = s.common(Part.makeBox(px1 - px0, PK_F - PK_B - 0.1, 15 + SEAT - TOP_FLOOR - 0.1,
                                   V(px0, PK_B + 0.05, TOP_FLOOR + 0.05))).Volume
        print("CHK claw", nm, "tie channel material %.2f, gap above the board edge material %.2f (both must be 0)" % (g, g2))
d.save()
print("CHK saved")
