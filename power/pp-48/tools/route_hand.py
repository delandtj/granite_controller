#!/usr/bin/env python3
"""Scripted copper of the PP-48 columns and core (group "hand", kept by krt-route).
Usage: route_hand.py <in.kicad_pcb> <out.kicad_pcb> [N]

docs/adr/0004-plane-column-layout.md. Run after tools/place.py; the same
copper is drawn in every column (column-local (u, v) in mm, column c at
x = X0 + 25 (c - 1)), only the net names change per column:

- Power path: F VBUS pour over the shunt VBUS pads with six 0.6/0.3 vias to
  the In2 VBUS plane; F SENSE pour from the shunts' other pads into Q1's drain
  tab; F OUT pour from Q1's source pins to S1 (VIN), B OUT pour joining the
  clamp D1 and cap C2 to S1; gate: Q1 pin 4 -> via -> B -> via -> U1 GATE.
- INA238 Kelvin: IN+ (pin 10) and IN- (pin 9) to the bottom ends of R2's
  pads, 0.2 mm, under the shunts.
- OV latch U3: OVK (pin 7) and CLRN (pin 6) escape inward to vias under its
  body (KRT finds no way out between the +3V3 and LQ pins).
- Node: F and B NODE pours from S4 (VOUT) to J1 pin 2.
- In2 channel (ADR 0004 decision 5), rows v = 59.85 + 0.3 k, 0.15 mm:
  the column's own SDA / SCL on rows 0 / 1 from the core-side edge to the
  tap vias under the pull-ups R26 / R27; rows 0-13 arriving from the right
  jog down two rows (staggered 0.3 mm, bottom row first) and leave on the
  left as rows 2-15. Row 2p / 2p + 1 on the right carries column c + 1 + p;
  rows whose column does not exist on this plane are not drawn.
- +3V3 trunk on In2 at v = 64.95 (0.4 mm) across the column, tapped by a via
  to R26 / R27 pin 1.
- GND pours on F, In1 and B over the whole board (priority 0).
Core block (tools/core.py, core-local (u, v) from x = X0 - 48):
- Tile power path on B: XT90 VIN pad -> TVS D1 / C2 and the shunts R1 / R2
  as wide tracks (VIN owns no zone, so KRT still routes its small pads),
  F track to the bulk C1; TSENSE pour shunts -> FET drain tabs; VBUS pour
  under the FET sources with 30 vias to In2. Gate resistors R10 / R11 at the
  gate pins, R11's GATE side by a via to F.
- Kelvin: LM5069 U7 SENSE / VIN and INA238 U8 IN- / IN+ to the shunt pads
  (U7 SENSE hops over F); U7 / U8 VBUS pins and C14 / C15 by vias to In2.
- In2 channel: the rows from column 1 dip at the core edge under the VBUS
  via field (so that field feeds the In2 plane above the rows, which reaches
  the columns), run left along the bottom and end on a via ladder by the mux
  U6; F tracks ladder -> mux pins (N <= 4: channels 0-3, the side facing the
  ladder). KRT leaves the COLn nets alone (pcbkit.toml route_extra). The +3V3 trunk ends on a via at the
  core edge, F track up to L1.
- Isolation (ADR 0003 decision 14): CTRL_GND pours F / In1 / B over the
  controller world (top left, plus the strip under the 12 V module's
  outputs), a 2 mm rule area band (no tracks, vias or pours, all layers)
  around it, no In2 fill under it.
The In2 VBUS plane comes from krt-route's plane pass. Rule areas
"column-edge n" (F / B, no tracks or vias, 0.4 mm wide) on every inner column
edge keep the router inside each column, so tools/copycol.py can copy column 1.
"""
import sys

import pcbnew

PITCH, X0, Y0 = 25.0, 100.0, 100.0
CX, END = X0 - 48.0, 8.0                # core left edge, end strip width (pcbkit.toml [outline])
GROUP = 'hand'
ROW0, ROW = 59.85, 0.3
JOG_R, JOG_STEP = 17.5, 0.3
TAP_SDA, TAP_SCL, TAP_V = 11.0, 12.6, 58.6
V3_V, V3_TAP = 64.95, 13.0
# core channel: dip verticals (lowest row at DIP_R), bottom run from DIP_V, ladder from (LAD_U, LAD_V)
DIP_R, DIP_V, CH = 46.7, 67.1, 0.3
LAD_U, LAD_V, LAD_DU, LAD_DV = 27.0, 66.4, 0.55, 0.45
V3_VIA = 47.3
MUX_D, MUX_X, MUX_STEP = 67.1, 19.6, 0.5       # ladder -> mux on F: bottom run, verticals by U6
# controller world and the 2 mm isolation band around it (core-local)
CTRL = [(0.3, 0.3), (46.0, 0.3), (46.0, 6.0), (21.6, 6.0), (21.6, 21.0), (0.3, 21.0)]
BAND = [(0.0, 21.0), (21.6, 21.0), (21.6, 6.0), (46.0, 6.0), (46.0, 0.0), (48.0, 0.0),
        (48.0, 8.0), (23.6, 8.0), (23.6, 23.0), (0.0, 23.0)]


def mm(v):
    return pcbnew.FromMM(v)


class Col:
    def __init__(self, board, group, c, n):
        self.b, self.g, self.c, self.n = board, group, c, n
        self.x0 = X0 + PITCH * (c - 1)

    def net(self, name):
        if name in ('GND', 'VBUS', '+3V3'):
            full = name
        elif name.startswith('COL'):
            full = '/' + name                    # root sheet labels
        else:
            full = '/Column %d/%s' % (self.c, name)
        ni = self.b.FindNet(full)
        if ni is None:
            raise SystemExit('no net ' + full)
        return ni

    def p(self, u, v):
        return pcbnew.VECTOR2I(mm(self.x0 + u), mm(Y0 + v))

    def track(self, net, pts, w, layer):
        ni = self.net(net) if isinstance(net, str) else net
        for a, b in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(self.b)
            t.SetStart(self.p(*a)); t.SetEnd(self.p(*b))
            t.SetWidth(mm(w)); t.SetLayer(layer); t.SetNet(ni)
            self.b.Add(t); self.g.AddItem(t)

    def via(self, net, at, size=0.5, drill=0.2):
        v = pcbnew.PCB_VIA(self.b)
        v.SetPosition(self.p(*at)); v.SetWidth(mm(size)); v.SetDrill(mm(drill))
        v.SetNet(self.net(net) if isinstance(net, str) else net)
        self.b.Add(v); self.g.AddItem(v)

    def zone(self, net, layer, pts, prio=3):
        z = pcbnew.ZONE(self.b)
        z.SetLayer(layer); z.SetNet(self.net(net)); z.SetAssignedPriority(prio)
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)
        z.SetMinThickness(mm(0.25)); z.SetLocalClearance(mm(0.2))
        ol = z.Outline(); ol.NewOutline()
        for u, v in pts:
            ol.Append(mm(self.x0 + u), mm(Y0 + v))
        self.b.Add(z); self.g.AddItem(z)


def column(k):
    F, B, IN2 = pcbnew.F_Cu, pcbnew.B_Cu, pcbnew.In2_Cu
    c = k.c
    # ---- power path
    k.zone('VBUS', F, [(7.95, 9.6), (10.3, 9.6), (10.3, 16.9), (7.95, 16.9)])
    for v in (10.3, 11.5, 12.7, 14.3, 15.5, 16.4):
        k.via('VBUS', (8.45, v), 0.6, 0.3)
    k.zone('SENSE', F, [(14.6, 9.6), (17.8, 9.6), (17.8, 11.2), (22.95, 11.2), (22.95, 16.6), (14.6, 16.6)])
    k.zone('OUT', F, [(17.0, 0.6), (24.6, 0.6), (24.6, 10.8), (19.35, 10.8), (19.35, 8.6), (17.0, 8.6)])
    k.zone('OUT', B, [(13.8, 1.0), (24.6, 1.0), (24.6, 8.3), (13.8, 8.3)])
    k.track('GATE', [(18.59, 10.1), (17.6, 9.1)], 0.3, F)
    k.via('GATE', (17.6, 9.1))
    k.track('GATE', [(17.6, 9.1), (17.6, 11.0), (23.3, 16.7), (23.3, 17.9)], 0.3, B)
    k.via('GATE', (23.3, 17.9))
    k.track('GATE', [(23.3, 17.9), (22.7, 18.5), (21.6, 18.5)], 0.3, F)
    # ---- INA238 Kelvin to R2's pad ends
    k.track('VBUS', [(5.5, 15.15), (5.5, 17.6), (9.54, 17.6), (9.54, 16.9)], 0.2, F)
    k.track('SENSE', [(5.0, 15.15), (5.0, 18.1), (15.46, 18.1), (15.46, 16.9)], 0.2, F)
    # ---- OV latch U3: OVK (pin 7) and CLRN (pin 6) sit between the +3V3 and LQ pins; escape
    # inward under the body to vias (B behind U3 is kept free), the router takes them from there
    k.track('OVK', [(19.4, 33.25), (18.4, 33.25), (18.1, 32.95)], 0.2, F)
    k.via('OVK', (18.1, 32.95))
    k.track('CLRN', [(19.4, 33.75), (18.4, 33.75), (18.1, 34.05)], 0.2, F)
    k.via('CLRN', (18.1, 34.05))
    # ---- node
    for lay in (F, B):
        k.zone('NODE', lay, [(16.3, 52.6), (24.6, 52.6), (24.6, 69.2), (16.3, 69.2)])
    # ---- In2 channel
    left = 0.5 if c == 1 else 0.0
    right = 25.0
    row = lambda i: ROW0 + ROW * i
    k.track('COL%d_SDA' % c, [(left, row(0)), (TAP_SDA, row(0)), (TAP_SDA, TAP_V)], 0.15, IN2)
    k.track('COL%d_SCL' % c, [(left, row(1)), (TAP_SCL, row(1)), (TAP_SCL, TAP_V)], 0.15, IN2)
    for name, u in (('COL%d_SDA' % c, TAP_SDA), ('COL%d_SCL' % c, TAP_SCL)):
        k.via(name, (u, TAP_V))
        k.track(name, [(u, TAP_V), (u, 57.51)], 0.2, F)
    for s in range(14):
        col = c + 1 + s // 2
        if col > k.n:
            continue
        net = 'COL%d_%s' % (col, 'SDA' if s % 2 == 0 else 'SCL')
        u0 = JOG_R - (13 - s) * JOG_STEP
        k.track(net, [(right, row(s)), (u0, row(s)), (u0 - 2 * ROW, row(s + 2)), (left, row(s + 2))], 0.15, IN2)
    # ---- +3V3 trunk and tap
    k.track('+3V3', [(left, V3_V), (right if c < k.n else V3_TAP, V3_V)], 0.4, IN2)   # last column: ends at its tap
    k.via('+3V3', (V3_TAP, V3_V))
    k.track('+3V3', [(V3_TAP, V3_V), (13.9, 64.05), (13.9, 56.49), (12.6, 56.49), (11.0, 56.49)], 0.3, F)


class Core(Col):
    def __init__(self, board, group):
        self.b, self.g, self.c, self.n = board, group, 0, 0
        self.x0 = CX

    def net(self, name):
        ni = self.b.FindNet(name) or self.b.FindNet('/' + name)
        if ni is None:
            raise SystemExit('no net ' + name)
        return ni


def core(k, n):
    F, B, IN1, IN2 = pcbnew.F_Cu, pcbnew.B_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu
    # ---- tile power path: XT90 VIN pad -> D1, C2, shunts (B), C1 (F)
    k.track('VIN', [(31.15, 48.0), (31.15, 51.09)], 3.0, B)
    k.track('VIN', [(33.0, 48.5), (35.6, 51.0)], 2.0, B)
    k.track('VIN', [(33.1, 44.5), (33.1, 41.5)], 2.0, B)
    k.track('VIN', [(29.3, 44.5), (28.4, 42.07)], 1.0, B)
    k.track('VIN', [(31.1, 47.0), (21.0, 47.0), (18.6, 49.4)], 1.5, F)
    k.zone('Input/TSENSE', B, [(29.45, 56.3), (40.2, 56.3), (40.2, 62.9), (29.45, 62.9)])
    k.zone('VBUS', B, [(30.45, 63.85), (40.6, 63.85), (40.6, 57.2), (41.95, 57.2), (41.95, 66.85), (30.45, 66.85)])
    for i in range(11):                         # two staggered rows, R11 and the GATE via between the FETs
        if not 33.5 < 30.95 + i < 36.5:
            k.via('VBUS', (30.95 + i, 65.6), 0.6, 0.3)
        if not 33.0 < 31.45 + i < 37.0:
            k.via('VBUS', (31.45 + i, 66.45), 0.6, 0.3)
    for i in range(8):
        k.via('VBUS', (41.2, 57.7 + i), 0.6, 0.3)
    # ---- gates: R10 left of Q2, R11 under Q3's gate pin with its GATE side on a via to F
    k.track('Net-(Q2-G)', [(28.1, 63.91), (28.1, 64.1), (28.5, 64.5), (29.69, 64.5)], 0.3, B)
    k.track('Net-(Q3-G)', [(35.86, 65.85), (36.09, 65.6), (36.09, 64.5)], 0.3, B)
    k.track('Input/GATE', [(34.84, 65.85), (34.0, 65.85)], 0.3, B)
    k.via('Input/GATE', (34.0, 65.85))
    # ---- LM5069 U7 Kelvin to R1: SENSE over F, VIN on B; GND pin to a via under the body
    k.track('Input/TSENSE', [(27.6, 52.5), (28.5, 52.5), (28.95, 52.05)], 0.2, B)
    k.via('Input/TSENSE', (28.95, 52.05))
    k.track('Input/TSENSE', [(28.95, 52.05), (28.95, 57.0)], 0.2, F)
    k.via('Input/TSENSE', (28.95, 57.0))
    k.track('Input/TSENSE', [(28.95, 57.0), (29.8, 57.0)], 0.25, B)
    k.track('VIN', [(27.6, 53.0), (29.4, 53.0), (29.9, 52.5), (29.9, 51.4)], 0.2, B)
    k.track('GND', [(26.85, 54.5), (26.2, 54.5)], 0.2, B)
    k.via('GND', (26.2, 54.5))
    # U7 OUT (VBUS) and C15 share a via to In2; C14 has its own
    k.track('VBUS', [(22.65, 53.0), (21.9, 53.0)], 0.2, B)
    k.via('VBUS', (21.9, 53.0))
    k.track('VBUS', [(21.9, 53.0), (21.33, 52.43), (21.33, 51.5)], 0.3, F)
    k.track('VBUS', [(21.33, 43.5), (21.33, 42.3)], 0.3, F)
    k.via('VBUS', (21.33, 42.3))
    # ---- INA238 U8 Kelvin to R2 (IN+ pin 10, IN- pin 9), VBUS pin to a via, GND pin 7 under the body to pin 2
    k.track('VIN', [(40.25, 51.6), (37.5, 51.6)], 0.2, B)
    k.track('Input/TSENSE', [(40.25, 52.1), (38.6, 52.1), (38.6, 56.6), (38.2, 57.0), (37.6, 57.0)], 0.2, B)
    k.track('VBUS', [(39.53, 52.6), (39.2, 52.6)], 0.2, B)
    k.via('VBUS', (39.2, 52.6))
    k.track('GND', [(40.97, 53.1), (41.6, 53.1), (42.6, 52.1), (43.83, 52.1)], 0.2, B)
    k.track('+3V3', [(40.25, 53.6), (40.52, 53.9), (40.52, 55.4)], 0.3, B)
    # ---- In2 channel: row i (0 = COL1_SDA) from column 1's edge (u 48.5), down at the core edge
    # (lowest row first), left under the FETs' VBUS via field, up the ladder to its via
    rows = 2 * n
    if n > 4:
        raise SystemExit('core channel: N > 4 needs the mux pins on the far side (U6 pins 13-20), not drawn')
    for i in range(rows):
        r = ROW0 + ROW * i
        a = DIP_R - ROW * (rows - 1 - i)
        lo = DIP_V + ROW * i
        lu, lv = LAD_U - LAD_DU * i, LAD_V - LAD_DV * i
        net = 'COL%d_%s' % (i // 2 + 1, 'SDA' if i % 2 == 0 else 'SCL')
        k.track(net, [(48.5, r), (a + CH, r), (a, r + CH), (a, lo - CH), (a - CH, lo),
                      (lu + CH, lo), (lu, lo - CH), (lu, lv)], 0.15, IN2)
        k.via(net, (lu, lv))
        # F: ladder via down-left, left under the ladder (row 0 lowest), up the right side of the mux
        # (rot 180: channel pins 4.. top-down = the highest row first) into its pin
        pin = [p for p in k.b.FindFootprintByReference('U6').Pads() if p.GetNumber() == str(4 + i)][0]
        pu, pv = pcbnew.ToMM(pin.GetPosition().x) - CX, pcbnew.ToMM(pin.GetPosition().y) - Y0
        d = MUX_D + MUX_STEP * (rows - 1 - i)
        x = MUX_X + MUX_STEP * i
        k.track(net, [(lu, lv), (lu + lv - d, d), (x + CH, d), (x, d - CH), (x, pv + CH), (x - CH, pv), (pu, pv)], 0.2, F)
    # ---- +3V3 trunk end: via at the core edge, F up to L1's output pad
    k.track('+3V3', [(48.5, V3_V), (V3_VIA, V3_V)], 0.4, IN2)
    k.via('+3V3', (V3_VIA, V3_V))
    k.track('+3V3', [(V3_VIA, V3_V), (V3_VIA, 38.3), (46.6, 37.6)], 0.4, F)
    # ---- controller world
    for lay in (F, IN1, B):
        k.zone('CTRL_GND', lay, CTRL, prio=1)
    rule_area(k.b, 'isolation', (F, IN1, IN2, B), [(CX + u, Y0 + v) for u, v in BAND])
    rule_area(k.b, 'ctrl-no-in2', (IN2,), [(CX + u, Y0 + v) for u, v in CTRL], keep_tracks=True)


def rule_area(board, name, layers, pts, keep_tracks=False):
    """rule area: no zone fills; no tracks or vias either unless keep_tracks"""
    z = pcbnew.ZONE(board)
    z.SetIsRuleArea(True)
    z.SetZoneName(name)
    ls = pcbnew.LSET()
    for lay in layers:
        ls.AddLayer(lay)
    z.SetLayerSet(ls)
    z.SetDoNotAllowTracks(not keep_tracks); z.SetDoNotAllowVias(not keep_tracks)
    z.SetDoNotAllowPads(False); z.SetDoNotAllowZoneFills(True); z.SetDoNotAllowFootprints(False)
    ol = z.Outline(); ol.NewOutline()
    for px, py in pts:
        ol.Append(mm(px), mm(py))
    board.Add(z)


def fences(board, n):
    """no tracks or vias on F / B along every inner column edge, so the router keeps
    each column's copper inside the column (the In2 channel and planes cross freely)"""
    for z in list(board.Zones()):
        if z.GetIsRuleArea() and z.GetZoneName().startswith(('column-edge', 'isolation', 'ctrl-no-in2')):
            board.Delete(z)
    for c in range(0, n):                       # c = 0: core / column 1
        x = X0 + PITCH * c
        z = pcbnew.ZONE(board)
        z.SetIsRuleArea(True)
        z.SetZoneName('column-edge %d' % c)
        ls = pcbnew.LSET(); ls.AddLayer(pcbnew.F_Cu); ls.AddLayer(pcbnew.B_Cu)
        z.SetLayerSet(ls)
        z.SetDoNotAllowTracks(True); z.SetDoNotAllowVias(True)
        z.SetDoNotAllowPads(False); z.SetDoNotAllowZoneFills(False); z.SetDoNotAllowFootprints(False)
        ol = z.Outline(); ol.NewOutline()
        for px, py in ((x - 0.2, Y0), (x + 0.2, Y0), (x + 0.2, Y0 + 72.0), (x - 0.2, Y0 + 72.0)):
            ol.Append(mm(px), mm(py))
        board.Add(z)


def gnd_pours(board, group, n):
    """GND on F, In1 and B over the whole board except the controller world and its
    isolation band (core top left), priority 0 (the power pours win).
    In the kept group, so krt-route treats GND as a plane net: pads land in the
    pours (thermal spokes; the standoffs and J1 are solid, tools/place.py), no
    GND tracks between pads."""
    ni = board.FindNet('GND')
    for lay in (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.B_Cu):
        z = pcbnew.ZONE(board)
        z.SetLayer(lay); z.SetNet(ni); z.SetAssignedPriority(0)
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
        z.SetThermalReliefGap(mm(0.25)); z.SetThermalReliefSpokeWidth(mm(0.4))
        z.SetMinThickness(mm(0.25)); z.SetLocalClearance(mm(0.2))
        ol = z.Outline(); ol.NewOutline()
        xe = X0 + PITCH * n + END - 0.3
        for px, py in ((CX + 0.3, Y0 + 23.0), (CX + 23.6, Y0 + 23.0), (CX + 23.6, Y0 + 8.0), (X0, Y0 + 8.0),
                       (X0, Y0 + 0.3), (xe, Y0 + 0.3), (xe, Y0 + 71.7), (CX + 0.3, Y0 + 71.7)):
            ol.Append(mm(px), mm(py))
        board.Add(z); group.AddItem(z)


def main(src, dst, n):
    board = pcbnew.LoadBoard(src)
    fences(board, n)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Delete(it)
            board.Delete(g)
    g = pcbnew.PCB_GROUP(board)
    g.SetName(GROUP)
    board.Add(g)
    for c in range(1, n + 1):
        column(Col(board, g, c, n))
    core(Core(board, g), n)
    gnd_pours(board, g, n)
    board.Save(dst)
    print('hand copper:', len(list(g.GetItems())), 'items in', n, 'columns')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 2)
