import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import FreeCAD as App, Part, Mesh, MeshPart, Import
F=FRAME + ""

d=App.openDocument(F+"Frame.FCStd")
sh=d.getObject("Params"); w=float(sh.get("frame_w")); hgt=float(sh.get("frame_h"))
body=d.getObject("FrameBody").Shape.copy(); flood=d.getObject("FloodModifiers").Shape.copy()
print("CHK body valid",body.isValid(),len(body.Solids),body.BoundBox)
m=App.Matrix(); m.rotateX(1.5707963267948966)
def orient(s):
    s=s.copy(); s.translate(App.Vector(-w/2,0,-hgt/2)); return s.transformGeometry(m)
pb=orient(body); pf=orient(flood)
print("CHK print bb",pb.BoundBox,"vol",round(pb.Volume),"flood bb",pf.BoundBox)
o=App.newDocument("Exp")
fs=o.addObject("Part::Feature","Frame"); fs.Shape=body; o.recompute()
Import.export([fs],F+"Frame.step")
def mesh(s,name):
    mm=MeshPart.meshFromShape(Shape=s,LinearDeflection=0.02,AngularDeflection=0.2,Relative=False)
    f=o.addObject("Mesh::Feature",name); f.Mesh=mm; return f
mf=mesh(pb,"frame"); mm=mesh(pf,"flood_modifiers"); o.recompute()
Mesh.export([mf],F+"frame.stl"); Mesh.export([mm],F+"flood_modifiers.stl")
Mesh.export([mf],F+"frame.3mf"); Mesh.export([mf,mm],F+"frame_flood.3mf")
for f in ("frame.stl","flood_modifiers.stl","frame.3mf","frame_flood.3mf"):
    x=Mesh.Mesh(F+f); print("CHK",f,x.BoundBox,x.CountFacets,"vol",round(x.Volume),"solid",x.isSolid(),"nonmanifold",x.hasNonManifolds())
print("CHK new step bb",Part.read(F+"Frame.step").BoundBox)
