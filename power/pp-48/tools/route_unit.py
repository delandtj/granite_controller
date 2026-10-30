#!/usr/bin/env python3
"""Scripted copper of the PP-48 unit board (group "hand", kept by krt-route).
Usage: route_unit.py <in.kicad_pcb> <out.kicad_pcb>

docs/adr/0005-chained-plane-boards.md. Run after tools/place_unit.py.

- Slots A (x 100) and B (x 125): tools/route_hand.py's column copper (power
  pours, gate hops, Kelvin, OV latch / arm FF escapes, node pours) and slot
  A's In2 bus to the expander in slot B, unchanged; rev1's In2 channel is
  replaced by the band below.
- In2 band (the long nets, off the router): ten rows at v 59.85 + 0.3 i (the
  rev1 channel height, between the standoff pads and the node connectors)
  from J301's pins, row order = pin order (top pin, top row, outermost
  vertical), so nothing crosses. Rows 0-7 (EXP_RSTN, PRESS_EN, CH3..CH1:
  everything J302 passes on) cross both slots into the strip, each up to a
  pad via of the ladder at u 0.4 + 0.6 i (strip-local), krt-route takes them
  on to J302 on F / B; the band ends in the strip's left edge, so the
  strip's In2 stays one piece of the VBUS plane. Rows 8 / 9 (SEG_SCL /
  SEG_SDA) end on tap pad vias in slot A. EXP_RSTN also has its own row from
  pin 14 to a tap by the expander U5. +3V3: rev1's trunk (v 64.95, 0.4 mm)
  from J301 pin 1 to J302 pin 14 with a tap pad via per slot. In2 no-via
  rule areas over all of it; tools/unitsplit.py cuts these nets into the
  pieces krt-route has to join (margin / slot / strip).
- Strip power path on F: VIN zone column (XT60 -> C201 -> D201 -> shunt VIN
  pads), USENSE zone (shunt USENSE pads -> FET drain tabs), VBUS zone (FET
  sources along the bottom edge, up the column right of J302's pins) with a
  via field to In2; F rule areas (no tracks) over them keep krt-route out,
  KRT skips zone nets so the VIN / USENSE small pads are drawn here: U207
  SENSE / VIN and U208 IN- / IN+ Kelvin to the shunt pads, R233 / R235 VIN
  pads by vias into the VIN column. Gates: pin 4 -> pad via under the VBUS
  pour, krt-route takes it to R210 / R211 on B.
- GND pours F / In1 / B over the board, priority 0 (the power pours win), and
  GND vias at the TVS and bulk cap GND pads.
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import route_hand as rh                  # noqa: E402  (rev1 slot copper)

X0, Y0 = rh.X0, rh.Y0                    # slot A origin (100, 100)
LEFT, RIGHT, BOTTOM = X0 - 9.0, X0 + 69.5, Y0 + 72.0
H4 = (166.0, 168.5)                     # strip hole (tools/place_unit.py)
SX = X0 + 50.0                           # strip origin
GROUP = 'hand'
# In2 band: J301 pins (pin, net) top-down = rows 0..9; rows 0-7 run to the ladder in the strip (krt-route on to
# J302), rows 8 / 9 (the segment) end on tap vias in slot A (krt-route on to the segment's parts)
J301_X = 97.59
BAND = [(14, '/EXP_RSTN'), (13, '/PRESS_EN'), (12, '/Chain/CH3_SCL'), (11, '/Chain/CH3_SDA'), (9, '/Chain/CH2_SCL'),
        (8, '/Chain/CH2_SDA'), (6, '/Chain/CH1_SCL'), (5, '/Chain/CH1_SDA'), (3, '/SEG_SCL'), (2, '/SEG_SDA')]
LADDER = 8                               # rows 0..7
SEG_TAPS = {8: X0 + 10.8, 9: X0 + 9.6}   # row -> tap x (tap via 0.65 below the row's end), left of J1
ROW0, ROW = 59.85, 0.3                   # row v (slot-local, as rev1)
VERT0 = 98.8 + 0.3 * 9                   # row 0's vertical (absolute x), the next ones 0.3 further left (row 9 at 98.8)
LAD_U, LAD_DU, LAD_V, LAD_DV = 0.4, 0.6, 59.35, 0.45   # ladder via i at (SX + LAD_U + LAD_DU i, LAD_V - LAD_DV i)
TRUNK_Y = Y0 + 64.95                    # +3V3 trunk (rev1 height)
TAPS = (X0 + 11.6, X0 + 36.6)           # trunk tap vias, slot A / B, left of the node terminals J1 / J51
RST_Y, RST_TAP = Y0 + 31.9, (136.0, Y0 + 31.9)   # EXP_RSTN row J301 pin 14 -> tap by U5 / R30 (above slot A's escapes)


class Unit(rh.Col):
    """slot column with the unit board's net names (/Unit/A_X, /Unit/B_X)"""

    def net(self, name):
        if name in ('GND', 'VBUS', '+3V3'):
            full = name
        else:
            full = '/Unit/%s_%s' % (self.slot, name)
        ni = self.b.FindNet(full)
        if ni is None:
            raise SystemExit('no net ' + full)
        return ni


class Abs(rh.Col):
    """absolute coordinates, nets by full name"""

    def __init__(self, board, group):
        self.b, self.g, self.c, self.n = board, group, 0, 0
        self.x0 = 0.0

    def p(self, x, y):
        return pcbnew.VECTOR2I(rh.mm(x), rh.mm(y))

    def net(self, name):
        ni = self.b.FindNet(name) or self.b.FindNet('/' + name)
        if ni is None:
            raise SystemExit('no net ' + name)
        return ni

    def zone(self, net, layer, pts, prio=3):
        z = pcbnew.ZONE(self.b)
        z.SetLayer(layer); z.SetNet(self.net(net)); z.SetAssignedPriority(prio)
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)
        z.SetMinThickness(rh.mm(0.25)); z.SetLocalClearance(rh.mm(0.2))
        ol = z.Outline(); ol.NewOutline()
        for x, y in pts:
            ol.Append(rh.mm(x), rh.mm(y))
        self.b.Add(z); self.g.AddItem(z)


def pad_at(board, ref, num):
    for p in board.FindFootprintByReference(ref).Pads():
        if p.GetNumber() == num:
            return pcbnew.ToMM(p.GetPosition().x), pcbnew.ToMM(p.GetPosition().y)
    raise SystemExit('%s has no pad %s' % (ref, num))


def band(k):
    """In2 band J301 -> ladder in the strip / segment taps (absolute coordinates)"""
    IN2 = pcbnew.In2_Cu
    rows = [Y0 + ROW0 + ROW * i for i in range(len(BAND))]
    for i, (pin, net) in enumerate(BAND):
        py = pad_at(k.b, 'J301', str(pin))[1]
        x = VERT0 - ROW * i
        head = [(J301_X, py), (x - 0.3, py), (x, py + 0.3), (x, rows[i] - 0.3), (x + 0.3, rows[i])]
        if i < LADDER:
            lx, ly = SX + LAD_U + LAD_DU * i, Y0 + LAD_V - LAD_DV * i
            k.track(net, head + [(lx - 0.3, rows[i]), (lx, rows[i] - 0.3), (lx, ly)], 0.15, IN2)
            k.pad_via(net, (lx, ly), 'BV9%02d' % i)
        else:                                   # segment: down to a tap (the rows below end further left)
            tx = SEG_TAPS[i]
            ty = rows[-1] + 0.65
            k.track(net, head + [(tx - 0.3, rows[i]), (tx, rows[i] + 0.3), (tx, ty)], 0.15, IN2)
            k.pad_via(net, (tx, ty), 'SV%d' % i)
    # EXP_RSTN to the expander: its own In2 row from J301 pin 14 up and right to a tap by U5 / R30
    px, py = pad_at(k.b, 'J301', '14')
    k.track('/EXP_RSTN', [(px, py), (px, RST_Y + 0.4), (px + 0.4, RST_Y), RST_TAP], 0.15, IN2)
    k.pad_via('/EXP_RSTN', RST_TAP, 'RV1')
    k.track('/EXP_RSTN', [RST_TAP, pad_at(k.b, 'R30', '1')], 0.2, pcbnew.B_Cu)   # tap -> /RESET pull-up (krt-route on to U5)
    rh.rule_area(k.b, 'band rst', (IN2,), [(px - 0.4, RST_Y - 0.4), (RST_TAP[0] - 0.4, RST_Y - 0.4), (RST_TAP[0] - 0.4, RST_Y + 0.4),
                                           (px + 0.4, RST_Y + 0.4), (px + 0.4, py - 0.9), (px - 0.4, py - 0.9)],
                 keep_tracks=True, no_vias=True)
    # +3V3 trunk on In2 (rev1's, 0.4 mm): J301 pin 1 down below the band, right to J302 pin 14, one tap pad
    # via per slot (tools/unitsplit.py gives KRT one +3V3 section per slot and one in the strip)
    (ax, ay), (bx, by) = pad_at(k.b, 'J301', '1'), pad_at(k.b, 'J302', '14')
    k.track('+3V3', [(ax, ay), (ax, TRUNK_Y - 0.4), (ax + 0.4, TRUNK_Y), (bx - 0.4, TRUNK_Y), (bx, TRUNK_Y - 0.4), (bx, by)], 0.4, IN2)
    for i, x in enumerate(TAPS):
        k.pad_via('+3V3', (x, TRUNK_Y), 'PV3%s' % 'AB'[i])
    rh.rule_area(k.b, 'band trunk', (IN2,), [(ax - 0.5, TRUNK_Y - 0.5), (bx + 0.5, TRUNK_Y - 0.5), (bx + 0.5, TRUNK_Y + 0.5),
                                             (ax - 0.5, TRUNK_Y + 0.5)], keep_tracks=True, no_vias=True)
    # KRT does not see In2 when it places vias: none through the band or the J301 verticals
    xl = VERT0 - ROW * (len(BAND) - 1) - 0.4
    pts = [(xl, Y0 + 34.6), (VERT0 + 0.4, Y0 + 34.6), (VERT0 + 0.4, rows[0] - 0.4),
           (SX + LAD_U - 0.4, rows[0] - 0.4), (SX + LAD_U - 0.4, rows[LADDER - 1] + 0.4),
           (SEG_TAPS[8] + 0.4, rows[LADDER - 1] + 0.4), (SEG_TAPS[8] + 0.4, rows[-1] + 0.4), (xl, rows[-1] + 0.4)]
    rh.rule_area(k.b, 'band', (IN2,), pts, keep_tracks=True, no_vias=True)


def escapes(k):
    """U5's boxed-in pins out to vias (krt-route takes them on from there): top row SDA (30) / SCL (29)
    up between the supply escape vias, /RESET (28) straight up into the EXP_RSTN row's tap via; U_PGD
    (24, top right corner) right and up, B_DISARM (23) and B_OFF (22) right under it to staggered vias; U_OFF (12, bottom row) down to a via where slot B's LED
    resistors R71 / R72 used to sit (tools/place_unit.py moves them down), B_PWRD (13) and B_RUNOFF (16) down to vias"""
    F = pcbnew.F_Cu
    pin = lambda n: pad_at(k.b, 'U5', n)
    (x, y) = pin('30')
    k.track('/SEG_SDA', [(x, y), (x, y - 1.16), (x - 0.5, y - 1.66), (x - 0.5, y - 1.96)], 0.2, F)
    k.pad_via('/SEG_SDA', (x - 0.5, y - 1.96), 'EV30')
    (x, y) = pin('29')
    k.track('/SEG_SCL', [(x, y), (x, y - 1.61), (x - 0.15, y - 1.76)], 0.2, F)
    k.pad_via('/SEG_SCL', (x - 0.15, y - 1.76), 'EV29')
    (x, y) = pin('28')                          # straight up into the EXP_RSTN row's tap (band())
    k.track('/EXP_RSTN', [(x, y), (x, RST_TAP[1] + 0.5), (x - 0.5, RST_TAP[1]), RST_TAP], 0.2, F)
    # pins 24 / 23 / 22 (U_PGD, B_DISARM, B_OFF) all leave upward: staggered vias up-left of each other, left of R62
    (x, y) = pin('24')
    k.track('/U_PGD', [(x, y), (x + 0.36, y), (x + 0.76, y - 0.4), (x + 0.76, y - 0.95)], 0.2, F)
    k.pad_via('/U_PGD', (x + 0.76, y - 0.95), 'EV24')
    (x, y) = pin('23')
    k.track('/Unit/B_DISARM', [(x, y), (x + 1.06, y), (x + 1.36, y - 0.3), (x + 1.36, y - 0.85)], 0.2, F)
    k.pad_via('/Unit/B_DISARM', (x + 1.36, y - 0.85), 'EV23')
    (x, y) = pin('22')
    k.track('/Unit/B_OFF', [(x, y), (x + 1.76, y), (x + 2.16, y - 0.4), (x + 2.16, y - 0.65)], 0.2, F)
    k.pad_via('/Unit/B_OFF', (x + 2.16, y - 0.65), 'EV22')
    (x, y) = pin('12')
    k.track('/U_OFF', [(x, y), (x, y + 1.36)], 0.2, F)
    k.pad_via('/U_OFF', (x, y + 1.36), 'EV12')
    # bottom row: B_PWRD (13) down past EV12 to a via between R71 / R72's top pads (B goes on down between
    # them to R69), B_RUNOFF (16) down to a via under the pin
    (x, y) = pin('13')
    k.track('/Unit/B_PWRD', [(x, y), (x, y + 2.0)], 0.2, F)
    k.pad_via('/Unit/B_PWRD', (x, y + 2.0), 'EV13')
    (x, y) = pin('16')
    k.track('/Unit/B_RUNOFF', [(x, y), (x, y + 1.0)], 0.2, F)
    k.pad_via('/Unit/B_RUNOFF', (x, y + 1.0), 'EV16')
    # U_OFF into the strip on In2: EXP_RSTN / PRESS_EN climb the strip's left edge from the band ladder and
    # cross its top to J302's top pins, which closes a loop around Q204 on F / B. Short hop from slot B's top
    # right corner under the strip edge to Q204 pin 1 (along the VBUS current from strip to slots, so the slit
    # in the In2 plane is minor); krt-route joins EV12 to UV1 inside slot B
    q = pad_at(k.b, 'Q204', '1')
    a, z = (148.8, 130.8), (q[0], q[1] + 1.2)
    k.pad_via('/U_OFF', a, 'UV1')
    k.via('/U_OFF', z)
    k.track('/U_OFF', [a, z], 0.2, pcbnew.In2_Cu)
    k.track('/U_OFF', [z, q], 0.2, pcbnew.B_Cu)
    m = lambda t: (a[0] + (z[0] - a[0]) * t, a[1] + (z[1] - a[1]) * t)   # no vias on the hop (between its end vias)
    (x0, y0), (x1, y1) = m(0.25), m(0.75)
    rh.rule_area(k.b, 'band uoff', (pcbnew.In2_Cu,), [(x0, y0 - 0.5), (x1, y1 - 0.5), (x1, y1 + 0.5), (x0, y0 + 0.5)],
                 keep_tracks=True, no_vias=True)


def strip(k):
    """strip power path, Kelvin and gate escapes (absolute coordinates; the right-hand parts follow
    J302's pin column JX and the board edge RIGHT)"""
    F, B = pcbnew.F_Cu, pcbnew.B_Cu
    JX = pad_at(k.b, 'J302', '1')[0]               # pin column; pads +-0.88
    pin14 = pad_at(k.b, 'J302', '14')[1]           # bottom pin (3V3)
    xt = pad_at(k.b, 'J201', '2')                  # XT60 VIN pin
    q3 = max(pad_at(k.b, 'Q203', n)[0] for n in '1234') + 0.7
    ur, vl = JX - 1.16, JX + 1.24                  # USENSE column right edge, VBUS column left edge
    top = pin14 + 0.95                             # below the last pin: the drain tabs' row
    vin = [(xt[0] - 2.6, 113.9), (xt[0] + 2.6, 113.9), (xt[0] + 2.6, 123.5), (154.6, 123.5), (154.6, 142.3), (158.6, 142.3),
           (158.6, 145.5), (154.4, 145.5), (154.4, 154.0), (150.3, 154.0), (150.3, 119.5), (xt[0] - 2.6, 119.5)]
    usense = [(155.6, 145.75), (ur, 145.75), (ur, top), (q3, top), (q3, 167.5), (151.4, 167.5), (151.4, top), (155.6, top)]
    vbus = [(vl, 137.2), (RIGHT - 0.3, 137.2), (RIGHT - 0.3, BOTTOM - 0.3), (150.3, BOTTOM - 0.3), (150.3, 168.0),
            (q3 + 0.4, 168.0), (q3 + 0.4, top), (vl, top)]
    k.zone('VIN', F, vin)
    k.zone('Input/USENSE', F, usense)
    k.zone('VBUS', F, vbus)
    # VBUS via field to In2: right of J302's pins (two columns) and along the bottom edge
    for i in range(21):
        y = Y0 + 38.0 + 1.4 * i
        for x in (vl + 1.0, vl + 3.0):
            if y < top - 0.4:
                k.via('VBUS', (x, y), 0.6, 0.3)
    gates = [pad_at(k.b, q, '4')[0] for q in ('Q202', 'Q203')]
    rs = [pcbnew.ToMM(k.b.FindFootprintByReference(r).GetPosition().x) for r in ('R210', 'R211')]   # gate resistors on B
    for x in (151.0 + 1.2 * j for j in range(int((RIGHT - 152.0) / 1.2))):
        if all(abs(x - g) > 0.9 for g in gates) and all(abs(x - r) > 1.6 for r in rs) and not (H4[0] - 2.4 < x < H4[0] + 2.4):
            k.via('VBUS', (x, BOTTOM - 1.0), 0.6, 0.3)
    # gates: pin 4 down to a pad via (krt-route goes on to R210 / R211 on B)
    for q, net in (('Q202', 'Net-(Q202-G)'), ('Q203', 'Net-(Q203-G)')):
        gx, gy = pad_at(k.b, q, '4')
        k.track(net, [(gx, gy), (gx, gy + 1.45)], 0.3, F)
        k.pad_via(net, (gx, gy + 1.45), 'GV' + q[1:])
    # keep KRT's tracks out of the pours (vias allowed: the hand ones are inside)
    rh_area(k.b, 'pour vin', vin)
    rh_area(k.b, 'pour usense', usense)
    rh_area(k.b, 'pour vbus', [(vl, 137.2), (RIGHT - 0.3, 137.2), (RIGHT - 0.3, 167.9), (q3 + 0.4, 167.9), (q3 + 0.4, top), (vl, top)])
    edges = [150.3] + [x for g in gates for x in (g - 0.6, g + 0.6)] + [RIGHT - 0.3]
    for j in range(0, len(edges), 2):
        rh_area(k.b, 'pour vbus %d' % j, [(edges[j], 168.0), (edges[j + 1], 168.0), (edges[j + 1], BOTTOM - 0.3), (edges[j], BOTTOM - 0.3)])
    # ---- Kelvin: U207 (B) SENSE pin 1 / VIN pin 2, U208 (B, below it) IN- pin 9 / IN+ pin 10, by vias into
    # the F pours next to the shunt pads (VIN column on the left, USENSE pour from R201 / R202's right pads)
    x1, y1 = pad_at(k.b, 'U207', '1')
    x2, y2 = pad_at(k.b, 'U207', '2')
    k.track('VIN', [(x2 - 0.75, y2), (150.6, y2), (150.6, y2 - 0.2)], 0.2, B)
    k.via('VIN', (150.6, y2 - 0.2))
    k.track('Input/USENSE', [(x1 - 0.75, y1), (x1 - 0.75, y1 + 1.2), (155.9, y1 + 1.2)], 0.2, B)   # under U207
    k.via('Input/USENSE', (155.9, y1 + 1.2))
    x8, y8 = pad_at(k.b, 'U208', '8')
    x9, y9 = pad_at(k.b, 'U208', '9')
    x10, y10 = pad_at(k.b, 'U208', '10')
    k.track('Input/USENSE', [(x9, y9), (x9 + 0.75, y9), (x9 + 1.35, y9 + 0.6)], 0.2, B)
    k.via('Input/USENSE', (x9 + 1.35, y9 + 0.6))
    k.track('VIN', [(x10, y10), (x10, y10 + 1.3), (153.5, y10 + 1.3)], 0.2, B)               # under U208
    k.via('VIN', (153.5, y10 + 1.3))
    # U208 VBUS (pin 8, boxed in by its neighbours) to a via just right of R202's USENSE pad (In2 plane)
    k.track('VBUS', [(x8, y8), (x8 + 2.1, y8)], 0.2, B)
    k.via('VBUS', (x8 + 2.3, y8))
    # U208 VS (pin 6) to the A1 strap (pin 1, +3V3 for address 0x44) under the body, decoupler C216 under pin 1
    x6, y6 = pad_at(k.b, 'U208', '6')
    x1, y1 = pad_at(k.b, 'U208', '1')
    k.track('+3V3', [(x6, y6), (x6 - 0.95, y6), (x1 + 0.95, y1), (x1, y1)], 0.2, B)
    cx, cy = pad_at(k.b, 'C216', '1')
    k.track('+3V3', [(x1, y1), (cx, y1 + 0.75), (cx, cy)], 0.2, B)
    # dividers: VIN pads (B) by vias into the VIN column
    for r in ('R233', 'R235'):
        x, y = pad_at(k.b, r, '1')
        k.track('VIN', [(x, y), (x, y + 1.3)], 0.3, B)
        k.via('VIN', (x, y + 1.3))
    # GND vias at the TVS and the bulk cap
    for at in ((159.0, 136.0), (159.0, 137.3), (159.6, 133.2), (161.0, 133.2)):
        k.via('GND', at, 0.6, 0.3)


def rh_area(board, name, pts):
    """F rule area: no tracks (vias and fills allowed)"""
    z = pcbnew.ZONE(board)
    z.SetIsRuleArea(True)
    z.SetZoneName(name)
    ls = pcbnew.LSET(); ls.AddLayer(pcbnew.F_Cu)
    z.SetLayerSet(ls)
    z.SetDoNotAllowTracks(True); z.SetDoNotAllowVias(False)
    z.SetDoNotAllowPads(False); z.SetDoNotAllowZoneFills(False); z.SetDoNotAllowFootprints(False)
    ol = z.Outline(); ol.NewOutline()
    for x, y in pts:
        ol.Append(rh.mm(x), rh.mm(y))
    board.Add(z)


def gnd_pours(board, group):
    ni = board.FindNet('GND')
    for lay in (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.B_Cu):
        z = pcbnew.ZONE(board)
        z.SetLayer(lay); z.SetNet(ni); z.SetAssignedPriority(0)
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        z.SetThermalReliefGap(rh.mm(0.25)); z.SetThermalReliefSpokeWidth(rh.mm(0.4))
        z.SetMinThickness(rh.mm(0.25)); z.SetLocalClearance(rh.mm(0.2))
        ol = z.Outline(); ol.NewOutline()
        for x, y in ((LEFT + 0.3, Y0 + 0.3), (RIGHT - 0.3, Y0 + 0.3), (RIGHT - 0.3, BOTTOM - 0.3), (LEFT + 0.3, BOTTOM - 0.3)):
            ol.Append(rh.mm(x), rh.mm(y))
        board.Add(z); group.AddItem(z)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for z in list(board.Zones()):
        if z.GetIsRuleArea() and z.GetZoneName().startswith(('bus ', 'pour ', 'band')):
            board.Delete(z)
    # KRT joins the VIN / USENSE / strip VBUS pads inside their F pours with stray stubs: the pours do that
    t = board.Tracks()
    stray = [t[i] for i in range(len(t)) if t[i].GetLayer() == pcbnew.F_Cu and t[i].GetParentGroup() is None
             and t[i].Type() == pcbnew.PCB_TRACE_T
             and (t[i].GetNetname() in ('VIN', '/Input/USENSE')
                  or (t[i].GetNetname() == 'VBUS' and pcbnew.ToMM(t[i].GetStart().x) >= SX))]
    for it in stray:
        board.Delete(it)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Delete(it)
            board.Delete(g)
    g = pcbnew.PCB_GROUP(board)
    g.SetName(GROUP)
    board.Add(g)
    rh.channel = lambda k: None                 # rev1 channel / trunk: replaced by band()
    for c in (1, 2):
        rh.column(Unit(board, g, c, 2))
    k = Abs(board, g)
    band(k)
    escapes(k)
    strip(k)
    gnd_pours(board, g)
    board.Save(dst)
    print('hand copper:', len(list(g.GetItems())), 'items')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
