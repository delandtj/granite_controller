#!/usr/bin/env python3
"""Footprints for the screwed PM module mount, generated from docs/pm-interface.md.

Change a number in the spec first, then here, then rerun; never edit a
generated footprint by hand.

Four M2 screws through plated holes in the module into SMT threaded
standoffs (Sinhoo SMTSO2060CTJ, M2 x 6, 5.56 mm body, 3.6 mm pilot 1.53 mm
long, LCSC C2915628) reflowed on the host (tile column, bench carrier).
Current flows through the clamped pads: 1 VIN, 2 GND input end, 3 GND
output end, 4 VOUT. Screws 2 and 3 sit 2.0 mm closer to the centre line
than 1 and 4: turned 180 degrees no screw lines up, so a reversed module
cannot be fastened. The signals reach the module through six pogo pins on
the host (Xinyangze YZ130115073P-01, 2 mm base, 7.3 mm free, 5.5 mm
compressed, LCSC C5157258; 1.3 mm travel at the 6 mm gap) pressing on flat
pads on the module underside.

Coordinates: the module's top view, origin = module centre, body 58.4 x
22.9 mm, +x toward the output end, +y down. The host uses the same
coordinates, not mirrored (the module sits F side up on it). On the module
the screws are one footprint (PM_Mount_M2, origin = module centre) and the
aux pads another (PM_Aux_Pads_2x03, origin = AUX_OFS from it). On the host
every standoff and pogo pin is its own one-pad part (JLC places one part per
designator): host_parts() gives their positions for a module centre and
rotation, and the board-only PM_Module_Outline draws the module on the silk.

Aux pads (numbered by function, positions in the module frame):
  1 OV_KILL (+10.16, -9.12)  5 GND (+12.70, -9.12)  2 PGOOD (+15.24, -9.12)
  3 ID      (+10.16, -6.58)  6 GND (+12.70, -6.58)  4 RUN   (+15.24, -6.58)

Usage: pmmount.py   (writes into ../pm-19v-250w/pm.pretty, ../pm-bench-carrier/carrier.pretty)
       pmmount.py place <board> <lib> <cx> <cy> <rot> <ref base> <aux pads 1,2,..> [old,refs]
"""
import math
import os

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
LIBS = {
    'pm': os.path.join(HERE, '..', 'pm-19v-250w', 'pm.pretty'),
    'carrier': os.path.join(HERE, '..', 'pm-bench-carrier', 'carrier.pretty'),
}

BODY_W, BODY_H = 58.4, 22.9
SCREWS = {'1': (-25.70, -7.95), '2': (-25.70, 5.95), '3': (25.70, 5.95), '4': (25.70, -7.95)}  # VIN GND GND VOUT
AUX_OFS = (12.70, -7.85)          # aux block centre from the mount origin
AUX_P = 2.54
AUX_GRID = {1: (-1, -1), 5: (0, -1), 2: (1, -1), 3: (-1, 1), 6: (0, 1), 4: (1, 1)}   # (column, row) per pad
AUX_NAMES = ['OV_KILL', 'PGOOD', 'ID', 'RUN', 'GND', 'GND']

# module side: M2 clearance hole, head pad on top, standoff face pad underneath
MOD_DRILL, MOD_F, MOD_IN, MOD_B = 2.4, 5.0, 4.0, 6.2
MOD_VIAS, MOD_VIA_R, MOD_VIA_DRILL, MOD_VIA_D = 8, 2.2, 0.3, 0.6   # stitching ring inside the screw pads
# plane side: standoff pilot hole, solder pad for the 5.56 mm body
SO_DRILL, SO_F, SO_IN, SO_B, SO_BODY = 3.75, 6.4, 5.0, 5.0, 5.56
MOD_AUX_PAD, POGO_PAD = 2.2, 2.2
DASH_TAG = 0.0011

I = pcbnew.FromMM
V = lambda x, y: pcbnew.VECTOR2I(I(round(x, 4)), I(round(y, 4)))


def aux_pos(n):
    c, r = AUX_GRID[int(n)]
    return (c * AUX_P, r * AUX_P / 2)


def new_fp(name, descr, tags):
    fp = pcbnew.FOOTPRINT(None)
    fp.SetFPIDAsString(name)
    fp.SetLibDescription(descr)
    fp.SetKeywords(tags)
    fp.Reference().SetText('REF**')
    fp.Value().SetText(name)
    return fp


def text(fp, s, x, y, layer, size=0.8):
    t = pcbnew.PCB_TEXT(fp)
    t.SetText(s)
    t.SetPosition(V(x, y))
    t.SetLayer(layer)
    t.SetTextSize(V(size, size))
    t.SetTextThickness(I(0.12 if size < 1 else 0.15))
    fp.Add(t)


def rect(fp, x0, y0, x1, y1, layer, w=0.1, dash=False):
    r = pcbnew.PCB_SHAPE(fp, pcbnew.SHAPE_T_RECTANGLE)
    r.SetStart(V(x0, y0))
    r.SetEnd(V(x1, y1))
    r.SetLayer(layer)
    r.SetWidth(I(round(w + DASH_TAG, 4) if dash else w))      # dash style is not in the python API: tagged, fixed in save()
    fp.Add(r)


def circle(fp, x, y, r, layer, w=0.05):
    c = pcbnew.PCB_SHAPE(fp, pcbnew.SHAPE_T_CIRCLE)
    c.SetCenter(V(x, y))
    c.SetEnd(V(x + r, y))
    c.SetLayer(layer)
    c.SetWidth(I(w))
    fp.Add(c)


def th_pad(fp, num, x, y, drill, f, inner, b):
    p = pcbnew.PAD(fp)
    p.SetNumber(num)
    p.SetAttribute(pcbnew.PAD_ATTRIB_PTH)
    p.SetLayerSet(pcbnew.PAD.PTHMask())
    p.Padstack().SetMode(pcbnew.PADSTACK.MODE_FRONT_INNER_BACK)
    for lay, d in ((pcbnew.F_Cu, f), (pcbnew.PADSTACK.INNER_LAYERS, inner), (pcbnew.B_Cu, b)):
        p.SetShape(lay, pcbnew.PAD_SHAPE_CIRCLE)
        p.SetSize(lay, V(d, d))
    p.SetDrillShape(pcbnew.PAD_DRILL_SHAPE_CIRCLE)
    p.SetDrillSize(V(drill, drill))
    p.SetPosition(V(x, y))
    fp.Add(p)
    return p


def smd_pad(fp, num, x, y, d, back=False, paste=True):
    p = pcbnew.PAD(fp)
    p.SetNumber(num)
    p.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
    ls = pcbnew.LSET()
    for l in ((pcbnew.B_Cu, pcbnew.B_Mask) + ((pcbnew.B_Paste,) if paste else ()) if back else
              (pcbnew.F_Cu, pcbnew.F_Mask) + ((pcbnew.F_Paste,) if paste else ())):
        ls.AddLayer(l)
    p.SetLayerSet(ls)
    p.SetShape(pcbnew.F_Cu, pcbnew.PAD_SHAPE_CIRCLE)
    p.SetSize(pcbnew.F_Cu, V(d, d))
    p.SetPosition(V(x, y))
    fp.Add(p)
    return p


def paste_ring(fp, x, y, r_in, r_out):
    """four paste apertures around a standoff hole (aperture pads, no number)"""
    rm, w = (r_in + r_out) / 2, r_out - r_in
    for dx, dy in ((rm, 0), (-rm, 0), (0, rm), (0, -rm)):
        p = pcbnew.PAD(fp)
        p.SetNumber('')
        p.SetAttribute(pcbnew.PAD_ATTRIB_SMD)
        ls = pcbnew.LSET()
        ls.AddLayer(pcbnew.F_Paste)
        p.SetLayerSet(ls)
        p.SetShape(pcbnew.F_Cu, pcbnew.PAD_SHAPE_RECTANGLE)
        sz = (w, 1.6) if dx else (1.6, w)
        p.SetSize(pcbnew.F_Cu, V(*sz))
        p.SetPosition(V(x + dx, y + dy))
        fp.Add(p)


def outline(fp, silk):
    rect(fp, -BODY_W / 2, -BODY_H / 2, BODY_W / 2, BODY_H / 2, pcbnew.F_Fab)
    if silk:
        rect(fp, -BODY_W / 2, -BODY_H / 2, BODY_W / 2, BODY_H / 2, pcbnew.F_SilkS, 0.15, dash=True)
    ax, ay = AUX_OFS
    rect(fp, ax - 1.5 * AUX_P, ay - AUX_P, ax + 1.5 * AUX_P, ay + AUX_P, pcbnew.F_Fab, 0.1, dash=True)
    text(fp, 'AUX', ax, ay + AUX_P + 0.8, pcbnew.F_Fab, 0.6)
    fp.Reference().SetPosition(V(0, -BODY_H / 2 - 1.75))
    fp.Reference().SetLayer(pcbnew.F_SilkS)
    fp.Value().SetPosition(V(0, BODY_H / 2 + 1.75))
    fp.Value().SetLayer(pcbnew.F_Fab)
    for n, (x, y) in SCREWS.items():
        text(fp, n, x + (4.0 if x < 0 else -4.0), y, pcbnew.F_SilkS if silk else pcbnew.F_Fab)
    x, y = SCREWS['1']
    text(fp, 'VIN', x + 4.0, y + 2.0, pcbnew.F_SilkS if silk else pcbnew.F_Fab)


def module_mount():
    fp = new_fp('PM_Mount_M2',
                'PM module mount, module side (docs/pm-interface.md): four M2 screws (1 VIN, 2 GND input end, '
                '3 GND output end, 4 VOUT; 2 and 3 offset 2 mm inward, polarised) through 2.4 mm plated holes into '
                'SMT standoffs on the host. Head pad 5.0 mm on top, 6.2 mm standoff face pad underneath (5.56 mm '
                'body), ring of 8 vias. Aux pads: PM_Aux_Pads_2x03 at (+12.70, -7.85).',
                'PM module mount M2 screw standoff')
    fp.SetAttributes(pcbnew.FP_THROUGH_HOLE)
    outline(fp, silk=False)
    for n, (x, y) in SCREWS.items():
        th_pad(fp, n, x, y, MOD_DRILL, MOD_F, MOD_IN, MOD_B)
        for k in range(MOD_VIAS):                                   # same number = same net, solid to the pours
            a = 2 * math.pi * (k + 0.5) / MOD_VIAS
            th_pad(fp, n, x + MOD_VIA_R * math.cos(a), y + MOD_VIA_R * math.sin(a),
                   MOD_VIA_DRILL, MOD_VIA_D, MOD_VIA_D, MOD_VIA_D)
        circle(fp, x, y, MOD_F / 2 + 0.3, pcbnew.F_CrtYd)           # screw head
        circle(fp, x, y, MOD_B / 2 + 0.3, pcbnew.B_CrtYd)           # standoff body under the module
        circle(fp, x, y, SO_BODY / 2, pcbnew.B_Fab, 0.1)
    return fp


def host_outline():
    fp = new_fp('PM_Module_Outline',
                'PM module outline on the host (board-only, no pads): body, screw numbers, aux block; '
                'place at the module centre, rotated with the standoffs and pogo pins (pmmount.host_parts)',
                'PM module outline')
    fp.SetAttributes(pcbnew.FP_BOARD_ONLY | pcbnew.FP_EXCLUDE_FROM_BOM | pcbnew.FP_EXCLUDE_FROM_POS_FILES)
    outline(fp, silk=True)
    return fp


def host_standoff():
    fp = new_fp('PM_Standoff_M2x6_SMTSO',
                'SMT M2 x 6 threaded standoff (Sinhoo SMTSO2060CTJ, LCSC C2915628: 5.56 mm body, 3.6 mm pilot '
                '1.53 mm long) in a 3.75 mm plated hole, 6.4 mm solder pad with four paste apertures; carries '
                'the PM module current through the clamped pad',
                'standoff SMTSO M2 PM module mount')
    fp.SetAttributes(pcbnew.FP_SMD)                              # placed by the assembler
    th_pad(fp, '1', 0, 0, SO_DRILL, SO_F, SO_IN, SO_B)
    paste_ring(fp, 0, 0, SO_DRILL / 2 + 0.35, SO_F / 2 - 0.2)
    circle(fp, 0, 0, SO_F / 2 + 0.3, pcbnew.F_CrtYd)
    circle(fp, 0, 0, SO_BODY / 2, pcbnew.F_Fab, 0.1)
    fp.Reference().SetPosition(V(0, -SO_F / 2 - 1.0))
    fp.Reference().SetLayer(pcbnew.F_Fab)
    fp.Value().SetPosition(V(0, SO_F / 2 + 1.0))
    fp.Value().SetLayer(pcbnew.F_Fab)
    return fp


def module_aux():
    fp = new_fp('PM_Aux_Pads_2x03',
                'PM module aux contacts (docs/pm-interface.md): 2x3 flat %.1f mm pads on B.Cu at 2.54 mm, no '
                'paste, ENIG; pogo pins on the host press on them (1 OV_KILL 2 PGOOD 3 ID 4 RUN 5 GND 6 GND, '
                'GND in the middle column)' % MOD_AUX_PAD,
                'PM module aux pogo target pads')
    fp.SetAttributes(pcbnew.FP_SMD | pcbnew.FP_EXCLUDE_FROM_POS_FILES)
    for n in AUX_GRID:
        x, y = aux_pos(n)
        smd_pad(fp, str(n), x, y, MOD_AUX_PAD, back=True, paste=False)
    rect(fp, -1.5 * AUX_P - 0.5, -AUX_P - 0.5, 1.5 * AUX_P + 0.5, AUX_P + 0.5, pcbnew.B_CrtYd, 0.05)
    rect(fp, -1.5 * AUX_P, -AUX_P, 1.5 * AUX_P, AUX_P, pcbnew.B_Fab, 0.1)
    text(fp, '1', aux_pos(1)[0] - 1.9, aux_pos(1)[1], pcbnew.B_Fab, 0.6)
    fp.Reference().SetPosition(V(0, -AUX_P - 1.2))
    fp.Reference().SetLayer(pcbnew.F_Fab)
    fp.Value().SetPosition(V(0, AUX_P + 1.2))
    fp.Value().SetLayer(pcbnew.F_Fab)
    return fp


def host_pogo():
    fp = new_fp('PM_Pogo_YZ130115073P',
                'SMD pogo pin (Xinyangze YZ130115073P-01, LCSC C5157258: 2 mm base, 7.3 mm free, 5.5 mm '
                'compressed, 70 gf) on a %.1f mm pad; one per PM module aux contact, 2.54 mm grid' % POGO_PAD,
                'pogo pin spring PM module aux')
    fp.SetAttributes(pcbnew.FP_SMD)
    smd_pad(fp, '1', 0, 0, POGO_PAD)
    circle(fp, 0, 0, 1.0, pcbnew.F_Fab, 0.1)
    circle(fp, 0, 0, 1.25, pcbnew.F_CrtYd)
    fp.Reference().SetPosition(V(0, -2.0))
    fp.Reference().SetLayer(pcbnew.F_Fab)
    fp.Value().SetPosition(V(0, 2.0))
    fp.Value().SetLayer(pcbnew.F_Fab)
    return fp


def rotate(x, y, rot):
    """KiCad footprint rotation (degrees, counter-clockwise on screen, +y down)"""
    a = math.radians(rot)
    return (x * math.cos(a) + y * math.sin(a), -x * math.sin(a) + y * math.cos(a))


def host_parts(cx, cy, rot):
    """host-side positions for a module centred at (cx, cy), footprint rotation rot (0 bench, 270 tile):
    {'S1'..'S4': (x, y)} for the standoffs, {'A1'..'A6': (x, y)} for the pogo pins"""
    out = {}
    for n, (x, y) in SCREWS.items():
        dx, dy = rotate(x, y, rot)
        out['S' + n] = (round(cx + dx, 4), round(cy + dy, 4))
    for n in AUX_GRID:
        x, y = aux_pos(n)
        dx, dy = rotate(AUX_OFS[0] + x, AUX_OFS[1] + y, rot)
        out['A%d' % n] = (round(cx + dx, 4), round(cy + dy, 4))
    return out


def place(board, lib, cx, cy, rot, base, pogos, remove):
    """host board: delete the old socket footprints, add SO<base+1..4> standoffs (screws 1-4),
    PG<base+n> pogo pins for the aux contacts n in pogos and the board-only outline MOD<base // 100 or 1> for a module
    centred at (cx, cy); nets come from the netlist afterwards (pcbkit relink)"""
    pos = host_parts(cx, cy, rot)
    parts = [('SO%d' % (base + i + 1), 'PM_Standoff_M2x6_SMTSO', pos['S' + n]) for i, n in enumerate('1234')]
    parts += [('PG%d' % (base + n), 'PM_Pogo_YZ130115073P', pos['A%d' % n]) for n in pogos]
    parts += [('MOD%d' % max(1, base // 100), 'PM_Module_Outline', (cx, cy))]
    fps = [pcbnew.FootprintLoad(LIBS[lib], name) for _, name, _ in parts]    # before LoadBoard: after it the
    b = pcbnew.LoadBoard(board)                                              # IO plugin comes back unwrapped
    for r in remove:
        fp = b.FindFootprintByReference(r)
        if fp:
            b.Remove(fp)
    for (ref, name, (x, y)), fp in zip(parts, fps):
        old = b.FindFootprintByReference(ref)
        if old:
            b.Remove(old)
        fp.SetFPID(pcbnew.LIB_ID(lib, name))
        fp.SetReference(ref)
        b.Add(fp)
        fp.SetPosition(V(x, y))
        fp.SetOrientationDegrees(rot)
        print(ref, name, x, y, rot)
    b.Save(board)


def main():
    import sys
    if sys.argv[1:2] == ['place']:
        a = sys.argv[2:]
        return place(a[0], a[1], float(a[2]), float(a[3]), int(a[4]), int(a[5]), [int(n) for n in a[6].split(',')], a[7].split(',') if len(a) > 7 else [])
    io = pcbnew.PCB_IO_KICAD_SEXPR()
    for lib, fps in (('pm', [module_mount(), module_aux()]),
                     ('carrier', [host_outline(), host_standoff(), host_pogo()])):
        for fp in fps:
            io.FootprintSave(LIBS[lib], fp)
            path = os.path.join(LIBS[lib], fp.GetFPID().GetLibItemName().wx_str() + '.kicad_mod')
            txt = open(path).read()
            for w in ('0.1', '0.15'):
                tag = '%g' % (float(w) + DASH_TAG)
                txt = txt.replace('(width %s)\n\t\t\t(type default)' % tag, '(width %s)\n\t\t\t(type dash)' % w)
                txt = txt.replace('(width %s) (type default)' % tag, '(width %s) (type dash)' % w)
            open(path, 'w').write(txt)
            print('%s:%s' % (lib, fp.GetFPID().GetLibItemName()))


if __name__ == '__main__':
    main()
