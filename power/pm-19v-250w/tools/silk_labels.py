#!/usr/bin/env python3
"""Module silk the generic tools do not know about. Run after pcbkit silk.
- J1 (all four screw pads in one footprint): reference hidden, each screw
  gets its net (S1 VIN, S2 GND, S3 GND, S4 VOUT, docs/pm-interface.md) on
  both sides, at the first free spot around the screw.
- J2 (1.27 mm aux header on B): reference on B.Silkscreen next to its
  pads, at the first free spot below, left or right of them.
- References pcbkit silk hid for lack of room (parts above REF_AREA, so
  not the 0402/0603 passives), and visible ones that ended up under a part
  body: next to the part's own courtyard, nearest free spot, horizontal
  first. A part with no free spot keeps its reference hidden.
Free = clear of pads, other silk, part bodies (Fab outline) and the board
edge (vias are tented). Labels go in group "labels"; a re-run replaces them.
Usage: silk_labels.py <board.kicad_pcb>   (in place)
"""
import sys

import pcbnew

SIZE, THICK, GAP = 1.0, 0.15, 0.2
TEXT_GAP = 0.6  # label to label, so neighbours do not run together
SCREWS = {'1': 'VIN', '2': 'GND', '3': 'GND', '4': 'VOUT'}
REACH = 8.0     # search radius around a screw for its label
REF_AREA = 5.0  # mm^2 courtyard: 0805 and up get a reference
REF_REACH = 1.2 # furthest a reference sits from its own courtyard


def mm(v):
    return pcbnew.FromMM(v)


def tomm(v):
    return pcbnew.ToMM(v)


def grow(q, d):
    return (q[0] - d, q[1] - d, q[2] + d, q[3] + d)


def box_of(item, layer=None):
    if isinstance(item, pcbnew.EDA_TEXT):
        bb = item.GetEffectiveTextShape().BBox()      # the glyphs, not the line box
    else:
        bb = item.GetBoundingBox()
    return (tomm(bb.GetLeft()), tomm(bb.GetTop()), tomm(bb.GetRight()), tomm(bb.GetBottom()))


def hit(a, b, gap=GAP):
    return not (a[2] + gap <= b[0] or b[2] + gap <= a[0] or a[3] + gap <= b[1] or b[3] + gap <= a[1])


def body(fp, side):
    """Bounding box of the footprint's Fab outline on that side, or None."""
    fab = pcbnew.F_Fab if side == 'F' else pcbnew.B_Fab
    boxes = [box_of(it) for it in fp.GraphicalItems()
             if it.GetLayer() == fab and isinstance(it, pcbnew.PCB_SHAPE)]
    if not boxes:
        return None
    return (min(q[0] for q in boxes), min(q[1] for q in boxes),
            max(q[2] for q in boxes), max(q[3] for q in boxes))


def obstacles(b, side, skip_ref=None):
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
            if t.IsVisible() and t.GetLayer() == silk and fp.GetReference() != skip_ref:
                obs.append(grow(box_of(t), TEXT_GAP - GAP))
        bd = body(fp, side)
        if bd and fp.GetReference() != 'J1':      # J1's Fab outline is the whole module
            obs.append(bd)
    for d in b.GetDrawings():
        if d.GetLayer() == silk:
            obs.append(grow(box_of(d), TEXT_GAP - GAP) if isinstance(d, pcbnew.PCB_TEXT) else box_of(d))
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
            # not between the two screws of a column, where VIN and GND
            # would read as one pair
            mid = (edge[1] + edge[3]) / 2
            grid.sort(key=lambda q: ((q[0] - x) ** 2 + (q[1] - y) ** 2) ** 0.5
                      + (3.0 if (q[1] - y) * (mid - y) > 0 else 0.0))
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
    refs(b, edge)
    b.Save(path)


def gap_to(bb, c):
    """Distance between two boxes (0 when they touch or overlap)."""
    dx = max(c[0] - bb[2], bb[0] - c[2], 0.0)
    dy = max(c[1] - bb[3], bb[1] - c[3], 0.0)
    return (dx * dx + dy * dy) ** 0.5


def ownership(b, side):
    """Courtyard boxes of every part on that side, by reference."""
    layer = pcbnew.B_CrtYd if side == 'B' else pcbnew.F_CrtYd
    out = {}
    for fp in b.GetFootprints():
        if fp.GetReference() == 'J1' or fp.IsFlipped() != (side == 'B'):
            continue
        cy = fp.GetCourtyard(layer)
        if cy.OutlineCount():
            bb = cy.BBox()
            out[fp.GetReference()] = (tomm(bb.GetLeft()), tomm(bb.GetTop()), tomm(bb.GetRight()), tomm(bb.GetBottom()))
    return out


def clear_owner(bb, ref, owners, margin=0.5):
    """The label reads as ref's: nearer its courtyard than any other part's."""
    own = gap_to(bb, owners[ref])
    return all(gap_to(bb, c) >= own + margin for r, c in owners.items() if r != ref)


def refs(b, edge):
    fps = []
    for fp in b.GetFootprints():
        if fp.GetReference() in ('J1', 'J2'):
            continue
        side = 'B' if fp.IsFlipped() else 'F'
        cy = fp.GetCourtyard(pcbnew.B_CrtYd if side == 'B' else pcbnew.F_CrtYd)
        if cy.OutlineCount() == 0 or tomm(tomm(cy.Area())) < REF_AREA:
            continue
        fps.append((tomm(tomm(cy.Area())), fp, side, cy.BBox()))
    fps.sort(key=lambda q: -q[0])
    for _, fp, side, cb in fps:
        r = fp.Reference()
        obs = obstacles(b, side, skip_ref=fp.GetReference())
        owners = ownership(b, side)
        if (r.IsVisible() and not any(hit(box_of(r), o) for o in obs)
                and clear_owner(box_of(r), fp.GetReference(), owners)):
            continue
        c = (tomm(cb.GetLeft()), tomm(cb.GetTop()), tomm(cb.GetRight()), tomm(cb.GetBottom()))
        cx, cy_ = (c[0] + c[2]) / 2, (c[1] + c[3]) / 2
        silk = pcbnew.F_SilkS if side == 'F' else pcbnew.B_SilkS
        r.SetLayer(silk)
        r.SetMirrored(side == 'B')
        r.SetTextSize(pcbnew.VECTOR2I(mm(SIZE), mm(SIZE)))
        r.SetTextThickness(mm(THICK))
        r.SetKeepUpright(False)
        cands = []
        for rot in (0, 90):
            r.SetTextAngleDegrees(rot)
            r.SetPosition(pcbnew.VECTOR2I(mm(cx), mm(cy_)))
            tb = box_of(r)
            hw, hh = (tb[2] - tb[0]) / 2, (tb[3] - tb[1]) / 2
            steps = [0.1 * k for k in range(int(REF_REACH / 0.1) + 1)]
            for d in steps:
                for t in range(-20, 21):
                    for x, y in ((cx + 0.25 * t, c[1] - d - hh), (cx + 0.25 * t, c[3] + d + hh),
                                 (c[2] + d + hw, cy_ + 0.25 * t), (c[0] - d - hw, cy_ + 0.25 * t)):
                        if c[0] - hw <= x <= c[2] + hw and c[1] - hh <= y <= c[3] + hh:
                            cands.append((d + 0.05 * abs(t) + 0.5 * (rot == 90), x, y, rot))
        cands.sort()
        got = None
        for _, x, y, rot in cands:
            r.SetTextAngleDegrees(rot)
            r.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
            bb = box_of(r)
            inside = (bb[0] >= edge[0] + 0.3 and bb[1] >= edge[1] + 0.3 and
                      bb[2] <= edge[2] - 0.3 and bb[3] <= edge[3] - 0.3)
            if (inside and not any(hit(bb, o) for o in obs)
                    and clear_owner(bb, fp.GetReference(), owners)):
                got = (round(x, 2), round(y, 2), rot)
                break
        r.SetVisible(got is not None)
        print(side, fp.GetReference(), got if got else 'hidden (no room)')


if __name__ == '__main__':
    main(sys.argv[1])
