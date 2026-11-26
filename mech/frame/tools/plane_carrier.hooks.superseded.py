# Plane carrier generator: run with  freecadcmd plane_carrier.py
#
# The four plane units (pp-48-unit-fd, 78.5 x 72 each, 82.5 pitch) hang on the
# outside of a block, on the bus bar side (x = 0). The carrier is an open ladder
# printed in two identical halves; each half takes two units and hooks into two
# of the windows between the frame rails:
#   - carrying hooks: rigid, rest on the leg that is at the bottom in the tank
#     (frame z = 200) and reach down behind it
#   - catches: thin tabs under the leg at the top (frame z = 10) with a barb that
#     snaps behind it
# Nothing goes above or below the frames, so blocks can still be stacked.
#
# Block axes: x across (0 = bus bar side, the carrier is at x < 0), y = stacking
# direction (0 = back of the infra frame), z = the frames' own height axis.
# IN THE TANK FRAME z POINTS DOWN (checked on printed parts, 2026-11-19), so
# low z is the top of the block. The boards are the right way up in the tank:
# the units have their XT60 at the top edge and the node cables at the bottom
# edge, and unit 1's chain plug is at the infra frame end:
#   y = Y1 + n * 82.5 + board_x      z = Z0 + board_y      board front faces -x
#
# The strip between the units and the top of the block (z 10 .. 92) carries the
# switch supply and the controller (granite-controller-fd, 105 x 50), same
# mapping, network jack at the top edge. The 48 V leads of the units turn in to
# the bus bars between the units and the controller (z 64 .. 92).
#
# Heights come from the KiCad STEP exports: unit back side parts 5.2 mm (chain
# connectors), port module top 18.6 mm above the unit's front face.

import os
import FreeCAD as App
import Part
import Mesh
import MeshPart
import Import

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__))) + "/"
V = App.Vector

PITCH = 41.6          # node frame pitch
INFRA_D = 36.0        # infra frame depth
FRAME_H = 210.0
LEG_W = 16.0          # leg width in x
LEG_H = 10.0          # leg height in z

UNIT_W, UNIT_H, UNIT_PITCH = 78.5, 72.0, 82.5
UNIT_HOLES = [(4.0, 4.0), (4.0, 68.0), (75.3, 25.0), (75.0, 68.5)]
Y1 = INFRA_D + 3.0    # left edge of unit 1
Z0 = 96.0             # bottom edge of the units (their XT60 edge)

PLATE_T = 4.0         # ladder thickness
SPACER = 6.0          # loose M3 spacer between ladder and unit
HOLE_D = 4.0          # for M3 heat-set inserts, as on the frames
HALF_LEN = 2 * UNIT_PITCH          # 165
END_GAP = 0.2         # each half is this much shorter at both ends

RAIL_BOT = (Z0 - 4.0, Z0 + 11.0)   # under hole row board_y 4
RAIL_TOP = (Z0 + 61.0, Z0 + 76.0)  # under hole row board_y 68
STRUTS = [(20.0, 38.0), (145.0, HALF_LEN - 2.0 - END_GAP)]   # y range, relative to the half
RIB = (70.0, 81.0)                 # carries hole (75.3, 25) of the first unit
HOOK_Y = [(24.5, 36.5), (149.3, 161.3)]                      # inside the windows, clear of the gussets
HOOK_REACH = 19.5     # hooks end this far behind the outer face of the legs
TONGUE_L, TONGUE_H = 6.0, 3.3

# switch supply: CUI DQD50-48-S12-T (chassis mount, 76 x 31.5 x 21.3, screw terminals),
# standing on a pad below the first unit of a half, flat on the ladder, terminals outwards.
# Both halves get the pad (they stay identical); the converter goes on half A.
DC_L, DC_W, DC_T = 76.0, 31.5, 21.3
DC_HOLES = [(7.2, 27.25), (68.8, 4.25)]    # (along the length, across), 2 x 3.3 mm, diagonal
PAD_Y = (13.0, 61.0)                       # around the first strut, up to the controller's first hole row
PAD_Z = (LEG_H, RAIL_BOT[0])
DC_Y0, DC_Z0 = 13.25, 13.0                 # corner of the converter on the pad


# controller: 105 x 50, holes 4 mm in from each corner, on loose 10 mm spacers
# (its link plug is 9 mm deep on the back of the board)
CTL_W, CTL_H, CTL_SPACER = 105.0, 50.0, 10.0
CTL_Y0, CTL_Z0 = 51.0, 12.0                # board corner (x 0, y 0), relative to the half
CTL_HOLES = [(4.0, 4.0), (4.0, 46.0), (101.0, 4.0), (101.0, 46.0)]
CTL_RAIL = (LEG_H, LEG_H + 12.0)           # rail along the top edge, under hole row board_y 4
LEAD_WIN_Y, LEAD_WIN_Z = (147.0, 161.0), (66.0, 90.0)   # opening in the second strut for a 48 V lead


def box(x0, x1, y0, y1, z0, z1):
    return Part.makeBox(x1 - x0, y1 - y0, z1 - z0, V(x0, y0, z0))


def prism_xz(pts, y0, y1):
    w = Part.makePolygon([V(x, y0, z) for (x, z) in pts] + [V(pts[0][0], y0, pts[0][1])])
    return Part.Face(w).extrude(V(0, y1 - y0, 0))


def half():
    """One carrier half in its own coordinates: y = 0 is the left edge of its first unit."""
    ya, yb = -2.0 + END_GAP, HALF_LEN - 2.0 - END_GAP
    add = [box(-PLATE_T, 0, ya, yb, *RAIL_BOT), box(-PLATE_T, 0, ya, yb, *RAIL_TOP),
           box(-PLATE_T, 0, RIB[0], RIB[1], RAIL_BOT[1], RAIL_TOP[0])]
    for (y0, y1) in STRUTS:
        add.append(box(-PLATE_T, 0, y0, y1, LEG_H, FRAME_H - LEG_H))
    add.append(box(-PLATE_T, 0, PAD_Y[0], PAD_Y[1], PAD_Z[0], PAD_Z[1]))
    add.append(box(-PLATE_T, 0, PAD_Y[1], STRUTS[1][0], CTL_RAIL[0], CTL_RAIL[1]))
    # tongues at the far end, on the outer edge of each rail
    add.append(box(-PLATE_T, 0, yb, yb + TONGUE_L, RAIL_BOT[0], RAIL_BOT[0] + TONGUE_H))
    add.append(box(-PLATE_T, 0, yb, yb + TONGUE_L, RAIL_TOP[1] - TONGUE_H, RAIL_TOP[1]))
    for (y0, y1) in HOOK_Y:
        # carrying hook: sits on the leg at the bottom of the block (frame z 200 .. 210),
        # lip 5 mm down behind it
        zl = FRAME_H - LEG_H
        add.append(box(0, HOOK_REACH, y0, y1, zl - 3.5, zl))
        add.append(box(LEG_W + 0.4, HOOK_REACH, y0, y1, zl, zl + 5.0))
        # catch: 2.4 mm tab under the leg at the top (frame z 0 .. 10), barb 1.2 mm up behind it
        zt = LEG_H + 1.2
        add.append(box(0, HOOK_REACH, y0, y1, zt, zt + 2.4))
        add.append(prism_xz([(LEG_W + 0.5, zt), (LEG_W + 0.5, zt - 2.4), (LEG_W + 1.3, zt - 2.4), (HOOK_REACH, zt)], y0, y1))
    s = add[0]
    for a in add[1:]:
        s = s.fuse(a)
    # notches for the neighbour's tongues
    s = s.cut(box(-PLATE_T - 1, 1, ya - 1, ya + TONGUE_L + 0.4, RAIL_BOT[0] - 1, RAIL_BOT[0] + TONGUE_H + 0.2))
    s = s.cut(box(-PLATE_T - 1, 1, ya - 1, ya + TONGUE_L + 0.4, RAIL_TOP[1] - TONGUE_H - 0.2, RAIL_TOP[1] + 1))
    for n in range(2):
        for (bx, by) in UNIT_HOLES:
            s = s.cut(Part.makeCylinder(HOLE_D / 2, PLATE_T + 2, V(-PLATE_T - 1, n * UNIT_PITCH + bx, Z0 + by), V(1, 0, 0)))
    for (u, v) in DC_HOLES:
        s = s.cut(Part.makeCylinder(HOLE_D / 2, PLATE_T + 2, V(-PLATE_T - 1, DC_Y0 + v, DC_Z0 + u), V(1, 0, 0)))
    for (bx, by) in CTL_HOLES:
        s = s.cut(Part.makeCylinder(HOLE_D / 2, PLATE_T + 2, V(-PLATE_T - 1, CTL_Y0 + bx, CTL_Z0 + by), V(1, 0, 0)))
    s = s.cut(box(-PLATE_T - 1, 1, LEAD_WIN_Y[0], LEAD_WIN_Y[1], LEAD_WIN_Z[0], LEAD_WIN_Z[1]))
    return s.removeSplitter()


def unit_refs(n):
    """Envelopes of unit n (0..3) in block coordinates, from the KiCad STEP export."""
    y = Y1 + n * UNIT_PITCH
    xb = -PLATE_T - SPACER                     # back face of the board
    xf = xb - 1.6                              # front face
    r = {"Unit%d_board" % (n + 1): box(xf, xb, y, y + UNIT_W, Z0, Z0 + UNIT_H),
         "Unit%d_chain_plug" % (n + 1): box(xb, xb + 5.2, y - 6.5, y + 8.3, Z0 + 28.2, Z0 + 59.5),
         "Unit%d_chain_socket" % (n + 1): box(xb, xb + 5.2, y + 68.2, y + 83.0, Z0 + 28.2, Z0 + 59.6),
         "Unit%d_XT60" % (n + 1): box(xf - 8.5, xf, y + 59.2, y + 75.8, Z0 - 0.4, Z0 + 19.0),
         # mated plug and the start of the lead, before it turns in to the bars
         "Unit%d_XT60_plug" % (n + 1): box(xf - 8.5, xf, y + 59.2, y + 75.8, Z0 - 25.0, Z0 - 0.4),
         "Unit%d_terminals" % (n + 1): box(xf - 14.3, xf, y + 21.5, y + 57.5, Z0 + 61.1, Z0 + 72.3)}
    for k, bx in enumerate((6.5, 31.5)):       # port modules: 7 mm standoffs, board, parts to 10 mm
        r["Unit%d_module%s" % (n + 1, "AB"[k])] = box(xf - 18.6, xf - 7.0, y + bx, y + bx + 22.9, Z0 + 1.5, Z0 + 59.9)
    return r


def ctl_refs():
    """Envelopes of the controller on half A, in block coordinates."""
    y, z = Y1 + CTL_Y0, CTL_Z0
    xb = -PLATE_T - CTL_SPACER                 # back face of the board
    xf = xb - 1.6
    return {"Controller": box(xf, xb, y, y + CTL_W, z, z + CTL_H),
            "Ctl_RJ45": box(xf - 13.4, xf, y + 11.1, y + 30.1, z, z + 22.3),
            "Ctl_RJ45_cable": box(xf - 11.0, xf - 2.0, y + 14.7, y + 26.5, z - 35.0, z),
            "Ctl_J15_link_plug": box(xb, xb + 9.0, y + 95.1, y + 121.0, z + 24.4, z + 40.7),
            "Ctl_ESP32": box(xf - 3.2, xf, y + 73.0, y + 93.0, z + 30.0, z + 56.5)}


def block():
    d = App.openDocument(HERE + "InfraFrame.FCStd")
    parts = [d.getObject("InfraFrame").Shape.copy()]
    App.closeDocument(d.Name)
    d = App.openDocument(HERE + "Frame.FCStd")
    node = d.getObject("FrameBody").Shape.copy()
    App.closeDocument(d.Name)
    for k in range(8):
        f = node.copy()
        f.translate(V(0, INFRA_D + k * PITCH, 0))
        parts.append(f)
    return parts


def main():
    h = half()
    print("CHK half valid", h.isValid(), "solids", len(h.Solids), "bbox", h.BoundBox, "vol", round(h.Volume))
    halves = []
    for i in range(2):
        c = h.copy()
        c.translate(V(0, Y1 + i * HALF_LEN, 0))
        halves.append(c)
    print("CHK the two halves overlap", round(halves[0].common(halves[1]).Volume, 3))
    frames = block()
    for i, c in enumerate(halves):
        clash = [(k, round(c.common(f).Volume, 2)) for k, f in enumerate(frames) if c.common(f).Volume > 1e-6]
        print("CHK half", "AB"[i], "y", round(c.BoundBox.YMin, 1), round(c.BoundBox.YMax, 1), "clash with frames (0 = infra)", clash)
    refs = {}
    for n in range(4):
        refs.update(unit_refs(n))
    refs["SwitchSupply_DQD50"] = box(-PLATE_T - DC_T, -PLATE_T, Y1 + DC_Y0, Y1 + DC_Y0 + DC_W, DC_Z0, DC_Z0 + DC_L)
    crefs = ctl_refs()
    refs.update(crefs)
    for cn, cr in crefs.items():
        hit = [(n, round(r.common(cr).Volume, 1)) for n, r in refs.items() if n not in crefs and r.common(cr).Volume > 1e-6]
        fr = [(k, round(cr.common(f).Volume, 1)) for k, f in enumerate(frames) if cr.common(f).Volume > 1e-6]
        print("CHK %-18s x %6.1f..%6.1f y %6.1f..%6.1f z %6.1f..%6.1f  clash with other parts %s  with frames %s" % (
            cn, cr.BoundBox.XMin, cr.BoundBox.XMax, cr.BoundBox.YMin, cr.BoundBox.YMax, cr.BoundBox.ZMin, cr.BoundBox.ZMax, hit, fr))
    dc = refs["SwitchSupply_DQD50"]
    print("CHK switch supply y", dc.BoundBox.YMin, dc.BoundBox.YMax, "z", dc.BoundBox.ZMin, dc.BoundBox.ZMax,
          "clash with unit parts", [(n, round(r.common(dc).Volume, 1)) for n, r in refs.items() if r is not dc and r.common(dc).Volume > 1e-6])
    bad = [(n, round(r.common(c).Volume, 1)) for n, r in refs.items() for c in halves if r.common(c).Volume > 1e-6]
    print("CHK parts clashing with the carrier", bad)
    xmin = min(r.BoundBox.XMin for r in refs.values())
    print("CHK plane sticks out", round(-xmin, 1), "mm from the frame face; units span y", Y1, Y1 + 3 * UNIT_PITCH + UNIT_W)

    if os.path.exists(HERE + "PlaneCarrier.FCStd"):
        os.remove(HERE + "PlaneCarrier.FCStd")
    doc = App.newDocument("PlaneCarrier")
    objs = []
    for i, c in enumerate(halves):
        o = doc.addObject("Part::Feature", "CarrierHalf" + "AB"[i])
        o.Shape = c
        objs.append(o)
    grp = doc.addObject("App::DocumentObjectGroup", "Reference")
    robjs = []
    for n, r in refs.items():
        o = doc.addObject("Part::Feature", n)
        o.Shape = r
        grp.addObject(o)
        robjs.append(o)
    doc.recompute()
    doc.saveAs(HERE + "PlaneCarrier.FCStd")
    Import.export(objs, HERE + "PlaneCarrier.step")

    # print orientation: outer face on the bed, hooks up; one half (print it twice)
    p = h.copy()
    m = App.Matrix()
    m.rotateY(-1.5707963267948966)             # x -> z
    p = p.transformGeometry(m)
    bb = p.BoundBox
    p.translate(V(-(bb.XMin + bb.XMax) / 2, -(bb.YMin + bb.YMax) / 2, -bb.ZMin))
    mesh = MeshPart.meshFromShape(Shape=p, LinearDeflection=0.02, AngularDeflection=0.2, Relative=False)
    mf = doc.addObject("Mesh::Feature", "print_mesh")
    mf.Mesh = mesh
    Mesh.export([mf], HERE + "plane_carrier_half.stl")
    Mesh.export([mf], HERE + "plane_carrier_half.3mf")
    doc.removeObject("print_mesh")
    chk = Mesh.Mesh(HERE + "plane_carrier_half.stl")
    print("CHK stl bbox", chk.BoundBox, "vol", round(chk.Volume), "solid", chk.isSolid(), "nonmanifold", chk.hasNonManifolds())

    out = os.environ.get("PLANE_VIZ")
    if out:
        fo = []
        for k, f in enumerate(frames):
            o = doc.addObject("Part::Feature", "Frame%d" % k)
            o.Shape = f
            fo.append(o)
        Import.export(objs + robjs + fo, out)


main()
