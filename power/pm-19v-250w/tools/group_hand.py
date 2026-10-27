#!/usr/bin/env python3
"""Put every track and via that is not in group "power" (tools/power.py)
into group "hand": the scripted routes of tools/route_drive.py and
tools/route_input.py, which [krt] keep_groups protects from krt-route's
strip. Run after the handroute plans, before krt-route.
Usage: group_hand.py <board.kicad_pcb>   (in place)
"""
import sys

import pcbnew


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
        if t.m_Uuid.AsString() not in power:
            grp.AddItem(t)
            n += 1
    b.Save(path)
    print('hand group:', n, 'items')


if __name__ == '__main__':
    main(sys.argv[1])
