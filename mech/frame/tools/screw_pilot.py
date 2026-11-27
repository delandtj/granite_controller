import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
# Board mount without heat-set inserts: the standoff holes become pilot holes for
# self-tapping screws (run once:  freecadcmd tools/screw_pilot.py, then tools/export.py).
# 2.5 mm x 10 deep, for 3 mm self-tapping screws (ST2.9 / 3 x 10..12 pan head), the
# same pilot as the plane carrier's screws (car_pilot_d). With a 3 x 10 screw through
# the 1.6 mm board that is 8.4 mm of thread in the standoff, 1.6 mm of hole spare.
import FreeCAD as App
d = App.openDocument(FRAME + "Frame.FCStd")
sh = d.getObject("Params")
sh.set("A22", "'insert_d")       # alias kept: PilotSketch uses Params.insert_d
sh.set("B22", "2.5")
sh.set("C22", "'Pilot hole for a 3 mm self-tapping screw (was 4.0 for M3 heat-set inserts)")
sh.set("B29", "10")
sh.set("C29", "'Pilot hole depth from the standoff top (3 x 10 screw: 8.4 in the plastic)")
d.recompute()
b = d.getObject("FrameBody")
print("CHK valid", b.Shape.isValid(), "solids", len(b.Shape.Solids), "errors",
      [o.Name for o in b.Group if "Invalid" in o.State or "Error" in o.State])
for e in b.Shape.Edges:
    c = getattr(e.Curve, "Radius", None)
    if c and abs(c - 1.25) < 1e-6 and abs(e.Curve.Axis.y) > 0.99:
        print("CHK pilot circle at x %.1f y %.1f z %.1f" % tuple(e.Curve.Center))
d.save()
