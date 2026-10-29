#!/usr/bin/env python3
"""Tie floating F / B GND (and CTRL_GND) pour islands to In1 with one via each.
Usage: gndstitch.py <in.kicad_pcb> <out.kicad_pcb>

Run after tools/copycol.py. The GND pours of tools/route_hand.py are in the
group krt-route keeps, so KRT treats GND as a plane net and draws no GND
tracks; the price is that a pour island between parts can end up with no
through-hole GND pad and no via in it, and the 0402 GND pads on it float.
This fills the zones, and for every F / B GND island without a GND via or
through-hole GND pad inside, places one 0.6 / 0.3 via (group "gnd-stitch",
replaced on a re-run) at the first grid point (0.1 mm, scanned from the
island's centre outward) that lies 0.35 mm inside the island and clears
other-net copper on every layer by 0.2 mm. Islands with no such point are
reported. Same copper gives the same vias, so the columns stay identical.
"""
import math
import sys

import pcbnew

GROUP = 'gnd-stitch'
NETS = ('GND', 'CTRL_GND')          # CTRL_GND: the controller world's pours (core top left)
VIA, DRILL, CLR, STEP = 0.6, 0.3, 0.2, 0.1


def mm(v):
    return pcbnew.FromMM(v)


def stitch(board, gnd, zones, g):
    """one via per F / B island of net gnd with no via or through-hole pad of that net"""
    t = board.Tracks()
    items = [t[i] for i in range(len(t))]
    vias = [it for it in items if it.Type() == pcbnew.PCB_VIA_T]
    pads = [p for fp in board.GetFootprints() for p in fp.Pads()]
    anchors = [v.GetPosition() for v in vias if v.GetNetCode() == gnd.GetNetCode()]
    anchors += [p.GetPosition() for p in pads
                if p.GetNetCode() == gnd.GetNetCode() and p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH]
    others = [it for it in items if it.GetNetCode() != gnd.GetNetCode()]
    opads = [p for p in pads if p.GetNetCode() != gnd.GetNetCode()]
    # other-net pours on F / B (OUT, SENSE, NODE, VBUS, ...); the inner planes give way to a via by themselves
    ozones = [z for z in zones if z.GetNetCode() != gnd.GetNetCode() and z.GetLayer() in (pcbnew.F_Cu, pcbnew.B_Cu)]
    reach = mm(VIA / 2 + CLR)

    def clear(p):
        for it in others:
            if it.HitTest(p, reach):
                return False
        for pd in opads:
            if pd.HitTest(p, reach):
                return False
        for z in ozones:
            for lay in z.GetLayerSet().Seq():
                if z.HitTestFilledArea(lay, p, reach):
                    return False
        for fp in board.GetFootprints():        # plated / unplated holes of any net
            for pd in fp.Pads():
                if pd.HasHole() and (pd.GetPosition() - p).EuclideanNorm() < mm(VIA / 2 + 0.3) + max(pd.GetDrillSize().x, pd.GetDrillSize().y) // 2:
                    return False
        return True

    placed, stuck = 0, []
    for z in zones:
        if z.GetNetCode() != gnd.GetNetCode() or z.GetLayer() not in (pcbnew.F_Cu, pcbnew.B_Cu):
            continue
        polys = z.GetFilledPolysList(z.GetLayer())
        for k in range(polys.OutlineCount()):
            one = pcbnew.SHAPE_POLY_SET(polys.Outline(k))
            for h in range(polys.HoleCount(k)):
                one.AddHole(polys.Hole(k, h))
            if any(one.Contains(a) for a in anchors):
                continue
            box = one.BBox()
            cx, cy = box.Centre().x, box.Centre().y
            pts = []
            for ix in range(int(box.GetWidth() / mm(STEP)) + 1):
                for iy in range(int(box.GetHeight() / mm(STEP)) + 1):
                    x, y = box.GetX() + ix * mm(STEP), box.GetY() + iy * mm(STEP)
                    pts.append(((x - cx) ** 2 + (y - cy) ** 2, x, y))
            inner = one.CloneDropTriangulation()
            inner.Inflate(-mm(VIA / 2 + 0.05), pcbnew.CORNER_STRATEGY_ROUND_ALL_CORNERS, mm(0.01))
            for _, x, y in sorted(pts):
                p = pcbnew.VECTOR2I(int(x), int(y))
                if inner.Contains(p) and clear(p):
                    v = pcbnew.PCB_VIA(board)
                    v.SetPosition(p); v.SetWidth(mm(VIA)); v.SetDrill(mm(DRILL)); v.SetNet(gnd)
                    board.Add(v); g.AddItem(v)
                    anchors.append(p)
                    placed += 1
                    break
            else:
                stuck.append((gnd.GetNetname(), pcbnew.ToMM(cx), pcbnew.ToMM(cy), z.GetLayerName(), one.Area() / 1e12))
    return placed, stuck


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Delete(it)
            board.Delete(g)
    filler = pcbnew.ZONE_FILLER(board)
    zones = [z for z in board.Zones() if not z.GetIsRuleArea()]
    filler.Fill(zones)
    g = pcbnew.PCB_GROUP(board)
    g.SetName(GROUP)
    board.Add(g)
    placed, stuck = 0, []
    for name in NETS:
        p, st = stitch(board, board.FindNet(name), zones, g)
        placed += p
        stuck += st
    board.Save(dst)
    print('gnd-stitch vias:', placed)
    for net, x, y, lay, a in stuck:
        print('no room for a via in the %s %s island at (%.2f, %.2f), %.2f mm2' % (lay, net, x, y, a))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
