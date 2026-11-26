import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
# Two node frames with a zip tie on one top joint, turned the tank way up, for a picture.
import os
import FreeCAD as App, Import, Part
V = App.Vector
d = App.openDocument(FRAME + "Frame.FCStd")
s = d.getObject("FrameBody").Shape
def box(x0, x1, y0, y1, z0, z1): return Part.makeBox(x1 - x0, y1 - y0, z1 - z0, V(x0, y0, z0))
P = 41.6
a = s.copy()
bq = s.copy(); bq.translate(V(0, P, 0))
t, y0, y1 = 1.3, P + 1.3, P + 4.9
tie = box(-t, 16 + t, y0, y1, 0.3, 0.3 + t)                  # over the tenon, in the recess
for q in (box(-t, 0, y0, y1, 0.3, 12.9), box(16, 16 + t, y0, y1, 0.3, 12.9), box(-t, 16 + t, y0, y1, 11.6, 12.9)):
    tie = tie.fuse(q)
tie = tie.fuse(box(16 + t, 16 + t + 4.5, P + 0.6, P + 5.6, 3.0, 9.0))   # head, on the inside of the leg
m = App.Matrix(-1, 0, 0, 216, 0, 1, 0, 0, 0, 0, -1, 210, 0, 0, 0, 1)
n = App.newDocument("v")
out = []
for name, sh in (("FrameA", a), ("FrameB", bq), ("Tie", tie.removeSplitter())):
    o = n.addObject("Part::Feature", name); o.Shape = sh.transformGeometry(m); out.append(o)
n.recompute()
Import.export(out, os.environ["OUT"])
print("CHK tie clash with frames", round(tie.common(a).Volume, 3), round(tie.common(bq).Volume, 3))
