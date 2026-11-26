import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
# Adds the pilot holes for the plane carrier's self-tapping screws to Frame.FCStd:
# two holes in the bus bar side face (x = 0), one in each leg, 2.5 mm x 10 deep.
# Run once with  freecadcmd tools/carrier_pilot.py  from mech/frame.
import FreeCAD as App
import Part
import Sketcher
V = App.Vector
F = FRAME + "Frame.FCStd"
d = App.openDocument(F)
b = d.getObject("FrameBody")
sh = d.getObject("Params")
if d.getObject("CarrierPilotSketch") is None:
    v0 = b.Shape.Volume
    for row, (alias, val) in enumerate([("car_pilot_d", "2.5"), ("car_pilot_y", "32.7"), ("car_pilot_depth", "10")], start=50):
        sh.set("A%d" % row, alias)
        sh.set("B%d" % row, val)
        sh.setAlias("B%d" % row, alias)
    d.recompute()
    yz = [o for o in b.Origin.OriginFeatures if o.Role == "YZ_Plane"][0]
    sk = b.newObject("Sketcher::SketchObject", "CarrierPilotSketch")
    sk.AttachmentSupport = [(yz, "")]
    sk.MapMode = "FlatFace"
    d.recompute()
    for i, z in enumerate((5.0, 205.0)):
        sk.addGeometry(Part.Circle(V(32.7, z, 0), V(0, 0, 1), 1.25))
        sk.addConstraint(Sketcher.Constraint("Diameter", i, 2.5))
        sk.setExpression("Constraints[%d]" % (sk.ConstraintCount - 1), "Params.car_pilot_d")
        sk.addConstraint(Sketcher.Constraint("DistanceX", -1, 1, i, 3, 32.7))
        sk.setExpression("Constraints[%d]" % (sk.ConstraintCount - 1), "Params.car_pilot_y")
        sk.addConstraint(Sketcher.Constraint("DistanceY", -1, 1, i, 3, z))
    d.recompute()
    for rev in (False, True):
        p = b.newObject("PartDesign::Pocket", "CarrierPilotHoles")
        p.Profile = sk
        p.Type = 0
        p.Length = 10
        p.Reversed = rev
        p.setExpression("Length", "Params.car_pilot_depth")
        d.recompute()
        dv = v0 - b.Shape.Volume
        print("CHK reversed", rev, "state", p.State, "removed", round(dv, 1))
        if dv > 50:
            break
        d.removeObject(p.Name)
        d.recompute()
    sk.Visibility = False
s = b.Shape
print("CHK frame valid", s.isValid(), "solids", len(s.Solids), "vol", round(s.Volume), "bbox", s.BoundBox)
for z in (5, 205):
    c = Part.makeCylinder(1.0, 9.5, V(0, 32.7, z), V(1, 0, 0))
    c2 = Part.makeCylinder(1.0, 3, V(11, 32.7, z), V(1, 0, 0))
    print("CHK hole at z", z, "material left in the hole", round(s.common(c).Volume, 2), "solid behind it", round(s.common(c2).Volume / c2.Volume, 2))
d.save()
