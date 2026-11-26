import os
FRAME = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"  # mech/frame/
import sys
sys.argv=['x']
import FreeCAD as App, Import, Part, runpy, os
V=App.Vector
src=open(FRAME + "plane_carrier.py").read().replace('\nmain()\n','\n')
g={'__file__': FRAME + 'plane_carrier.py'}
exec(compile(src,'pc','exec'),g)
h=g['half']()
# tank way up: frame z points down -> rotate 180 about y; boards then face +x
m=App.Matrix(-1,0,0,0, 0,1,0,0, 0,0,-1,210, 0,0,0,1); h=h.transformGeometry(m)
d=App.newDocument('h'); o=d.addObject('Part::Feature','CarrierHalf'); o.Shape=h; d.recompute()
Import.export([o],os.environ['OUT'])
print('CHK',h.BoundBox)
