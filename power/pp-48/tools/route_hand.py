#!/usr/bin/env python3
"""Scripted copper of the PP-48 columns (group "hand", kept by krt-route).
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
The In2 VBUS plane comes from krt-route's plane pass. Rule areas
"column-edge n" (F / B, no tracks or vias, 0.4 mm wide) on every inner column
edge keep the router inside each column, so tools/copycol.py can copy column 1.
"""
import sys

import pcbnew

PITCH, X0, Y0 = 25.0, 100.0, 100.0
GROUP = 'hand'
ROW0, ROW = 59.85, 0.3
JOG_R, JOG_STEP = 17.5, 0.3
TAP_SDA, TAP_SCL, TAP_V = 11.0, 12.6, 58.6
V3_V, V3_TAP = 64.95, 13.0


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


def fences(board, n):
    """no tracks or vias on F / B along every inner column edge, so the router keeps
    each column's copper inside the column (the In2 channel and planes cross freely)"""
    for z in list(board.Zones()):
        if z.GetIsRuleArea() and z.GetZoneName().startswith('column-edge'):
            board.Delete(z)
    for c in range(1, n):
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
    """GND on F, In1 and B over the whole board, priority 0 (the power pours win).
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
        for px, py in ((X0 + 0.3, Y0 + 0.3), (X0 + PITCH * n - 0.3, Y0 + 0.3),
                       (X0 + PITCH * n - 0.3, Y0 + 71.7), (X0 + 0.3, Y0 + 71.7)):
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
    gnd_pours(board, g, n)
    board.Save(dst)
    print('hand copper:', len(list(g.GetItems())), 'items in', n, 'columns')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 2)
