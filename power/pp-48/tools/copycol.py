#!/usr/bin/env python3
"""Copy column 1's routed copper to columns 2..N (docs/adr/0004 decision 4).
Usage: copycol.py <in.kicad_pcb> <out.kicad_pcb> [N]

krt-route routes every column; only column 1 is kept as the template. Tracks
and vias outside group "hand" (tools/route_hand.py draws that copper in every
column itself) are deleted in columns 2..N and replaced by column 1's,
translated by 25 mm per column, with the nets renamed: /Column 1/X ->
/Column c/X, /COL1_* -> /COLc_*; GND, VBUS and +3V3 stay. A routed track or
via that is not wholly inside column 1 stops the copy: such a route would
not be the same in every column. The plane zones (whole board) are left
alone; refill them afterwards (kicad-cli pcb drc --refill-zones).
"""
import sys

import pcbnew

PITCH, X0 = 25.0, 100.0
GROUP = 'hand'


def mm(v):
    return pcbnew.FromMM(v)


def col_of(x):
    return int((pcbnew.ToMM(x) - X0) // PITCH) + 1


def rename(name, c):
    if name.startswith('/Column 1/'):
        return '/Column %d/%s' % (c, name[len('/Column 1/'):])
    if name.startswith('/COL1_'):
        return '/COL%d_%s' % (c, name[len('/COL1_'):])
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
        else:
            other.append(it)
    if bad:
        for it in bad:
            print('crosses the column 1 edge: %s %s (%.2f, %.2f)-(%.2f, %.2f)' % (
                it.GetNetname(), it.GetLayerName(), pcbnew.ToMM(it.GetStart().x), pcbnew.ToMM(it.GetStart().y),
                pcbnew.ToMM(it.GetEnd().x), pcbnew.ToMM(it.GetEnd().y)))
        raise SystemExit('%d routed items leave column 1; reroute before copying' % len(bad))
    for it in other:
        board.Delete(it)
    made = 0
    for c in range(2, n + 1):
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
    print('template', len(template), 'items; removed', len(other), '; copied', made, 'into columns 2..%d' % n)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 2)
