#!/usr/bin/env python3
"""Footprints for the screwed PM module mount (docs/power-board.md decision 4).

The module is held by four M2 screws through plated holes at the DOSA
eighth-brick corner pin positions: screw 1 VIN (DOSA pin 1), 2 GND input
end (DOSA 3), 3 GND output end (DOSA 4), 4 VOUT (DOSA 8).
The screws go into SMT threaded standoffs (Sinhoo SMTSO2060CTJ, M2 x 6,
5.56 mm body, 3.6 mm pilot 1.53 mm long, LCSC C2915628) reflowed on the
plane (or bench carrier). Current flows through the clamped pads. The
signals reach the module through a 2x4 grid of pogo pins on the plane
(Xinyangze YZ130115073P-01, 2 mm base, 7.3 mm free, 5.5 mm compressed,
LCSC C5157258; 1.3 mm compression at the 6 mm gap) pressing on flat pads
on the module underside.

All footprints use the module's top-view coordinates (origin = module
centre, body 58.4 x 22.9 mm, pin 1 top left), like the old DOSA pattern,
so plane and module line up with the same placement. On the module the
screws are one footprint (PM_Mount_M2) and the aux pads another
(PM_Aux_Pads_2x04) placed at AUX_OFS from it. On the host every standoff
and pogo pin is its own one-pad part (JLC places one part per designator):
host_parts() gives their positions for a module centre, and the board-only
PM_Module_Outline draws the module on the silk.

Aux pin map (top view, pins 1-4 on the row nearer the pin 1/8 edge):
  1 OV_KILL  2 PGOOD  3 ID  4 TEMP  5 GND  6 SDA  7 SCL  8 RUN
The plane has no RUN pogo (the slot is switched upstream; RUN open = on).

Usage: pmmount.py   (writes into ../pm-19v-200w/pm.pretty, ../pp-48-2s/pp.pretty,
                     ../pm-bench-carrier/carrier.pretty)
       pmmount.py place <board> <lib> <cx> <cy> <rot> <ref base> <aux pins 1,2,..> [old,refs]
"""
import os

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
LIBS = {
    'pm': os.path.join(HERE, '..', 'pm-19v-200w', 'pm.pretty'),
    'pp': os.path.join(HERE, '..', 'pp-48-2s', 'pp.pretty'),
    'carrier': os.path.join(HERE, '..', 'pm-bench-carrier', 'carrier.pretty'),
}

BODY_W, BODY_H = 58.4, 22.9
SCREWS = {'1': (-25.4, -7.62), '2': (-25.4, 7.62), '3': (25.4, 7.62), '4': (25.4, -7.62)}   # DOSA corner pins 1 3 4 8
AUX_OFS = (12.7, -8.255)          # aux block centre from the mount origin
AUX_P = 2.54
AUX_NAMES = ['OV_KILL', 'PGOOD', 'ID', 'TEMP', 'GND', 'SDA', 'SCL', 'RUN']

# module side: M2 clearance hole, head pad on top, standoff face pad underneath
MOD_DRILL, MOD_F, MOD_IN, MOD_B = 2.4, 5.0, 4.0, 6.2
# plane side: standoff pilot hole, solder pad for the 5.56 mm body
SO_DRILL, SO_F, SO_IN, SO_B, SO_BODY = 3.75, 6.4, 5.0, 5.0, 5.56
MOD_AUX_PAD, POGO_PAD = 2.2, 2.2
DASH_TAG = 0.0011

I = pcbnew.FromMM
V = lambda x, y: pcbnew.VECTOR2I(I(round(x, 4)), I(round(y, 4)))


def aux_pos(n):
    k = int(n) - 1
    return ((k % 4 - 1.5) * AUX_P, (k // 4 - 0.5) * AUX_P)


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
    rect(fp, ax - 2 * AUX_P, ay - AUX_P, ax + 2 * AUX_P, ay + AUX_P, pcbnew.F_Fab, 0.1, dash=True)
    text(fp, 'AUX', ax, ay + AUX_P + 0.8, pcbnew.F_Fab, 0.6)
    fp.Reference().SetPosition(V(0, -BODY_H / 2 - 1.75))
    fp.Reference().SetLayer(pcbnew.F_SilkS)
    fp.Value().SetPosition(V(0, BODY_H / 2 + 1.75))
    fp.Value().SetLayer(pcbnew.F_Fab)
    for n, (x, y) in SCREWS.items():
        text(fp, n, x + (3.6 if x < 0 else -3.6), y, pcbnew.F_SilkS if silk else pcbnew.F_Fab)


def module_mount():
    fp = new_fp('PM_Mount_M2',
                'PM module mount, module side: four M2 screws (1 VIN, 2 GND input end, 3 GND output end, 4 VOUT at '
                'the DOSA eighth-brick corner pins 1 3 4 8) through 2.4 mm plated holes into SMT standoffs on the host. '
                'Head pad 5.0 mm on top, 6.2 mm standoff face pad underneath (5.56 mm body). Aux pads: '
                'PM_Aux_Pads_2x04 at (+12.7, -8.255).',
                'PM module mount M2 screw standoff')
    fp.SetAttributes(pcbnew.FP_THROUGH_HOLE)
    outline(fp, silk=False)
    for n, (x, y) in SCREWS.items():
        th_pad(fp, n, x, y, MOD_DRILL, MOD_F, MOD_IN, MOD_B)
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
    fp = new_fp('PM_Aux_Pads_2x04',
                'PM module aux contacts: 2x4 flat %.1f mm pads on B.Cu at 2.54 mm, no paste, ENIG; pogo pins '
                'on the host press on them (1 OV_KILL 2 PGOOD 3 ID 4 TEMP 5 GND 6 SDA 7 SCL 8 RUN)' % MOD_AUX_PAD,
                'PM module aux pogo target pads')
    fp.SetAttributes(pcbnew.FP_SMD | pcbnew.FP_EXCLUDE_FROM_POS_FILES)
    for n in range(1, 9):
        x, y = aux_pos(n)
        smd_pad(fp, str(n), x, y, MOD_AUX_PAD, back=True, paste=False)
    rect(fp, -2 * AUX_P - 0.5, -AUX_P - 0.5, 2 * AUX_P + 0.5, AUX_P + 0.5, pcbnew.B_CrtYd, 0.05)
    rect(fp, -2 * AUX_P, -AUX_P, 2 * AUX_P, AUX_P, pcbnew.B_Fab, 0.1)
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


def host_parts(cx, cy, rot):
    """host-side positions for a module centred at (cx, cy), footprint rotation rot (0 or 180):
    {'S1'..: (x, y)} for the standoffs at screw 1/3/4/8, {'A1'..'A8': (x, y)} for the pogo pins"""
    sgn = -1 if rot == 180 else 1
    out = {}
    for n, (x, y) in SCREWS.items():
        out['S' + n] = (round(cx + sgn * x, 4), round(cy + sgn * y, 4))
    for n in range(1, 9):
        x, y = aux_pos(n)
        out['A%d' % n] = (round(cx + sgn * (AUX_OFS[0] + x), 4), round(cy + sgn * (AUX_OFS[1] + y), 4))
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
                     ('pp', [host_outline(), host_standoff(), host_pogo()]),
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
