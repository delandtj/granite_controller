import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
# Test variant of the node frame for the Dynatron K129 (26.5 mm): cooler room 29 mm.
# Frame.FCStd itself is not saved.
import FreeCAD as App, Part, Mesh, MeshPart, Import
F = FRAME + ""
d = App.openDocument(F + "Frame.FCStd")
sh = d.getObject("Params"); sh.set("B35", "29"); d.recompute()
print("CHK bad", [o.Name for o in d.Objects if "Invalid" in o.State])
w = float(sh.get("frame_w")); hgt = float(sh.get("frame_h"))
print("CHK cooler_h", sh.get("cooler_h"), "standoff", sh.get("standoff_h"), "pitch", sh.get("pitch"))
body = d.getObject("FrameBody").Shape.copy()
print("CHK body valid", body.isValid(), len(body.Solids), body.BoundBox)
m = App.Matrix(); m.rotateX(1.5707963267948966)
pb = body.copy(); pb.translate(App.Vector(-w / 2, 0, -hgt / 2)); pb = pb.transformGeometry(m)
o = App.newDocument("Exp")
fs = o.addObject("Part::Feature", "Frame"); fs.Shape = body; o.recompute()
Import.export([fs], F + "Frame_k129.step")
mf = o.addObject("Mesh::Feature", "frame"); mf.Mesh = MeshPart.meshFromShape(Shape=pb, LinearDeflection=0.02, AngularDeflection=0.2, Relative=False)
Mesh.export([mf], F + "frame_k129.stl"); Mesh.export([mf], F + "frame_k129.3mf")
x = Mesh.Mesh(F + "frame_k129.stl"); print("CHK stl", x.BoundBox, "vol", round(x.Volume), "solid", x.isSolid(), "nonmanifold", x.hasNonManifolds())
