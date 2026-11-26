import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import FreeCAD as App, Import
V=App.Vector
out=[]
d=App.openDocument(FRAME + "PlaneCarrier.FCStd")
objs=[o for o in d.Objects if o.TypeId=="Part::Feature"]
i=App.openDocument(FRAME + "InfraFrame.FCStd")
fr=App.openDocument(FRAME + "Frame.FCStd")
node=fr.getObject("FrameBody").Shape.copy()
n=App.newDocument("viz")
def add(name,shape):
    s=shape.copy(); s.rotate(V(108,0,105),V(0,1,0),180)
    o=n.addObject("Part::Feature",name); o.Shape=s; out.append(o)
for o in objs: add(o.Name,o.Shape)
for o in i.Objects:
    if o.TypeId=="Part::Feature": add(o.Name,o.Shape)
for k in range(8):
    f=node.copy(); f.translate(V(0,36+k*41.6,0)); add("Node%d"%k,f)
n.recompute()
Import.export(out,"block_tank.step")
Import.export([o for o in out if not o.Name.startswith("Node") and not o.Name.startswith("Carrier") and not o.Name.startswith("Unit") and not o.Name.startswith("Ctl") and o.Name not in ("Controller","SwitchSupply_DQD50")],"infra_tank.step")
print("CHK",[o.Name for o in out])
