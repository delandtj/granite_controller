# End frame generator: run with  freecadcmd end_frame.py
#
# Builds EndFrame.FCStd from Frame.FCStd: a flat frame that closes a block. It
# drops onto the dovetails of the last node frame like any other frame and
# covers that frame's legs and the solder side of its board. It is the back
# slice of a node frame (rails, bands, dovetail grooves, bus bar holes) without
# legs, standoffs or gussets.
#
# Frame axes as in Frame.FCStd: x across (0 = bus bar side), y = stacking
# direction (0 = back, on the print bed), z = the frame's height axis (it
# points DOWN in the tank).
#
# THICK is less than the 12 mm rail of a node frame on purpose: a block is
# 36 (infra) + 8 x 41.6 = 368.8 mm, and the tank half is taken as 379 mm.
# The dovetail grooves are 6.2 mm deep, so 9 mm leaves a 2.8 mm wall in front.

import os
import FreeCAD as App
import Part
import Mesh
import MeshPart
import Import

HERE = os.path.dirname(os.path.abspath(__file__)) + "/"
V = App.Vector

THICK = 9.0
BLOCK_LEN = 36.0 + 8 * 41.6
TANK_HALF = 379.0


def main():
    src = App.openDocument(HERE + "Frame.FCStd")
    sh = src.getObject("Params")
    node = src.getObject("FrameBody").Shape.copy()
    pitch = float(sh.get("pitch"))
    W = float(sh.get("frame_w"))
    H = float(sh.get("frame_h"))
    for n in ("Standoffs", "PilotHoles"):
        src.getObject(n).Suppressed = True
    src.recompute()
    bad = [o.Name for o in src.Objects if "Invalid" in o.State]
    base = src.getObject("FrameBody").Shape.copy()
    # flood openings: the node frame's slicer modifier boxes (walls 0), moved from y 8..11 to
    # y 5.5..8.5 to stay inside this thinner frame (0.5 mm short of its front face)
    flood = []
    for o in src.getObject("FloodModifiers").Links:
        f = o.Shape.copy()
        f.translate(V(0, THICK - 0.5 - f.BoundBox.YMax, 0))
        flood.append(f)
    App.closeDocument(src.Name)                    # never saved: Frame.FCStd stays as it is

    shape = base.common(Part.makeBox(W + 20, THICK + 5, H + 20, V(-10, -5, -10))).removeSplitter()
    print("CHK recompute problems", bad)
    print("CHK end frame valid", shape.isValid(), "solids", len(shape.Solids), "bbox", shape.BoundBox, "vol", round(shape.Volume))
    placed = shape.copy()
    placed.translate(V(0, pitch, 0))
    print("CHK clash with the last node frame", round(placed.common(node).Volume, 3))
    print("CHK block with end frame", BLOCK_LEN + THICK, "mm, tank half", TANK_HALF, "slack", round(TANK_HALF - BLOCK_LEN - THICK, 1))

    if os.path.exists(HERE + "EndFrame.FCStd"):
        os.remove(HERE + "EndFrame.FCStd")
    doc = App.newDocument("EndFrame")
    f = doc.addObject("Part::Feature", "EndFrame")
    f.Shape = shape
    doc.recompute()
    doc.saveAs(HERE + "EndFrame.FCStd")
    Import.export([f], HERE + "EndFrame.step")

    # print orientation: back on the bed, centred (same as the node frame)
    m = App.Matrix()
    m.rotateX(1.5707963267948966)
    def orient(s):
        s = s.copy()
        s.translate(V(-W / 2, 0, -H / 2))
        return s.transformGeometry(m)

    def mesh(s, name):
        o = doc.addObject("Mesh::Feature", name)
        o.Mesh = MeshPart.meshFromShape(Shape=orient(s), LinearDeflection=0.02, AngularDeflection=0.2, Relative=False)
        return o

    for fl in flood:
        inner = fl.common(Part.makeBox(W, 100, H, V(0, -50, 0)))
        print("CHK flood modifier y %.1f..%.1f z %.1f..%.1f: solid inside %.2f" % (
            fl.BoundBox.YMin, fl.BoundBox.YMax, fl.BoundBox.ZMin, fl.BoundBox.ZMax, shape.common(inner).Volume / inner.Volume))
    mf = mesh(shape, "print_mesh")
    mm = mesh(Part.makeCompound(flood), "flood_modifiers")
    Mesh.export([mf], HERE + "end_frame.stl")
    Mesh.export([mf], HERE + "end_frame.3mf")
    Mesh.export([mf, mm], HERE + "end_frame_flood.3mf")   # set walls 0 on flood_modifiers in the slicer
    doc.removeObject("print_mesh")
    doc.removeObject("flood_modifiers")
    chk = Mesh.Mesh(HERE + "end_frame.stl")
    print("CHK stl bbox", chk.BoundBox, "vol", round(chk.Volume), "solid", chk.isSolid(), "nonmanifold", chk.hasNonManifolds())

    out = os.environ.get("END_VIZ")
    if out:
        o2 = doc.addObject("Part::Feature", "LastNode")
        o2.Shape = node
        f.Shape = placed
        Import.export([f, o2], out)


main()
