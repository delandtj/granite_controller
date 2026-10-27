#!/usr/bin/env python3
"""High-current copper for the bench carrier: the VIN and VOUT trunks as wide
tracks on F.Cu (and B.Cu where both ends are through-hole pads), in a group
"power" that krt-route keeps. GND is the gndpour on both layers (run it
before this script: the GND zones are moved into the same group, so KRT
keeps them and treats GND as a plane net instead of routing it as tracks),
with the 10 A pads set to solid zone connection in tools/place.py.
Usage: power.py <in.kicad_pcb> <out.kicad_pcb>   (after place.py and gndpour)

Paths (13.2 A out at 250 W, ~5.5-7 A in):
  VIN_J  J1.1 -> F1.1                                   2.5 mm, F.Cu
  VIN    F1.2 -> D1.2 -> C1.1 -> C2.1 -> C3.1 -> SO1 -> TP1    2.5 mm, F.Cu;
         C1.1 -> C2.1 -> SO1 also on B.Cu
  VOUT   SO4 -> TP3, SO4 -> C5.1 -> C4.1 -> J2.1          4.0 mm, F.Cu + B.Cu
"""
import sys

import pcbnew

GROUP = 'power'

# net, layer, width, points
TRACKS = [
    ('/VIN_J', 'F.Cu', 2.5, [(105.5, 125.0), (105.5, 109.5), (110.55, 109.5), (110.55, 106.5)]),
    # C1.2 / C2.2 (GND) sit 5 mm right of the VIN pads: the trunk drops below C2
    ('/VIN', 'F.Cu', 2.5, [(115.45, 106.5), (115.45, 112.5), (120.0, 112.5), (120.0, 131.0), (132.5, 131.0),
                           (132.5, 118.47)]),
    ('/VIN', 'F.Cu', 2.5, [(132.5, 119.55), (139.3, 119.55), (139.3, 112.5)]),
    ('/VIN', 'B.Cu', 2.5, [(120.0, 112.5), (120.0, 131.0), (132.5, 131.0), (132.5, 119.55), (139.3, 119.55)]),
    ('/VOUT', 'F.Cu', 4.0, [(190.7, 119.55), (190.7, 112.5)]),
    ('/VOUT', 'F.Cu', 4.0, [(190.7, 115.0), (219.5, 115.0), (219.5, 124.0), (224.5, 124.0)]),
    ('/VOUT', 'F.Cu', 2.0, [(205.0, 115.0), (205.0, 110.0)]),
    ('/VOUT', 'B.Cu', 4.0, [(190.7, 119.55), (190.7, 115.0), (219.5, 115.0), (219.5, 124.0), (224.5, 124.0)]),
]


def mm(v):
    return pcbnew.FromMM(v)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Remove(it)
            board.Remove(g)
    grp = pcbnew.PCB_GROUP(board)
    grp.SetName(GROUP)
    board.Add(grp)
    nets = board.GetNetsByName()
    n = 0
    for net, layer, w, pts in TRACKS:
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(board)
            t.SetStart(pcbnew.VECTOR2I(mm(x0), mm(y0)))
            t.SetEnd(pcbnew.VECTOR2I(mm(x1), mm(y1)))
            t.SetWidth(mm(w))
            t.SetLayer(board.GetLayerID(layer))
            t.SetNet(nets[net])
            board.Add(t)
            grp.AddItem(t)
            n += 1
    zones = 0
    for z in board.Zones():
        if not z.GetIsRuleArea() and z.GetNetname() == 'GND':
            grp.AddItem(z)
            zones += 1
    board.Save(dst)
    print('power tracks', n, 'GND zones grouped', zones)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
