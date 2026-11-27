# Infra frame generator: run with  freecadcmd infra_frame.py
#
# Builds InfraFrame.FCStd from Frame.FCStd: the same rails, legs, dovetails and
# bus bar holes as a node frame, but 36 mm deep and fitted for the block switch
# and the plane core (pp-48-core-fd, 68 x 72). The controller is not in this
# frame: it is on the plane carrier (plane_carrier.py).
#
# Frame axes (the same as Frame.FCStd): x across (0 = bus bar side), y = stacking
# direction (0 = back, the side that lies on the print bed), z = the frame's own
# height axis.
#
# IN THE TANK THE FRAMES STAND WITH FRAME z POINTING DOWN (checked on printed
# parts, 2026-11-19: a frame seated on the dovetails of its neighbours has
# frame z = 0 at the top). The first infra frame was laid out with frame z up
# and came out upside down.
#
# The numbers below are in LAYOUT axes, which are the tank's: lx across, lz up,
#   frame_x = W - lx      frame_z = H - lz      (y is the same)
# so in layout axes the bus bar side is at lx = W. turn() does the conversion;
# the CHK lines print frame axes.
#
# The core faces the BACK of the frame (-y), so that its two ribbon plugs for
# unit 1 point to the bus bar side, where the plane carrier is:
#   lx = CORE_X0 + board_x      lz = CORE_ZTOP - board_y
# Its parts side is towards y = 0, its ribbon headers towards the front.
#
# The added parts are plain solids fused onto the parametric frame body, so
# after changing a number below (or the Params sheet of Frame.FCStd) run this
# script again; InfraFrame.FCStd is overwritten.

import os
import FreeCAD as App
import Part
import Mesh
import MeshPart
import Import

HERE = os.path.dirname(os.path.abspath(__file__)) + "/"
V = App.Vector

DEPTH = 36.0          # frame depth (rail + legs), node frames are 41.6

# switch: Sodola SL902-SWTGW218AS, taken as 160 x 90 x 25, ports up
SW_W, SW_H, SW_T = 160.0, 90.0, 25.0
SW_Z = 99.5           # underside of the switch (lz)
SW_Y = 5.5            # back of the switch (just in front of the band)
SHELF_T = 4.0

# core: 68 x 72, holes 4 mm in from each corner
CORE_X0, CORE_ZTOP = 130.0, 93.5     # lx of board x = 0 (18 mm in from the bus bar side rail), lz of board y = 0
CORE_YF = 14.0                        # parts side of the board (it faces the back)
CORE_W, CORE_H = 68.0, 72.0

SO_D, PILOT_D, PILOT_DEPTH = 9.0, 2.5, 10.0   # pilot for 3 x 10 self-tapping screws (was 4.0 x 7 for M3 inserts)

# cooler of the first node frame, in FRAME axes: centre of the chip, half width of the pocket
# (cooler base 90 x 90 plus 1 mm around), and how deep it reaches into the infra frame's front
COOLER_X, COOLER_Z, COOLER_HALF, COOLER_POCKET = 116.21, 100.25, 46.0, 6.0
BAR_T = 5.0           # thickness of the back bars (same as the bands)


def box(x0, x1, y0, y1, z0, z1):
    return Part.makeBox(x1 - x0, y1 - y0, z1 - z0, V(x0, y0, z0))


def cyl(x, z, y0, y1, d):
    return Part.makeCylinder(d / 2.0, y1 - y0, V(x, y0, z), V(0, 1, 0))


def build():
    src = App.openDocument(HERE + "Frame.FCStd")
    sh = src.getObject("Params")
    sh.set("B18", str(DEPTH))                      # pitch
    for n in ("Standoffs", "PilotHoles"):
        src.getObject(n).Suppressed = True
    src.recompute()
    bad = [o.Name for o in src.Objects if "Invalid" in o.State]
    base = src.getObject("FrameBody").Shape.copy()
    W = float(sh.get("frame_w"))
    H = float(sh.get("frame_h"))
    band = float(sh.get("band"))
    App.closeDocument(src.Name)                    # never saved: Frame.FCStd stays as it is

    add = []
    # shelf under the switch, back rib and two front lips
    add.append(box(band, W - band, 0, DEPTH, SW_Z - SHELF_T, SW_Z))
    add.append(box(band, W - band, 0, BAR_T, SW_Z, SW_Z + 8))
    for x0 in (30.0, W - 60.0):
        add.append(box(x0, x0 + 30, SW_Y + SW_T + 0.5, SW_Y + SW_T + 3.5, SW_Z, SW_Z + 3))
    # two back bars that carry the core standoffs, from the lower band to the shelf
    shelf_bot = SW_Z - SHELF_T
    core_holes = [(CORE_X0 + bx, CORE_ZTOP - by) for bx in (4.0, 64.0) for by in (4.0, 68.0)]
    for bx in (4.0, 64.0):
        add.append(box(CORE_X0 + bx - 5, CORE_X0 + bx + 5, 0, BAR_T, 20.0, shelf_bot))
    cut = []
    for (x, z) in core_holes:
        add.append(cyl(x, z, 0, CORE_YF, SO_D))
        cut.append(cyl(x, z, CORE_YF - PILOT_DEPTH, CORE_YF + 1, PILOT_D))

    # layout axes -> frame axes
    def turn(sh):
        sh.rotate(V(W / 2.0, 0, H / 2.0), V(0, 1, 0), 180)
        return sh

    add = [turn(s) for s in add]
    cut = [turn(s) for s in cut]

    shape = base
    for s in add:
        shape = shape.fuse(s)
    for s in cut:
        shape = shape.cut(s)
    # pocket in the front for the first node's cooler: a tall cooler (Dynatron K129, 26.5 mm)
    # stands up to 5.2 mm proud of the node frame's rail back and reaches in here (frame axes)
    shape = shape.cut(box(COOLER_X - COOLER_HALF, COOLER_X + COOLER_HALF, DEPTH - COOLER_POCKET, DEPTH + 1,
                          COOLER_Z - COOLER_HALF, COOLER_Z + COOLER_HALF))
    # zip tie recess across the top of the tenons (as on the node frame, see tools/tie_slots.py):
    # the tie that locks this frame into the first node frame lies flush with the top of the legs
    for tx in (0.0, 200.0):
        shape = shape.cut(box(tx, tx + 16.0, DEPTH + 0.9, DEPTH + 5.3, -1.0, 1.6))
    shape = shape.removeSplitter()

    yb = CORE_YF + 1.6                 # ribbon header side of the core
    refs = {
        "Switch": box((W - SW_W) / 2, (W + SW_W) / 2, SW_Y, SW_Y + SW_T, SW_Z, SW_Z + SW_H),
        "Core": box(CORE_X0, CORE_X0 + CORE_W, CORE_YF, yb, CORE_ZTOP - CORE_H, CORE_ZTOP),
        # connector envelopes from the board files (courtyards), with the plug that goes on them
        "Core_J2_link_plug": box(CORE_X0 - 15, CORE_X0 + 8.4, yb, yb + 7.5, CORE_ZTOP - 36.8, CORE_ZTOP - 28.2),
        "Core_J3_link_plug": box(CORE_X0 + 57.7, CORE_X0 + 85, yb, yb + 7.5, CORE_ZTOP - 36.8, CORE_ZTOP - 28.2),
        "Core_J301_chain_plug": box(CORE_X0 + 57.7, CORE_X0 + 85, yb, yb + 7.5, CORE_ZTOP - 59.6, CORE_ZTOP - 40.8),
        "Core_XT30_plug": box(CORE_X0 + 31.3, CORE_X0 + 45.7, CORE_YF - 6.9, CORE_YF, CORE_ZTOP - 72.4 - 20, CORE_ZTOP - 56.5),
        "Core_C1_cap": box(CORE_X0 + 16.2, CORE_X0 + 28.8, CORE_YF - 10.9, CORE_YF, CORE_ZTOP - 54.8, CORE_ZTOP - 44.0),
    }
    refs = {n: turn(r) for n, r in refs.items()}
    return shape, refs, W, H, bad


def main():
    shape, refs, W, H, bad = build()
    print("CHK recompute problems", bad)
    print("CHK infra frame valid", shape.isValid(), "solids", len(shape.Solids), "bbox", shape.BoundBox, "vol", round(shape.Volume))
    for n, r in refs.items():
        c = shape.common(r).Volume
        others = sum(r.common(o).Volume for m, o in refs.items() if m != n and "Core" not in (n, m))
        print("CHK ref %-22s x %6.1f..%6.1f y %5.1f..%5.1f z %6.1f..%6.1f  clash with frame %.1f  with other parts %.1f" % (
            n, r.BoundBox.XMin, r.BoundBox.XMax, r.BoundBox.YMin, r.BoundBox.YMax, r.BoundBox.ZMin, r.BoundBox.ZMax, c, others))
    # the node frame that follows it in the block
    node = App.openDocument(HERE + "Frame.FCStd")
    nxt = node.getObject("FrameBody").Shape.copy()
    App.closeDocument(node.Name)
    nxt.translate(V(0, DEPTH, 0))
    print("CHK clash with the next node frame", round(shape.common(nxt).Volume, 3))

    if os.path.exists(HERE + "InfraFrame.FCStd"):
        os.remove(HERE + "InfraFrame.FCStd")
    doc = App.newDocument("InfraFrame")
    f = doc.addObject("Part::Feature", "InfraFrame")
    f.Shape = shape
    grp = doc.addObject("App::DocumentObjectGroup", "Reference")
    for n, r in refs.items():
        o = doc.addObject("Part::Feature", n)
        o.Shape = r
        grp.addObject(o)
    doc.recompute()
    doc.saveAs(HERE + "InfraFrame.FCStd")
    Import.export([f], HERE + "InfraFrame.step")

    # print orientation: back on the bed, legs up, centred (same as the node frame)
    m = App.Matrix()
    m.rotateX(1.5707963267948966)
    p = shape.copy()
    p.translate(V(-W / 2, 0, -H / 2))
    p = p.transformGeometry(m)
    mesh = MeshPart.meshFromShape(Shape=p, LinearDeflection=0.02, AngularDeflection=0.2, Relative=False)
    mf = doc.addObject("Mesh::Feature", "print_mesh")
    mf.Mesh = mesh
    Mesh.export([mf], HERE + "infra_frame.stl")
    Mesh.export([mf], HERE + "infra_frame.3mf")
    doc.removeObject("print_mesh")
    chk = Mesh.Mesh(HERE + "infra_frame.stl")
    print("CHK stl bbox", chk.BoundBox, "vol", round(chk.Volume), "solid", chk.isSolid(), "nonmanifold", chk.hasNonManifolds())

    # scratch export for pictures
    out = os.environ.get("INFRA_VIZ")
    if out:
        objs = [f] + [doc.getObject(n) for n in refs]
        Import.export(objs, out)


main()
