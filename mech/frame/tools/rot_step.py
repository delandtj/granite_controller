import FreeCAD as App, Import, os
V=App.Vector
src=os.environ["ROT_IN"]; dst=os.environ["ROT_OUT"]
d=App.newDocument("r")
Import.insert(src,"r")
objs=[o for o in d.Objects if hasattr(o,"Shape") and o.TypeId=="Part::Feature"]
for o in objs:
    s=o.Shape.copy(); s.rotate(V(108,0,105),V(0,1,0),180); o.Shape=s
d.recompute()
Import.export(objs,dst)
print("CHK",[(o.Label,o.Shape.BoundBox.YMin,o.Shape.BoundBox.YMax) for o in objs])
