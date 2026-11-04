#!/usr/bin/env python3
"""Generate controller-fd/schematic/canlink.kicad_sch (CAN link sheet).

Every pin gets a short wire stub ending in a label, a global label or a
power symbol; no wire crosses another, so no junctions are needed.
"""
import re
import sys
import uuid as _uuid

sys.path.insert(0, __file__.rsplit('/', 1)[0])
from sx import parse, find

PRJ = sys.argv[1]  # .../controller-fd
SCH = PRJ + '/schematic'
SYS = '/usr/share/kicad/symbols'
SHEET_UUID = '7c1d5a0e-3f2b-4e8a-9b61-2d4c8e5f0a17'
ROOT_UUID = 'bd3d1081-8fd5-561e-93ea-9d28b3145149'
PROJECT = 'granite-controller-fd'

NS = _uuid.UUID('6b1c3a52-0f7e-4c35-9d0a-5a7e2f1c0fd0')
_n = [0]


def uid():
    _n[0] += 1
    return str(_uuid.uuid5(NS, 'canlink-%d' % _n[0]))


def block(text, head):
    """Return the balanced s-expression starting at text.index(head)."""
    i = text.index(head)
    d = 0
    for j in range(i, len(text)):
        c = text[j]
        if c == '(':
            d += 1
        elif c == ')':
            d -= 1
            if d == 0:
                return text[i:j + 1]
    raise ValueError(head)


def embedded(path, libid):
    """Embedded lib symbol from an existing sheet's lib_symbols."""
    t = open(path).read()
    ls = block(t, '\t(lib_symbols')
    return '\t\t' + block(ls, '(symbol "%s"' % libid)


def from_lib(lib, name):
    t = open('%s/%s.kicad_sym' % (SYS, lib)).read()
    s = block(t, '\t(symbol "%s"\n' % name)
    s = s.replace('(symbol "%s"' % name, '(symbol "%s:%s"' % (lib, name), 1)
    # re-indent one level (library is at 1 tab, lib_symbols children at 2)
    return '\n'.join(('\t' + ln) if ln else ln for ln in s.split('\n'))


def pins_of(symtext):
    n = parse(symtext)
    out = {}

    def walk(x):
        for c in x:
            if isinstance(c, list):
                if c and c[0] == 'pin':
                    at = find(c, 'at')[0]
                    num = find(c, 'number')[0][1]
                    out.setdefault(num, (float(at[1]), float(at[2]), int(at[3])))
                else:
                    walk(c)
    walk(n)
    return out


LIBSYM = {}
LIBSYM['Device:R'] = embedded(SCH + '/ethernet.kicad_sch', 'Device:R')
LIBSYM['Device:C'] = embedded(SCH + '/ethernet.kicad_sch', 'Device:C')
LIBSYM['Device:Crystal_GND24'] = embedded(SCH + '/ethernet.kicad_sch', 'Device:Crystal_GND24')
LIBSYM['power:GND'] = embedded(SCH + '/ethernet.kicad_sch', 'power:GND')
LIBSYM['power:+3V3'] = embedded(SCH + '/ethernet.kicad_sch', 'power:+3V3')
LIBSYM['power:+5V'] = embedded(SCH + '/power.kicad_sch', 'power:+5V')
LIBSYM['Interface_CAN_LIN:MCP2517FD-xSL'] = from_lib('Interface_CAN_LIN', 'MCP2517FD-xSL')
LIBSYM['Interface_CAN_LIN:TJA1049T-3'] = from_lib('Interface_CAN_LIN', 'TJA1049T-3')
LIBSYM['Power_Protection:NUP2105L'] = from_lib('Power_Protection', 'NUP2105L')
LIBSYM['Connector_Generic:Conn_02x03_Odd_Even'] = from_lib('Connector_Generic', 'Conn_02x03_Odd_Even')
PINS = {k: pins_of(v) for k, v in LIBSYM.items()}

out = []          # top-level items (strings)
pwr_n = [2000]


def fmt(v):
    s = ('%.4f' % v).rstrip('0').rstrip('.')
    return '0' if s == '-0' else s


def font(justify=None, hide=False):
    s = '(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)'
    if justify:
        s += '\n\t\t\t\t(justify %s)' % justify
    if hide:
        s += '\n\t\t\t\t(hide yes)'
    return s + '\n\t\t\t)'


def prop(name, value, x, y, hide=False, justify=None, ang=0):
    return ('\t\t(property "%s" "%s"\n\t\t\t(at %s %s %d)\n%s\t\t\t(show_name no)\n'
            '\t\t\t(do_not_autoplace no)\n\t\t\t%s\n\t\t)' % (
                name, value, fmt(x), fmt(y), ang,
                '\t\t\t(hide yes)\n' if hide else '', font(justify)))


def symbol(libid, ref, value, x, y, fields=None, power=False, ref_off=(2.54, -1.27), val_off=(2.54, 1.27), ang=0, d='D'):
    fields = fields or {}
    lines = ['\t(symbol', '\t\t(lib_id "%s")' % libid, '\t\t(at %s %s %d)' % (fmt(x), fmt(y), ang),
             '\t\t(unit 1)', '\t\t(body_style 1)', '\t\t(exclude_from_sim no)',
             '\t\t(in_bom %s)' % ('no' if power else 'yes'),
             '\t\t(on_board %s)' % ('no' if power else 'yes'),
             '\t\t(in_pos_files %s)' % ('no' if power else 'yes'),
             '\t\t(dnp no)', '\t\t(fields_autoplaced no)', '\t\t(uuid "%s")' % uid()]
    if power:
        lines.append(prop('Reference', ref, x, y + 6.35, hide=True))
        dx, dy, _ = DIRS[d]
        lines.append(prop('Value', value, x + dx * 5.08, y + dy * 5.08))
    else:
        lines.append(prop('Reference', ref, x + ref_off[0], y + ref_off[1], justify='left'))
        lines.append(prop('Value', value, x + val_off[0], y + val_off[1], justify='left'))
    for k in ('Footprint', 'Datasheet', 'Description'):
        lines.append(prop(k, fields.get(k, ''), x, y, hide=True))
    for k in ('MANUFACTURER', 'MPN', 'LCSC'):
        if k in fields:
            lines.append(prop(k, fields[k], x, y, hide=True))
    for num in PINS[libid]:
        lines.append('\t\t(pin "%s"\n\t\t\t(uuid "%s")\n\t\t)' % (num, uid()))
    lines.append('\t\t(instances\n\t\t\t(project "%s"\n\t\t\t\t(path "/%s/%s"\n'
                 '\t\t\t\t\t(reference "%s")\n\t\t\t\t\t(unit 1)\n\t\t\t\t)\n\t\t\t)\n\t\t)'
                 % (PROJECT, ROOT_UUID, SHEET_UUID, ref))
    lines.append('\t)')
    out.append('\n'.join(lines))


def wire(x1, y1, x2, y2):
    out.append('\t(wire\n\t\t(pts\n\t\t\t(xy %s %s) (xy %s %s)\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)\n'
               '\t\t\t(type default)\n\t\t)\n\t\t(uuid "%s")\n\t)' % (fmt(x1), fmt(y1), fmt(x2), fmt(y2), uid()))


DIRS = {'L': (-1, 0, 180), 'R': (1, 0, 0), 'U': (0, -1, 90), 'D': (0, 1, 270)}


def label(name, x, y, d):
    ang = DIRS[d][2]
    just = 'left bottom' if ang in (0, 90) else 'right bottom'
    out.append('\t(label "%s"\n\t\t(at %s %s %d)\n\t\t%s\n\t\t(uuid "%s")\n\t)'
               % (name, fmt(x), fmt(y), ang, font(just).replace('\n\t\t\t', '\n\t\t').replace('\t\t\t\t', '\t\t\t'), uid()))


def glabel(name, x, y, d, shape):
    ang = DIRS[d][2]
    just = 'left' if ang in (0, 90) else 'right'
    f = font(just).replace('\n\t\t\t', '\n\t\t').replace('\t\t\t\t', '\t\t\t')
    out.append('\t(global_label "%s"\n\t\t(shape %s)\n\t\t(at %s %s %d)\n\t\t(fields_autoplaced yes)\n\t\t%s\n'
               '\t\t(uuid "%s")\n\t\t(property "Intersheetrefs" "${INTERSHEET_REFS}"\n\t\t\t(at %s %s 0)\n'
               '\t\t\t%s\n\t\t)\n\t)' % (name, shape, fmt(x), fmt(y), ang, f, uid(), fmt(x), fmt(y),
                                        font(just, hide=True)))


def no_connect(x, y):
    out.append('\t(no_connect\n\t\t(at %s %s)\n\t\t(uuid "%s")\n\t)' % (fmt(x), fmt(y), uid()))


def power(name, x, y, d):
    pwr_n[0] += 1
    if name == 'GND':
        ang = {'D': 0, 'L': 270, 'R': 90, 'U': 180}[d]
    else:
        ang = {'U': 0, 'L': 90, 'R': 270, 'D': 180}[d]
    symbol('power:' + name, '#PWR%d' % pwr_n[0], name, x, y, power=True, ang=ang, d=d)


def text(s, x, y):
    out.append('\t(text "%s"\n\t\t(exclude_from_sim no)\n\t\t(at %s %s 0)\n\t\t%s\n\t\t(uuid "%s")\n\t)'
               % (s, fmt(x), fmt(y), font('left bottom').replace('\n\t\t\t', '\n\t\t').replace('\t\t\t\t', '\t\t\t'), uid()))


def outward(ang):
    # lib pin angle -> outward direction on the sheet (symbol not rotated)
    return {0: 'L', 180: 'R', 90: 'D', 270: 'U'}[ang]


def part(libid, ref, value, x, y, conns, fields, stub=2.54, **kw):
    symbol(libid, ref, value, x, y, fields, **kw)
    done = set()
    for num, c in conns.items():
        px, py, ang = PINS[libid][num]
        sx, sy = x + px, y - py
        if (sx, sy) in done:
            continue
        done.add((sx, sy))
        if c[0] == 'nc':
            no_connect(sx, sy)
            continue
        d = outward(ang)
        dx, dy, _ = DIRS[d]
        ex, ey = sx + dx * stub, sy + dy * stub
        wire(sx, sy, ex, ey)
        if c[0] == 'net':
            label(c[1], ex, ey, d)
        elif c[0] == 'g':
            glabel(c[1], ex, ey, d, c[2])
        elif c[0] == 'pwr':
            power(c[1], ex, ey, d)


R0402 = 'Resistor_SMD:R_0402_1005Metric'
C0402 = 'Capacitor_SMD:C_0402_1005Metric'
C0603 = 'Capacitor_SMD:C_0603_1608Metric'


def res(ref, val, x, y, top, bot, lcsc, mpn, mfr='UNI-ROYAL'):
    part('Device:R', ref, val, x, y, {'1': top, '2': bot},
         {'Footprint': R0402, 'MANUFACTURER': mfr, 'MPN': mpn, 'LCSC': lcsc})


def cap(ref, val, x, y, top, bot, lcsc, mpn, mfr, fp=C0402):
    part('Device:C', ref, val, x, y, {'1': top, '2': bot},
         {'Footprint': fp, 'MANUFACTURER': mfr, 'MPN': mpn, 'LCSC': lcsc})


GND = ('pwr', 'GND')
P3V3 = ('pwr', '+3V3')
P5V = ('pwr', '+5V')

# ---------------------------------------------------------------- notes
text('CAN link (ADR 0006 decision 9): MCP2518FD CAN FD controller on the W5500 SPI bus (GPIO19 SCLK / GPIO20 MOSI / GPIO21 MISO), '
     'nCS GPIO2, nINT GPIO3; TCAN1044V transceiver, STB on GPIO0.', 20.32, 25.4)
text('Not isolated: the controller ground is LINK_GND (link pins 3 / 4); its 12 V is AUX_12V (pins 5 / 6), OR-ed from every core.', 20.32, 29.21)
text('MCP2518FD: SPI <= 17 MHz at SYSCLK 40 MHz (its own transactions; W5500 nCS GPIO18 stays). nINT push-pull after reset (IOCON.INTOD = 0), '
     '10k pull-up holds it high before init. GPIO0 / GPIO1 / CLKO unused; INT0 / INT1 have no pull-ups here: firmware sets them as outputs, low.', 20.32, 33.02)
text('Symbol: KiCad MCP2517FD-xSL (MCP2518FD is the pin-compatible successor, SOIC-14).', 20.32, 36.83)

# ---------------------------------------------------------------- MCP2518FD
MX, MY = 101.6, 96.52
part('Interface_CAN_LIN:MCP2517FD-xSL', 'U23', 'MCP2518FDT-E/SL', MX, MY, {
    '10': ('g', 'ETH_SCLK', 'input'),
    '11': ('g', 'ETH_MOSI', 'input'),
    '12': ('net', 'MCP_SDO'),
    '13': ('g', 'CAN_nCS', 'input'),
    '5': ('net', 'MCP_OSC2'),
    '6': ('net', 'MCP_OSC1'),
    '1': ('net', 'CAN_TXD'),
    '2': ('net', 'CAN_RXD'),
    '3': ('nc',),
    '4': ('g', 'CAN_nINT', 'output'),
    '8': ('nc',),
    '9': ('nc',),
    '14': P3V3,
    '7': GND,
}, {'Footprint': 'Package_SO:SOIC-14_3.9x8.7mm_P1.27mm',
    'Datasheet': 'https://ww1.microchip.com/downloads/aemDocuments/documents/APID/ProductDocuments/DataSheets/External-CAN-FD-Controller-with-SPI-Interface-DS20006027B.pdf',
    'Description': 'External CAN FD controller with SPI interface, SOIC-14',
    'MANUFACTURER': 'Microchip', 'MPN': 'MCP2518FDT-E/SL', 'LCSC': 'C626758'},
    stub=5.08, ref_off=(2.54, -15.24), val_off=(2.54, 15.24))

# SDO series R: both SDO and the W5500 MISO are tri-state outputs on one line
part('Device:R', 'R79', '33', 55.88, 88.9, {'1': ('net', 'MCP_SDO'), '2': ('g', 'ETH_MISO', 'output')},
     {'Footprint': R0402, 'MANUFACTURER': 'UNI-ROYAL', 'MPN': '0402WGF330JTCE', 'LCSC': 'C25105'})
text('R79: MCP2518FD SDO onto the shared MISO (tri-state when nCS is high).', 20.32, 160.02)
cap('C57', '100n', 121.92, 68.58, P3V3, GND, 'C1525', 'CL05B104KO5NNNC', 'Samsung')
text('U23 VDD', 116.84, 60.96)
res('R75', '10k', 76.2, 68.58, P3V3, ('g', 'CAN_nCS', 'input'), 'C25744', '0402WGF1002TCE')
res('R76', '10k', 86.36, 68.58, P3V3, ('g', 'CAN_nINT', 'output'), 'C25744', '0402WGF1002TCE')

# 40 MHz crystal, CL 15 pF: 2 x (15 - ~4 pF stray) = 22 pF
YX, YY = 63.5, 121.92
part('Device:Crystal_GND24', 'Y3', '40MHz 15pF YXC', YX, YY, {
    '1': ('net', 'MCP_OSC1'), '3': ('net', 'MCP_OSC2'), '2': GND, '4': GND,
}, {'Footprint': 'Crystal:Crystal_SMD_Abracon_ABM8G-4Pin_3.2x2.5mm',
    'Description': '40 MHz crystal, CL 15 pF, 3225',
    'MANUFACTURER': 'YXC', 'MPN': 'X322540MPB4SI', 'LCSC': 'C9010'},
    ref_off=(-2.54, -5.08), val_off=(6.35, -5.08))
cap('C58', '22p', 50.8, 134.62, ('net', 'MCP_OSC1'), GND, 'C1555', '0402CG220J500NT', 'FH')
cap('C59', '22p', 76.2, 134.62, ('net', 'MCP_OSC2'), GND, 'C1555', '0402CG220J500NT', 'FH')
text('Y3 load: CL 15 pF -> 22 pF each', 20.32, 156.21)

# ---------------------------------------------------------------- TCAN1044V
TX, TY = 182.88, 96.52
part('Interface_CAN_LIN:TJA1049T-3', 'U24', 'TCAN1044VDRQ1', TX, TY, {
    '1': ('net', 'CAN_TXD'),
    '4': ('net', 'CAN_RXD'),
    '5': P3V3,
    '8': ('g', 'CAN_STB', 'input'),
    '3': P5V,
    '2': GND,
    '7': ('net', 'CANH'),
    '6': ('net', 'CANL'),
}, {'Footprint': 'Package_SO:SOIC-8_3.9x4.9mm_P1.27mm',
    'Datasheet': 'https://www.ti.com/lit/ds/symlink/tcan1044v-q1.pdf',
    'Description': 'CAN FD transceiver 8 Mbit/s, VIO, STB; TJA1049T-3 symbol (same pinout)',
    'MANUFACTURER': 'TI', 'MPN': 'TCAN1044VDRQ1', 'LCSC': 'C1852061'},
    stub=5.08, ref_off=(2.54, -12.7), val_off=(2.54, 12.7))
text('STB: GPIO0, internal pull-up to VIO = standby until firmware drives it low (normal mode).', 154.94, 121.92)
text('VCC +5V (BUCK_5V via the SS34 OR, ~5.0 V loaded; 4.5-5.5 V), VIO +3V3.', 154.94, 125.73)

cap('C60', '100n', 203.2, 68.58, P5V, GND, 'C1525', 'CL05B104KO5NNNC', 'Samsung')
cap('C61', '4.7u', 213.36, 68.58, P5V, GND, 'C19666', 'CL10A475KO8NNNC', 'Samsung', fp=C0603)
cap('C62', '100n', 223.52, 68.58, P3V3, GND, 'C1525', 'CL05B104KO5NNNC', 'Samsung')
text('U24 VCC / VIO', 200.66, 60.96)

# ---------------------------------------------------------------- link box header
JX, JY = 314.96, 96.52
part('Connector_Generic:Conn_02x03_Odd_Even', 'J15', 'PLANE_LINK', JX, JY, {
    '1': ('net', 'CANH'), '2': ('net', 'CANL'),
    '3': GND, '4': GND,
    '5': ('g', 'AUX_12V', 'output'), '6': ('g', 'AUX_12V', 'output'),
}, {'Footprint': 'Connector_IDC:IDC-Header_2x03_P2.54mm_Horizontal',
    'Datasheet': 'https://www.lcsc.com/datasheet/lcsc_datasheet_1810161728_BOOMELE-Boom-Precision-Elec-2-54-2-3P_C34945.pdf',
    'Description': 'Plane link, keyed 2x3 right-angle box header at the right edge (6-pin IDC ribbon to core 1): 1 CANH, 2 CANL, 3/4 LINK_GND, 5/6 AUX_12V',
    'MANUFACTURER': 'BOOMELE', 'MPN': '2.54-2*3P Bent Header', 'LCSC': 'C34945'},
    stub=7.62, ref_off=(0, -6.35), val_off=(0, 7.62))
text('J15: keyed 2x3 right-angle box header; pre-made 6-pin IDC ribbon to core 1 link plug (ADR 0006 decision 9).', 284.48, 76.2)
text('Ribbon pin n to plug pin n: 1 CANH, 2 CANL, 3 / 4 LINK_GND (= controller GND), 5 / 6 AUX_12V.', 284.48, 80.01)

# ESD at the header
part('Power_Protection:NUP2105L', 'D13', 'PESD2CANFD24V-TR', 287.02, 134.62, {
    '1': ('net', 'CANH'), '2': ('net', 'CANL'), '3': GND,
}, {'Footprint': 'Package_TO_SOT_SMD:SOT-23',
    'Datasheet': 'https://www.lcsc.com/datasheet/lcsc_datasheet_2407251001_Nexperia-PESD2CANFD24V-TR_C552486.pdf',
    'Description': '2-line CAN FD ESD protection, 24 V, 6 pF, SOT-23',
    'MANUFACTURER': 'Nexperia', 'MPN': 'PESD2CANFD24V-TR', 'LCSC': 'C552486'},
    ref_off=(5.08, -1.27), val_off=(5.08, 1.27))

# split termination, always fitted (left end of the row)
res('R77', '60.4', 241.3, 132.08, ('net', 'CANH'), ('net', 'CAN_SPLIT'), 'C60310', '0402WGF604JTCE')
res('R78', '60.4', 248.92, 132.08, ('net', 'CAN_SPLIT'), ('net', 'CANL'), 'C60310', '0402WGF604JTCE')
cap('C63', '4.7n', 256.54, 132.08, ('net', 'CAN_SPLIT'), GND, 'C1538', '0402B472K500NT', 'FH')
text('Split termination, always fitted: 2 x 60.4R + 4.7 nF to LINK_GND (ADR 0006 decision 8).', 228.6, 152.4)
text('ESD D13 at J15: Nexperia PESD2CANFD24V (24 V, 6 pF per line).', 228.6, 156.21)

# ---------------------------------------------------------------- write
libsyms = '\n'.join(LIBSYM[k] for k in sorted(LIBSYM))
doc = ('(kicad_sch\n\t(version 20260306)\n\t(generator "eeschema")\n\t(generator_version "10.0")\n'
       '\t(uuid "%s")\n\t(paper "A3")\n\t(title_block\n\t\t(title "Granite controller FD - CAN link")\n'
       '\t\t(date "2026-11-04")\n\t\t(rev "FD")\n\t)\n\t(lib_symbols\n%s\n\t)\n%s\n'
       '\t(sheet_instances\n\t\t(path "/"\n\t\t\t(page "1")\n\t\t)\n\t)\n\t(embedded_fonts no)\n)\n'
       % (SHEET_UUID, libsyms, '\n'.join(out)))
open(SCH + '/canlink.kicad_sch', 'w').write(doc)
print('wrote', SCH + '/canlink.kicad_sch', len(out), 'items')
