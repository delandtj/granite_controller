import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
# Adds the zip tie lock for the top dovetails to Frame.FCStd (run once, from mech/frame):
#   TieSlots    a slot through each top leg under the dovetail groove, along x
#   TieRecess   a recess across the top of the groove walls and across the tenon top,
#               so a 3.6 mm tie over the joint lies flush with the top of the legs
# The tie goes over the tenon of the frame in front, down both sides of the leg and
# through the slot; its head sits on the inside of the leg. Only the top joints get one.
import FreeCAD as App
import Part
import Sketcher
V = App.Vector
F = FRAME + "Frame.FCStd"
TIE_W, TIE_D, SLOT_H = 4.4, 1.6, 2.2     # width of slot and recess, recess depth, slot height
TIE_Y, SLOT_Z = 0.9, 11.3                # from the frame's back; top of the slot (groove floor is at 10)
PITCH, FW, LEGW = 41.6, 216.0, 16.0

d = App.openDocument(F)
b = d.getObject("FrameBody")
org = {o.Role: o for o in b.Origin.OriginFeatures}


def rect(sk, x0, y0, x1, y1):
    n = sk.GeometryCount
    pts = [V(x0, y0, 0), V(x1, y0, 0), V(x1, y1, 0), V(x0, y1, 0)]
    for i in range(4):
        sk.addGeometry(Part.LineSegment(pts[i], pts[(i + 1) % 4]))
    for i in range(4):
        sk.addConstraint(Sketcher.Constraint("Coincident", n + i, 2, n + (i + 1) % 4, 1))
    sk.addConstraint(Sketcher.Constraint("Horizontal", n))
    sk.addConstraint(Sketcher.Constraint("Horizontal", n + 2))
    sk.addConstraint(Sketcher.Constraint("Vertical", n + 1))
    sk.addConstraint(Sketcher.Constraint("Vertical", n + 3))
    sk.addConstraint(Sketcher.Constraint("DistanceX", -1, 1, n, 1, x0))
    sk.addConstraint(Sketcher.Constraint("DistanceY", -1, 1, n, 1, y0))
    sk.addConstraint(Sketcher.Constraint("DistanceX", n, 1, n, 2, x1 - x0))
    sk.addConstraint(Sketcher.Constraint("DistanceY", n + 1, 1, n + 1, 2, y1 - y0))


def pocket(name, sk, length, expect):
    v0 = b.Shape.Volume
    for rev in (False, True):
        p = b.newObject("PartDesign::Pocket", name)
        p.Profile = sk
        p.Type = 0
        p.Length = length
        p.Reversed = rev
        d.recompute()
        dv = v0 - b.Shape.Volume
        if dv > 1:
            print("CHK", name, "reversed", rev, "removed", round(dv, 1), "expected about", expect)
            sk.Visibility = False
            return
        d.removeObject(p.Name)
        d.recompute()
    raise RuntimeError(name + " removed nothing")


if d.getObject("TieSlotSketch") is None:
    # slots: sketch on the YZ plane (sketch x = frame y, sketch y = frame z), cut along x.
    # Two pockets of one leg width each would need two planes; one cut through the whole
    # width only meets the two legs, the space between them is open at this height.
    sk = b.newObject("Sketcher::SketchObject", "TieSlotSketch")
    sk.AttachmentSupport = [(org["YZ_Plane"], "")]
    sk.MapMode = "FlatFace"
    d.recompute()
    rect(sk, TIE_Y, SLOT_Z, TIE_Y + TIE_W, SLOT_Z + SLOT_H)
    d.recompute()
    pocket("TieSlots", sk, FW, round(2 * LEGW * TIE_W * SLOT_H, 1))
    # recesses: sketch on the XY plane (z = 0, the top in the tank), cut 1.6 into the legs
    sk = b.newObject("Sketcher::SketchObject", "TieRecessSketch")
    sk.AttachmentSupport = [(org["XY_Plane"], "")]
    sk.MapMode = "FlatFace"
    d.recompute()
    for x0 in (0.0, FW - LEGW):
        rect(sk, x0, TIE_Y, x0 + LEGW, TIE_Y + TIE_W)                       # groove walls
        rect(sk, x0, PITCH + TIE_Y, x0 + LEGW, PITCH + TIE_Y + TIE_W)       # tenon top
    d.recompute()
    pocket("TieRecess", sk, TIE_D, "220")
s = b.Shape
print("CHK frame valid", s.isValid(), "solids", len(s.Solids), "vol", round(s.Volume), "bbox", s.BoundBox)
n = s.copy()
n.translate(V(0, PITCH, 0))
print("CHK clash with the next frame", round(s.common(n).Volume, 3))
for x0 in (0.0, FW - LEGW):
    c = s.common(Part.makeBox(LEGW, TIE_W - 0.2, SLOT_H - 0.2, V(x0, TIE_Y + 0.1, SLOT_Z + 0.1)))
    fl = s.common(Part.makeBox(LEGW, TIE_W, SLOT_Z - 10.0, V(x0, TIE_Y, 10.0)))
    print("CHK slot at x", x0, "material left in it", round(c.Volume, 2), "| groove floor above it", round(fl.Volume, 1), "of", round(LEGW * TIE_W * (SLOT_Z - 10), 1))
d.save()
