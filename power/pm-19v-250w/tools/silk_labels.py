#!/usr/bin/env python3
"""Module silk the generic tools do not know about. Run after pcbkit silk.
- J1 (all four screw pads in one footprint): reference hidden, each screw
  gets its net (S1 VIN, S2 GND, S3 GND, S4 VOUT, docs/pm-interface.md) on
  both sides, at the first free spot around the screw.
- J2 (aux pads, B.Cu only in an F footprint): reference moved to
  B.Silkscreen next to the pads.
Free = clear of pads, other silk and the board edge (vias are tented). Everything goes
in group "labels"; a re-run replaces it.
Usage: silk_labels.py <board.kicad_pcb>   (in place)
"""
import sys

import pcbnew

SIZE, THICK, GAP = 1.0, 0.15, 0.2
SCREWS = {'1': 'VIN', '2': 'GND', '3': 'GND', '4': 'VOUT'}
REACH = 8.0     # search radius around a screw for its label


def mm(v):
    return pcbnew.FromMM(v)


def tomm(v):
    return pcbnew.ToMM(v)


def box_of(item, layer=None):
    bb = item.GetBoundingBox()
    return (tomm(bb.GetLeft()), tomm(bb.GetTop()), tomm(bb.GetRight()), tomm(bb.GetBottom()))


def hit(a, b, gap=GAP):
    return not (a[2] + gap <= b[0] or b[2] + gap <= a[0] or a[3] + gap <= b[1] or b[3] + gap <= a[1])


def obstacles(b, side):
    cu = pcbnew.F_Cu if side == 'F' else pcbnew.B_Cu
    silk = pcbnew.F_SilkS if side == 'F' else pcbnew.B_SilkS
    obs = []
    for fp in b.GetFootprints():
        for p in fp.Pads():
            if p.IsOnLayer(cu):
                obs.append(box_of(p))
        for it in fp.GraphicalItems():
            if it.GetLayer() == silk:
                obs.append(box_of(it))
        for t in (fp.Reference(), fp.Value()):
            if t.IsVisible() and t.GetLayer() == silk:
                obs.append(box_of(t))
    for d in b.GetDrawings():
        if d.GetLayer() == silk:
            obs.append(box_of(d))
    return obs


def text(b, grp, s, x, y, side, rot=0):
    t = pcbnew.PCB_TEXT(b)
    t.SetText(s)
    t.SetLayer(pcbnew.F_SilkS if side == 'F' else pcbnew.B_SilkS)
    t.SetTextSize(pcbnew.VECTOR2I(mm(SIZE), mm(SIZE)))
    t.SetTextThickness(mm(THICK))
    t.SetMirrored(side == 'B')
    t.SetTextAngleDegrees(rot)
    t.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
    b.Add(t)
    return t


def place(b, grp, s, cands, side, obs, edge):
    for c in cands:
        x, y, rot = c if len(c) == 3 else (*c, 0)
        t = text(b, grp, s, x, y, side, rot)
        bb = box_of(t)
        inside = (bb[0] >= edge[0] + 0.3 and bb[1] >= edge[1] + 0.3 and
                  bb[2] <= edge[2] - 0.3 and bb[3] <= edge[3] - 0.3)
        if inside and not any(hit(bb, o) for o in obs):
            obs.append(bb)
            grp.AddItem(t)
            return (round(x, 2), round(y, 2), rot)
        b.Delete(t)
    return None


def main(path):
    b = pcbnew.LoadBoard(path)
    e = b.GetBoardEdgesBoundingBox()
    edge = (tomm(e.GetLeft()), tomm(e.GetTop()), tomm(e.GetRight()), tomm(e.GetBottom()))
    for d in list(b.GetDrawings()):
        g = d.GetParentGroup()
        if g and g.GetName() == 'labels':
            b.Delete(d)
    for g in list(b.Groups()):
        if g.GetName() == 'labels':
            b.Delete(g)
    grp = pcbnew.PCB_GROUP(b)
    grp.SetName('labels')
    b.Add(grp)

    j1 = b.FindFootprintByReference('J1')
    j1.Reference().SetVisible(False)
    j2 = b.FindFootprintByReference('J2')
    j2.Reference().SetVisible(False)

    for side in ('F', 'B'):
        obs = obstacles(b, side)
        for p in j1.Pads():
            name = SCREWS.get(p.GetNumber())
            if not name or tomm(p.GetSize().x) < 3:
                continue      # the small stitching pads around each screw
            x, y = tomm(p.GetPosition().x), tomm(p.GetPosition().y)
            grid = [(x + 0.25 * i, y + 0.25 * j) for i in range(-32, 33) for j in range(-32, 33)
                    if 0.25 * (i * i + j * j) ** 0.5 <= REACH]
            grid.sort(key=lambda q: (q[0] - x) ** 2 + (q[1] - y) ** 2)
            cands = [(qx, qy, 0) for qx, qy in grid] + [(qx, qy, 90) for qx, qy in grid]
            got = place(b, grp, name, cands, side, obs, edge)
            print(side, 'S' + p.GetNumber(), name, got if got else 'NO ROOM')
    obs = obstacles(b, 'B')
    xs = [tomm(p.GetPosition().x) for p in j2.Pads()]
    ys = [tomm(p.GetPosition().y) for p in j2.Pads()]
    jx, y1 = (min(xs) + max(xs)) / 2, max(ys)
    cands = [(jx, y1 + 2.0 + 0.4 * k) for k in range(5)]
    cands += [(min(xs) - 2.6 - 0.4 * k, (min(ys) + y1) / 2) for k in range(4)]
    cands += [(max(xs) + 2.6 + 0.4 * k, (min(ys) + y1) / 2) for k in range(4)]
    got = place(b, grp, 'J2', cands, 'B', obs, edge)
    print('B J2', got if got else 'NO ROOM')
    b.Save(path)


if __name__ == '__main__':
    main(sys.argv[1])
