#!/usr/bin/env python3
"""Silkscreen function labels for the connectors.

Usage: connlabels.py <in.kicad_pcb> <out.kicad_pcb>

Each connector gets "<ref> <NAME>" on F.Silkscreen where its reference used
to sit (the silk reference is hidden, F.Fab keeps it), and the wire-to-board
connectors get a short legend centred under each pin, between the label and
the body. Everything goes in group "conn-labels"; a re-run replaces it.
Run after silk.py (which would show the references again).
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
board = pcbnew.LoadBoard(src)
MM = pcbnew.FromMM

NAMES = {
    'J1': 'ETHERNET', 'J2': 'USB', 'J3': '19V IN', 'J8': 'EXPANSION',
    'J9': 'I2C', 'J10': 'DRY IN',
    'J4': '1-WIRE', 'J5': '1-WIRE', 'J6': '1-WIRE', 'J7': '1-WIRE',
}
NAMES.update({'J%d' % (10 + n): 'NODE %d' % n for n in range(1, 9)})
NODE = ['PW', 'RS', 'CM', 'L+', 'L-']
LEGENDS = {'J3': ['+', '-'], 'J9': ['G', '3V', 'DA', 'CL'], 'J10': ['1', '2', '3', '4', 'G']}
LEGENDS.update({j: ['3V', 'DQ', 'G'] for j in ('J4', 'J5', 'J6', 'J7')})
LEGENDS.update({'J%d' % (10 + n): NODE for n in range(1, 9)})

NAME_H, LEG_H = 1.0, 1.0   # JLC minimum silk text height
CLR = 0.2          # silk-to-silk and silk-to-pad clearance (mm)

# drop a previous run
for g in list(board.Groups()):
    if g.GetName() == 'conn-labels':
        for it in list(g.GetItems()):
            board.Delete(it)
        board.Delete(g)
group = pcbnew.PCB_GROUP(board)
group.SetName('conn-labels')
board.Add(group)
for ref in NAMES:
    board.FindFootprintByReference(ref).Reference().SetVisible(False)


def box(bb, grow=0.0):
    g = MM(grow)
    return (bb.GetLeft() - g, bb.GetTop() - g, bb.GetRight() + g, bb.GetBottom() + g)


# obstacles: footprint silk (shapes and visible texts), pads, board edge
obst = []
def seq(c):
    # index, do not iterate: SWIG iterators break on Python 3.14
    return [c[i] for i in range(len(c))]


for fp in board.GetFootprints():
    for it in seq(fp.GraphicalItems()):
        if it.GetLayer() == pcbnew.F_SilkS and (not hasattr(it, 'IsVisible') or it.IsVisible()):
            obst.append(box(it.GetBoundingBox(), CLR))
    for f in (fp.Reference(), fp.Value()):
        if f.GetLayer() == pcbnew.F_SilkS and f.IsVisible():
            obst.append(box(f.GetBoundingBox(), CLR))
    for p in fp.Pads():
        if p.IsOnLayer(pcbnew.F_Cu):
            obst.append(box(p.GetBoundingBox(), CLR))
for d in seq(board.Drawings()):
    if d.GetLayer() == pcbnew.F_SilkS:
        obst.append(box(d.GetBoundingBox(), CLR))
edge = box(board.GetBoardEdgesBoundingBox(), -0.3)


def free(bb):
    x0, y0, x1, y1 = box(bb)
    if x0 < edge[0] or y0 < edge[1] or x1 > edge[2] or y1 > edge[3]:
        return False
    return not any(x0 < b[2] and x1 > b[0] and y0 < b[3] and y1 > b[1] for b in obst)


def text(s, x, y, h, angle=0):
    t = pcbnew.PCB_TEXT(board)
    t.SetText(s)
    t.SetLayer(pcbnew.F_SilkS)
    t.SetTextSize(pcbnew.VECTOR2I(MM(h), MM(h)))
    t.SetTextThickness(MM(0.15))
    t.SetPosition(pcbnew.VECTOR2I(MM(x), MM(y)))
    t.SetHorizJustify(pcbnew.GR_TEXT_H_ALIGN_CENTER)
    t.SetVertJustify(pcbnew.GR_TEXT_V_ALIGN_CENTER)
    t.SetTextAngle(pcbnew.EDA_ANGLE(angle, pcbnew.DEGREES_T))
    return t


def ink(t):
    # the stroke outline, not GetBoundingBox() (which pads well past the ink)
    return t.GetEffectiveTextShape().BBox()


def add(t):
    board.Add(t); group.AddItem(t)
    obst.append(box(ink(t), CLR))


def place(cands, s, h):
    """First candidate (x, y[, angle]) where the text fits; None if none does."""
    for c in cands:
        t = text(s, *c[:2], h, *c[2:])
        if free(ink(t)):
            add(t)
            return c
    return None


def place_row(items, h):
    """All texts of a pin legend or none of them."""
    ts = [text(s, x, y, h) for s, x, y in items]
    if all(free(ink(t)) for t in ts):
        for t in ts:
            add(t)
        return True
    return False


mid_y = pcbnew.ToMM(board.GetBoardEdgesBoundingBox().GetCenter().y)
missing = []


def geom(ref):
    fp = board.FindFootprintByReference(ref)
    cy = fp.GetCourtyard(pcbnew.F_Cu).BBox()
    c = [pcbnew.ToMM(v) for v in (cy.GetLeft(), cy.GetTop(), cy.GetRight(), cy.GetBottom())]
    side = 1 if pcbnew.ToMM(fp.GetPosition().y) < mid_y else -1   # away from the edge
    # start from the body's own silk outline (tighter than the courtyard)
    silk = [it.GetBoundingBox() for it in seq(fp.GraphicalItems()) if it.GetLayer() == pcbnew.F_SilkS]
    if silk:
        e = max(pcbnew.ToMM(bb.GetBottom()) for bb in silk) if side > 0 else min(pcbnew.ToMM(bb.GetTop()) for bb in silk)
    else:
        e = c[3] if side > 0 else c[1]
    base = e + side * (CLR + 0.05 + (LEG_H + 0.15) / 2)
    return fp, c, side, base


# pass 1: pin legends right next to the bodies
nline = {}
for ref, leg in LEGENDS.items():
    fp, c, side, base = geom(ref)
    pads = sorted((p for p in fp.Pads() if p.GetNumber().isdigit()), key=lambda p: int(p.GetNumber()))
    if place_row([(s, pcbnew.ToMM(p.GetPosition().x), base) for p, s in zip(pads, leg)], LEG_H):
        nline[ref] = base + side * (LEG_H + 0.15 + CLR + 0.15)
    else:
        missing.append(ref + ' legend')

# pass 2: names, after the legends so they cannot push into a neighbour's
for ref, name in NAMES.items():
    fp, (cx0, cy0, cx1, cy1), side, base = geom(ref)
    ccx, ccy = (cx0 + cx1) / 2, (cy0 + cy1) / 2
    label = '%s %s' % (ref, name)
    n = nline.get(ref, base)
    w = 0.7 * NAME_H * len(label) / 2 + 0.6
    far = (cy0 if side > 0 else cy1) - side * (CLR + 0.6)    # beyond the other side
    cands = [(ccx, n)] + [(ccx + dx, n) for dx in (-1, 1, -2, 2, -3, 3)] + [(ccx, n + side * 0.5), (cx0 - w, base), (cx1 + w, base),
             (cx0 - w, ccy), (cx1 + w, ccy), (ccx, n + side * 1.1), (ccx, far)]
    # crowded rows: vertical beside the body, then the name alone, then the bare reference
    vert = [(x, ccy + side * dy / 4, 90) for x in (cx1 + 0.9, cx0 - 0.9, cx1 + 1.5, cx0 - 1.5) for dy in range(0, 11)]
    if not place(cands + vert, label, NAME_H) and not place(vert, name, NAME_H) \
            and not place(cands + vert, ref, NAME_H):
        missing.append(ref + ' name')

print('not placed:', missing or 'none')
board.Save(out)
print('labels for', len(NAMES), 'connectors')
