#!/usr/bin/env python3
"""Copy unit 1's routed copper to units 2..N/2 (docs/adr/0004 decisions 4, 9).
Usage: copycol.py <in.kicad_pcb> <out.kicad_pcb> [N]

krt-route routes every unit; only unit 1 is kept as the template. Tracks
and vias outside group "hand" (tools/route_hand.py draws that copper in every
slot itself) are deleted in units 2..N/2 and replaced by unit 1's,
translated by 50 mm per unit, with the nets renamed: /Unit 1/X ->
/Unit u/X, /UNIT1_* -> /UNITu_*; GND, VBUS and +3V3 stay. A routed track or
via that is not wholly inside unit 1 stops the copy: such a route would
not be the same in every unit. Core routes (left of unit 1) stay. The plane zones (whole board) are left
alone; refill them afterwards (kicad-cli pcb drc --refill-zones).
"""
import sys

import pcbnew

PITCH, X0 = 50.0, 100.0            # unit pitch
GROUP = 'hand'


def mm(v):
    return pcbnew.FromMM(v)


def col_of(x):
    return int((pcbnew.ToMM(x) - X0) // PITCH) + 1


def rename(name, c):
    if name.startswith('/Unit 1/'):
        return '/Unit %d/%s' % (c, name[len('/Unit 1/'):])
    if name.startswith('/UNIT1_'):
        return '/UNIT%d_%s' % (c, name[len('/UNIT1_'):])
    return name


def main(src, dst, n):
    board = pcbnew.LoadBoard(src)
    hand = set()
    for g in board.Groups():
        if g.GetName() == GROUP:
            hand = {it.m_Uuid.AsString() for it in g.GetItems()}
    t = board.Tracks()
    routed = [t[i] for i in range(len(t)) if t[i].m_Uuid.AsString() not in hand]
    template, other, bad = [], [], []
    for it in routed:
        cols = {col_of(it.GetStart().x), col_of(it.GetEnd().x)}
        if cols == {1}:
            template.append(it)
        elif 1 in cols:
            bad.append(it)
        elif min(cols) >= 2:
            other.append(it)                    # core routes (unit 0) stay
    if bad:
        for it in bad:
            print('crosses the unit 1 edge: %s %s (%.2f, %.2f)-(%.2f, %.2f)' % (
                it.GetNetname(), it.GetLayerName(), pcbnew.ToMM(it.GetStart().x), pcbnew.ToMM(it.GetStart().y),
                pcbnew.ToMM(it.GetEnd().x), pcbnew.ToMM(it.GetEnd().y)))
        raise SystemExit('%d routed items leave unit 1; reroute before copying' % len(bad))
    for it in other:
        board.Delete(it)
    made = 0
    for c in range(2, n // 2 + 1):
        d = pcbnew.VECTOR2I(mm(PITCH * (c - 1)), 0)
        for it in template:
            ni = board.FindNet(rename(it.GetNetname(), c))
            if ni is None:
                raise SystemExit('no net %s' % rename(it.GetNetname(), c))
            cp = it.Duplicate()
            cp.Move(d)
            cp.SetNet(ni)
            board.Add(cp)
            made += 1
    board.Save(dst)
    print('template', len(template), 'items; removed', len(other), '; copied', made, 'into units 2..%d' % (n // 2))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 2)
