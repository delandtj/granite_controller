#!/usr/bin/env python3
"""Scripted copper of the PP-48 core board (group "hand", kept by krt-route).
Usage: route_core.py <in.kicad_pcb> <out.kicad_pcb>

docs/adr/0005-chained-plane-boards.md decision 6, ADR 0003 decision 14. Run after tools/place_core.py.
Core-local (u, v) from the board's top-left corner (52, 100), 56 x 72 mm.

- Controller island (CTRL_GND): the block top left (u 0-21.6, v 0-21) and the strip along the top edge
  (u 21.6-31, v 0-9), CTRL_GND pours on F / In1 / B. The isolators U4 / U5 straddle the block's bottom
  edge, the coupled inductor L2 the strip's right edge (tools/place_core.py: one winding per side).
- Isolation: a 2 mm rule area (no tracks, vias or fills, every layer) between the island and the bus world;
  under L2 it narrows to the 1.48 mm between its pad columns. No In2 fill under the island (In2 is the VIN
  plane, krt-route's plane pass).
- Fly-Buck switch node: U2 SW -> L2 pad 1 on F, 0.6 mm (the router left it a 0.2 mm detour through vias).
- GND pours F / In1 / B over the whole board, priority 0 (the CTRL_GND pours win; the bands keep them
  apart).
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import route_hand as rh                  # noqa: E402  (track / via / pad_via helpers, rule_area)

CX, Y0, W, H = 52.0, 100.0, 56.0, 72.0
GROUP = 'hand'
NAMES = ('iso', 'pocket', 'ext ', 'ctrl-no-in2')     # this script's rule areas (redrawn on every run)
# the island, and the 2 mm band outside it (outer edge, then back along the island edge)
CTRL = [(0.3, 0.3), (31.0, 0.3), (31.0, 9.0), (21.6, 9.0), (21.6, 21.0), (0.3, 21.0)]
BAND = [(31.0, 0.0), (33.0, 0.0), (33.0, 11.0), (23.6, 11.0), (23.6, 23.0), (0.0, 23.0), (0.0, 21.0),
        (21.6, 21.0), (21.6, 9.0), (31.0, 9.0)]


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
        k.pour('CTRL_GND', lay, CTRL, 1)
    area(k.b, 'iso', (F, IN1, IN2, B), BAND)
    area(k.b, 'ctrl-no-in2', (IN2,), CTRL, keep_tracks=True)


def flybuck(k):
    """the Fly-Buck's switch node: U2 SW (pin 8) straight to L2 pad 1 on F, wide (krt-route takes R13 / C13 on)"""
    a = pad_at(k.b, 'U2', '8')
    c = pad_at(k.b, 'L2', '1')
    k.track('/Supplies/SW2', [a, (a[0] - 1.4, a[1]), c], 0.6, pcbnew.F_Cu)


def pad_at(board, ref, num):
    """core-local position of a pad"""
    for p in board.FindFootprintByReference(ref).Pads():
        if p.GetNumber() == num:
            return pcbnew.ToMM(p.GetPosition().x) - CX, pcbnew.ToMM(p.GetPosition().y) - Y0
    raise SystemExit('%s has no pad %s' % (ref, num))


def gnd_pours(k):
    for lay in (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.B_Cu):
        k.pour('GND', lay, box(0.3, 0.3, W - 0.3, H - 0.3), 0)


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
