#!/usr/bin/env python3
"""Put the hand-routed copper (tools/route_drive.py) into group "hand", which
[krt] keep_groups protects from krt-route's strip.
Usage: group_hand.py <board.kicad_pcb>   (in place)
"""
import sys

import pcbnew

NETS = {'/TG1', '/TG2', '/BG1', '/BG2', '/BOOST1', '/BOOST2', '/SW1', '/SW2', '/DRVCC'}
EP_VIAS = {(115.75, y) for y in (114.9, 116.1, 117.3)}


def main(path):
    b = pcbnew.LoadBoard(path)
    power = set()
    for g in list(b.Groups()):
        if g.GetName() == 'power':
            power = {it.m_Uuid.AsString() for it in g.GetItems()}
        if g.GetName() == 'hand':
            b.Remove(g)
    grp = pcbnew.PCB_GROUP(b)
    grp.SetName('hand')
    b.Add(grp)
    n = 0
    for t in b.GetTracks():
        if t.m_Uuid.AsString() in power:
            continue
        xy = (round(pcbnew.ToMM(t.GetPosition().x), 2), round(pcbnew.ToMM(t.GetPosition().y), 2))
        if t.GetNetname() in NETS or (t.Type() == pcbnew.PCB_VIA_T and xy in EP_VIAS):
            grp.AddItem(t)
            n += 1
    b.Save(path)
    print('hand group:', n, 'items')


if __name__ == '__main__':
    main(sys.argv[1])
