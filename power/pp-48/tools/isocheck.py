#!/usr/bin/env python3
"""Check the controller island's isolation on a routed (zone-filled) core board (ADR 0003 decision 14).
Usage: isocheck.py <board.kicad_pcb> [gap_mm]

Every copper item of the controller world (CTRL_* nets, the link side, L2's secondary SEC2, the 12 V LED)
against every bus-world item on the same layer: pads, tracks, vias, zone fills. Lists the pairs closer than
gap (default 1.0 mm) and the closest pair per layer. The isolators U4 / U5 and L2's own pads are the
barrier parts and are left out against each other; their pads still count against everything else.
"""
import sys

import pcbnew

CTRL_PREFIX = ('CTRL_', '/Link/CTRL_', '/Link/ISO_OUTA')
CTRL_NETS = ('/Supplies/SEC2', '/Supplies/LED12_A')
BARRIER = ('U4', 'U5', 'L2')


def ctrl(net):
    return net.startswith(CTRL_PREFIX) or net in CTRL_NETS


def items(board, layer):
    """(net, owner ref or '', shape, bbox) of the copper on a layer"""
    out = []
    for fp in board.GetFootprints():
        for p in fp.Pads():
            if p.IsOnLayer(layer) and p.GetNetname():
                out.append((p.GetNetname(), fp.GetReference(), p.GetEffectiveShape(layer), p.GetBoundingBox()))
    for t in board.Tracks():
        if t.IsOnLayer(layer) and t.GetNetname():
            out.append((t.GetNetname(), '', t.GetEffectiveShape(layer), t.GetBoundingBox()))
    for z in board.Zones():
        if not z.GetIsRuleArea() and z.IsOnLayer(layer) and z.GetNetname():
            fill = z.GetFilledPolysList(layer)
            for i in range(fill.OutlineCount()):
                poly = pcbnew.SHAPE_POLY_SET(fill.Outline(i))
                out.append((z.GetNetname(), '', poly, poly.BBox()))
    return out


def near(a, b, gap):
    return not (a.GetRight() + gap < b.GetLeft() or b.GetRight() + gap < a.GetLeft()
                or a.GetBottom() + gap < b.GetTop() or b.GetBottom() + gap < a.GetTop())


def main(path, gap):
    board = pcbnew.LoadBoard(path)
    g = pcbnew.FromMM(gap)
    bad = 0
    for layer in (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu):
        its = items(board, layer)
        cs = [i for i in its if ctrl(i[0])]
        bs = [i for i in its if not ctrl(i[0])]
        for cn, cr, csh, cbb in cs:
            for bn, br, bsh, bbb in bs:
                if cr in BARRIER and br == cr or not near(cbb, bbb, g):
                    continue
                if csh.Collide(bsh, g) if not isinstance(csh, pcbnew.SHAPE_POLY_SET) else bsh.Collide(csh, g):
                    bad += 1
                    x, y = pcbnew.ToMM(cbb.GetCenter().x), pcbnew.ToMM(cbb.GetCenter().y)
                    print('%s: %s %s ~ %s %s near (%.2f, %.2f)' % (board.GetLayerName(layer), cn, cr or '-', bn, br or '-', x, y))
    print('isocheck: %d pair(s) closer than %.2f mm' % (bad, gap))
    return bad


if __name__ == '__main__':
    sys.exit(1 if main(sys.argv[1], float(sys.argv[2]) if len(sys.argv) > 2 else 1.0) else 0)
