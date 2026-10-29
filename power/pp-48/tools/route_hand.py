#!/usr/bin/env python3
"""Scripted copper of the PP-48 slots, units and core (group "hand", kept by krt-route).
Usage: route_hand.py <in.kicad_pcb> <out.kicad_pcb> [N]

docs/adr/0004-plane-column-layout.md. Run after tools/place.py; the same
copper is drawn in every slot column (column-local (u, v) in mm, slot c at
x = X0 + 25 (c - 1); slot A of unit u is c = 2u - 1, slot B c = 2u), only
the net names change (/Unit u/A_X or B_X):

- Power path: F VBUS pour over the shunt VBUS pads with six 0.6/0.3 vias to
  the In2 VBUS plane; F SENSE pour from the shunts' other pads into Q1's drain
  tab; F OUT pour from Q1's source pins to S1 (VIN), B OUT pour joining the
  clamp D1 and cap C2 to S1; gate: Q1 pin 4 -> via -> B -> via -> U1 GATE.
- INA238 Kelvin: IN+ (pin 10) and IN- (pin 9) to the bottom ends of R2's
  pads, 0.2 mm, under the shunts.
- OV latch U3: OVK (pin 7) and CLRN (pin 6) escape inward to vias under its
  body (KRT finds no way out between the +3V3 and LQ pins); arm FF U10:
  PGD (pin 6) the same.
- Node: F and B NODE pours from S4 (VOUT) to J1 pin 2.
Per unit, drawn from slot A's origin across both slots (50 mm):
- In2 channel (ADR 0004 decision 5), rows v = 59.85 + 0.3 k, 0.15 mm:
  the unit's own SDA / SCL on rows 0 / 1 from the core-side edge to the
  tap vias under the pull-ups R26 / R27 (slot A); rows 0-5 arriving from
  the right jog down two rows in slot A (staggered 0.3 mm, bottom row
  first) and leave on the left as rows 2-7. Row 2p / 2p + 1 on the right
  carries unit u + 1 + p; rows whose unit does not exist are not drawn.
- +3V3 trunk on In2 at v = 64.95 (0.4 mm) across the unit, tapped by a via
  to R26 / R27 pin 1 (krt-route takes +3V3 on to slot B).
- In2 bus: slot A's 11 expander lines from a via ladder at slot A's old
  expander spot straight to a ladder beside the expander in slot B (bus(),
  under the shunts' VBUS feed, so the plane only gets a slot);
  tools/netsplit.py splits these nets at the ladders for krt-route.
In slot B: expander U5 supply pins out to vias by their caps (expander()).
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
  ladder; at most 4 units = channels 0-3). The +3V3 trunk ends on a via at the
  core edge, F track up to L1.
- Isolation (ADR 0003 decision 14): CTRL_GND pours F / In1 / B over the
  controller world (top left, plus the strip under the 12 V module's
  outputs), a 2 mm rule area band (no tracks, vias or pours, all layers)
  around it, no In2 fill under it.
The In2 VBUS plane comes from krt-route's plane pass. Rule areas
"unit-edge n" (F / B, no tracks or vias, 0.4 mm wide) on every unit edge
(core / unit 1 and last unit / end strip included) keep the router inside each unit, so
tools/copycol.py can copy unit 1; inside a unit the router crosses between
the slots (expander lines to slot B, +3V3, SDA / SCL).
"""
import sys

import pcbnew

PITCH, UNIT, X0, Y0 = 25.0, 50.0, 100.0, 100.0
CX, END = X0 - 48.0, 8.0                # core left edge, end strip width (pcbkit.toml [outline])
GROUP = 'hand'
ROW0, ROW = 59.85, 0.3
JOG_R, JOG_STEP = 17.5, 0.3
TAP_SDA, TAP_SCL, TAP_V = 11.0, 12.6, 58.6
V3_V, V3_TAP = 64.95, 13.0
EX, EY = 11.8, 37.6                     # expander U5 in slot B (tools/place.py)
# In2 bus of slot A's expander lines: ladders at BUS_A (slot A) and BUS_B (slot B, = 25 + 1.6),
# both slot A-local; BUS_V below the OV latch escape vias (v 34.05)
BUS_A, BUS_B, BUS_V, BUS_DU, BUS_DV = 7.5, 26.6, 34.8, 0.6, 0.5
BUS = ('DISARM', 'OFF', 'PGD', 'LQ', 'LCLR', 'OVK', 'PG', 'RUNOFF', 'LED', 'PWRD', 'RSTD')   # slot A nets
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
        self.unit, self.slot = (c + 1) // 2, 'A' if c % 2 else 'B'
        self.x0 = X0 + PITCH * (c - 1)

    def net(self, name):
        if name in ('GND', 'VBUS', '+3V3'):
            full = name
        elif name.startswith('UNIT'):
            full = '/' + name                    # root sheet labels
        else:
            full = '/Unit %d/%s_%s' % (self.unit, self.slot, name)
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

    def pad_via(self, net, at, ref, size=0.5, drill=0.2):
        """a via as a one-pad board-only footprint (tented, not in BOM / CPL / parity):
        KRT routes to pads, not to a net island of vias and tracks"""
        fp = pcbnew.FOOTPRINT(self.b)
        fp.SetReference(ref); fp.Reference().SetVisible(False); fp.Value().SetVisible(False)
        fp.SetAttributes(pcbnew.FP_BOARD_ONLY | pcbnew.FP_EXCLUDE_FROM_BOM | pcbnew.FP_EXCLUDE_FROM_POS_FILES)
        pad = pcbnew.PAD(fp)
        pad.SetNumber('1'); pad.SetAttribute(pcbnew.PAD_ATTRIB_PTH)
        pad.SetShape(pcbnew.PAD_SHAPE_CIRCLE); pad.SetSize(pcbnew.VECTOR2I(mm(size), mm(size)))
        pad.SetDrillSize(pcbnew.VECTOR2I(mm(drill), mm(drill)))
        pad.SetLayerSet(pcbnew.LSET.AllCuMask())
        fp.Add(pad)
        self.b.Add(fp)
        fp.SetPosition(self.p(*at))
        pad.SetNet(self.net(net) if isinstance(net, str) else net)
        self.g.AddItem(fp)

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
    F, B = pcbnew.F_Cu, pcbnew.B_Cu
    # ---- power path
    k.zone('VBUS', F, [(7.95, 9.6), (10.3, 9.6), (10.3, 16.9), (7.95, 16.9)])
    for v in (10.3, 11.5, 12.7, 14.3, 15.5, 16.4):
        k.via('VBUS', (8.45, v), 0.6, 0.3)
    k.zone('SENSE', F, [(14.6, 9.6), (17.8, 9.6), (17.8, 11.2), (22.95, 11.2), (22.95, 16.6), (14.6, 16.6)])
    # KRT does not avoid other nets' pours: keep its tracks and vias out of the SENSE pour
    rule_area(k.b, 'pour %d' % k.c, (F,), [(k.x0 + u, Y0 + v) for u, v in
                                          ((14.8, 11.4), (22.75, 11.4), (22.75, 15.9), (14.8, 15.9))], fills=True)
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
    # ---- arm FF U10: PGD (pin 6) between ARMS_N and ARM, the same escape inward to a via
    k.track('PGD', [(4.2, 25.85), (3.2, 25.85), (2.9, 26.15)], 0.2, F)
    k.via('PGD', (2.9, 26.15))
    # ---- aux socket J3: GND pins 5 and 6 joined under the body (pad 6 can end up boxed in)
    k.track('GND', [(20.35, 41.425), (20.35, 45.375)], 0.3, F)
    # ---- node
    for lay in (F, B):
        k.zone('NODE', lay, [(16.3, 52.6), (24.6, 52.6), (24.6, 69.2), (16.3, 69.2)])
    if k.slot == 'A':
        channel(k)
        bus(k)
    else:
        expander(k)


def bus(k):
    """In2 bus of slot A's expander lines (unit expander U5 sits in slot B): line j from a
    pad via (BVu0j, see Col.pad_via) at (BUS_A + 0.6 j, BUS_V + 0.5 j) in slot A straight right to the same ladder in slot B
    (BUS_B, slot A-local); line j passes the other ladder vias 0.5 mm or more away"""
    for j, name in enumerate(BUS):
        v = BUS_V + BUS_DV * j
        a, b = BUS_A + BUS_DU * j, BUS_B + BUS_DU * j
        k.track(name, [(a, v), (b, v)], 0.2, pcbnew.In2_Cu)
        k.pad_via(name, (a, v), 'BV%d%02d' % (k.unit, j))
        k.pad_via(name, (b, v), 'BV%d%02d' % (k.unit, 50 + j))
    # KRT does not see In2 tracks when it places vias: no vias through the bus band
    n = len(BUS) - 1
    band = [(BUS_A - 0.5, BUS_V - 0.4), (BUS_B + BUS_DU * n + 0.5, BUS_V - 0.4),
            (BUS_B + BUS_DU * n + 0.5, BUS_V + BUS_DV * n + 0.4), (BUS_A - 0.5, BUS_V + BUS_DV * n + 0.4)]
    rule_area(k.b, 'bus %d' % k.unit, (pcbnew.In2_Cu,), [(k.x0 + u, Y0 + v) for u, v in band], keep_tracks=True, no_vias=True)


def expander(k):
    """TCA6424A U5 (slot B, centre EX, EY) supply pins: VCCI (31) and VCCP (27) sit on the
    top row between SDA, SCL and /RESET; each goes outward to a via by its cap (C13 / C9),
    so the middle pins escape upward between the two vias"""
    F = pcbnew.F_Cu
    top = EY - 2.44
    k.track('+3V3', [(EX - 1.25, top), (EX - 1.25, EY - 3.3), (EX - 2.0, EY - 4.05)], 0.2, F)
    k.pad_via('+3V3', (EX - 2.0, EY - 4.05), 'PV%d%s3' % (k.unit, k.slot))
    k.track('+3V3', [(EX - 2.0, EY - 4.05), (EX - 2.59, EY - 3.9)], 0.25, F)
    k.track('+3V3', [(EX + 0.75, top), (EX + 0.75, EY - 3.3), (EX + 1.6, EY - 4.15)], 0.2, F)
    k.pad_via('+3V3', (EX + 1.6, EY - 4.15), 'PV%d%s4' % (k.unit, k.slot))
    k.track('+3V3', [(EX + 1.6, EY - 4.15), (EX + 1.49, EY - 5.3)], 0.25, F)
    for du in (-0.75, 0.75):                    # exposed pad: GND vias (its F island has no room for one)
        for dv in (-0.75, 0.75):
            k.via('GND', (EX + du, EY + dv))


def channel(k):
    """In2 channel and +3V3 trunk of one unit, from slot A's origin across both slots"""
    F, IN2 = pcbnew.F_Cu, pcbnew.In2_Cu
    un, units = k.unit, k.n // 2
    left = 0.5 if un == 1 else 0.0
    right = UNIT
    row = lambda i: ROW0 + ROW * i
    k.track('UNIT%d_SDA' % un, [(left, row(0)), (TAP_SDA, row(0)), (TAP_SDA, TAP_V)], 0.15, IN2)
    k.track('UNIT%d_SCL' % un, [(left, row(1)), (TAP_SCL, row(1)), (TAP_SCL, TAP_V)], 0.15, IN2)
    for name, u in (('UNIT%d_SDA' % un, TAP_SDA), ('UNIT%d_SCL' % un, TAP_SCL)):
        k.via(name, (u, TAP_V))
        k.track(name, [(u, TAP_V), (u, 57.51)], 0.2, F)
    for s in range(6):
        far = un + 1 + s // 2
        if far > units:
            continue
        net = 'UNIT%d_%s' % (far, 'SDA' if s % 2 == 0 else 'SCL')
        u0 = JOG_R - (5 - s) * JOG_STEP
        k.track(net, [(right, row(s)), (u0, row(s)), (u0 - 2 * ROW, row(s + 2)), (left, row(s + 2))], 0.15, IN2)
    # ---- +3V3 trunk and tap
    k.track('+3V3', [(left, V3_V), (right if un < units else V3_TAP, V3_V)], 0.4, IN2)   # last unit: ends at its tap
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
    # ---- In2 channel: row i (0 = UNIT1_SDA) from unit 1's edge (u 48.5), down at the core edge
    # (lowest row first), left under the FETs' VBUS via field, up the ladder to its via
    rows = n                                    # 2 per unit, at most 8 (channels 0-3)
    for i in range(rows):
        r = ROW0 + ROW * i
        a = DIP_R - ROW * (rows - 1 - i)
        lo = DIP_V + ROW * i
        lu, lv = LAD_U - LAD_DU * i, LAD_V - LAD_DV * i
        net = 'UNIT%d_%s' % (i // 2 + 1, 'SDA' if i % 2 == 0 else 'SCL')
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


def rule_area(board, name, layers, pts, keep_tracks=False, no_vias=False, fills=False):
    """rule area: no zone fills (unless fills); no tracks or vias either unless keep_tracks
    (no_vias: still no vias)"""
    z = pcbnew.ZONE(board)
    z.SetIsRuleArea(True)
    z.SetZoneName(name)
    ls = pcbnew.LSET()
    for lay in layers:
        ls.AddLayer(lay)
    z.SetLayerSet(ls)
    z.SetDoNotAllowTracks(not keep_tracks); z.SetDoNotAllowVias(no_vias or not keep_tracks)
    z.SetDoNotAllowPads(False); z.SetDoNotAllowZoneFills(not fills); z.SetDoNotAllowFootprints(False)
    ol = z.Outline(); ol.NewOutline()
    for px, py in pts:
        ol.Append(mm(px), mm(py))
    board.Add(z)


def fences(board, n):
    """no tracks or vias on F / B along every unit edge, so the router keeps each
    unit's copper inside the unit (the In2 channel and planes cross freely)"""
    for z in list(board.Zones()):
        if z.GetIsRuleArea() and z.GetZoneName().startswith(('column-edge', 'unit-edge', 'isolation', 'ctrl-no-in2', 'bus ', 'pour ')):
            board.Delete(z)
    for e in range(0, n // 2 + 1):              # e = 0: core / unit 1; the last: unit / end strip
        x = X0 + UNIT * e
        z = pcbnew.ZONE(board)
        z.SetIsRuleArea(True)
        z.SetZoneName('unit-edge %d' % e)
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
    print('hand copper:', len(list(g.GetItems())), 'items in', n, 'slots')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 2)
