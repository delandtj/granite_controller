#!/usr/bin/env python3
"""Scripted copper of the PP-48 core board, CAN FD link generation (group "hand", kept by krt-route).
Usage: route_core.py <in.kicad_pcb> <out.kicad_pcb>

docs/adr/0006-plane-link-can-fd.md decisions 2-5, docs/adr/0005-chained-plane-boards.md decision 6, ADR 0003
decision 14. Run after tools/place_core.py. Core-local (u, v) from the rev D frame's corner (52, 100); the
CAN FD board is 68 x 72 mm, u -8.5 .. 59.5.

- Link domain (LINK_GND pours on F / In1 / B): the rev D controller island's block top left (u -8.2-21.6,
  v 0-21), a strip along the whole top edge (v 0-9) and a channel down each edge to the link connectors
  (left u -8.2 .. -0.8, right u 49.6 .. 59.2, both to v 37.5). The ISO1044 U4 straddles the block's bottom
  edge, the coupled inductor L2 the strip's bottom edge at u 32 (tools/place_core.py).
- Isolation: a 2 mm rule area (no tracks, vias or fills, every layer) between the link domain and the bus
  world; under L2 the pads stand in it (1.48 mm between its pad rows) and a 'pocket' keeps both domains'
  fills 2 mm from the other side's L2 pads. No In2 fill under the link domain
  (In2 is the VIN plane, krt-route's plane pass).
- Fly-Buck switch node: U2 SW -> L2 pad 1 on F, 0.6 mm.
- GND pours F / In1 / B over the whole board, priority 0 (the LINK_GND pours win; the bands keep them
  apart).
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import route_hand as rh                  # noqa: E402  (track / via / pad_via helpers, rule_area)

CX, Y0, U0, U1, H = 52.0, 100.0, -8.5, 59.5, 72.0     # frame corner, board edges u U0 .. U1
GROUP = 'hand'
NAMES = ('iso', 'pocket', 'ext ', 'ctrl-no-in2', 'link-no-in2')   # this script's rule areas (redrawn every run)
# the link domain, and the 2 mm band outside it (outer edge, then back along the link domain's edge)
LINK = [(U0 + 0.3, 0.3), (U1 - 0.3, 0.3), (U1 - 0.3, 37.5), (49.6, 37.5), (49.6, 9.0), (21.6, 9.0), (21.6, 21.0),
        (-0.8, 21.0), (-0.8, 37.5), (U0 + 0.3, 37.5)]
BAND = [(U1, 37.5), (U1, 39.5), (47.6, 39.5), (47.6, 11.0), (23.6, 11.0), (23.6, 23.0), (1.2, 23.0), (1.2, 39.5),
        (U0, 39.5), (U0, 37.5), (-0.8, 37.5), (-0.8, 21.0), (21.6, 21.0), (21.6, 9.0), (49.6, 9.0), (49.6, 37.5)]


class Core(rh.Col):
    """core-local coordinates, nets by full name"""

    def __init__(self, board, group):
        self.b, self.g, self.c, self.n = board, group, 0, 0
        self.x0 = CX

    def net(self, name):
        ni = self.b.FindNet(name) or self.b.FindNet('/' + name)
        if ni is None:
            raise SystemExit('no net ' + name)
        return ni

    def pour(self, net, layer, pts, prio, thermal=True):
        z = pcbnew.ZONE(self.b)
        z.SetLayer(layer); z.SetNet(self.net(net)); z.SetAssignedPriority(prio)
        if thermal:
            z.SetPadConnection(pcbnew.ZONE_CONNECTION_THERMAL)
            z.SetThermalReliefGap(rh.mm(0.25)); z.SetThermalReliefSpokeWidth(rh.mm(0.4))
        else:
            z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)
        z.SetMinThickness(rh.mm(0.25)); z.SetLocalClearance(rh.mm(0.2))
        ol = z.Outline(); ol.NewOutline()
        for u, v in pts:
            ol.Append(rh.mm(CX + u), rh.mm(Y0 + v))
        self.b.Add(z); self.g.AddItem(z)


def box(u0, v0, u1, v1):
    return [(u0, v0), (u1, v0), (u1, v1), (u0, v1)]


def area(board, name, layers, pts, **kw):
    rh.rule_area(board, name, layers, [(CX + u, Y0 + v) for u, v in pts], **kw)


def island(k):
    F, IN1, IN2, B = pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu
    for lay in (F, IN1, B):
        k.pour('LINK_GND', lay, LINK, 1)
    area(k.b, 'iso', (F, IN1, IN2, B), BAND)
    area(k.b, 'link-no-in2', (IN2,), LINK, keep_tracks=True)
    # L2 stands in the band (1.48 mm between its pad rows): no fill of either domain within 2 mm of the
    # other domain's L2 pads
    area(k.b, 'pocket', (F, IN1, IN2, B), box(27.6, 8.7, 36.4, 11.3))


def flybuck(k):
    """the Fly-Buck's switch node: U2 SW (pin 8) straight up to L2 pad 1 on F, wide (krt-route takes R13 / C13
    on)"""
    a = pad_at(k.b, 'U2', '8')
    c = pad_at(k.b, 'L2', '1')
    k.track('/Supplies/SW2', [a, (a[0], c[1] + 0.7), c], 0.6, pcbnew.F_Cu)


def pad_at(board, ref, num):
    """core-local position of a pad"""
    for p in board.FindFootprintByReference(ref).Pads():
        if p.GetNumber() == num:
            return pcbnew.ToMM(p.GetPosition().x) - CX, pcbnew.ToMM(p.GetPosition().y) - Y0
    raise SystemExit('%s has no pad %s' % (ref, num))


def gnd_pours(k):
    for lay in (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.B_Cu):
        k.pour('GND', lay, box(U0 + 0.3, 0.3, U1 - 0.3, H - 0.3), 0)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for z in list(board.Zones()):
        if z.GetIsRuleArea() and z.GetZoneName().startswith(NAMES):
            board.Delete(z)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Delete(it)
            board.Delete(g)
    g = pcbnew.PCB_GROUP(board)
    g.SetName(GROUP)
    board.Add(g)
    k = Core(board, g)
    island(k)
    flybuck(k)
    gnd_pours(k)
    board.Save(dst)
    print('hand copper:', len(list(g.GetItems())), 'items')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
