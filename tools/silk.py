#!/usr/bin/env python3
"""Tidy silkscreen after placement.

Usage: silk.py <in.kicad_pcb> <out.kicad_pcb>

- Value fields go to F.Fab (assembly does not need them on silk).
- Small passives (courtyard < 8 mm^2: 0402/0603 R/C/L, small diodes,
  resistor arrays) get their silk reference hidden; the F.Fab
  ${REFERENCE} text stays for the assembly drawing.
- Footprint silk lines thinner than 0.15 mm are widened to 0.15 mm (JLC
  minimum line width; library parts use 0.12 mm).
- Every other reference is set to 1.0 mm / 0.15 mm (JLC minimum text
  height) and moved to the first
  spot around its courtyard (above, below, right, left), else inside its own
  courtyard, that is inside the board, off every pad and silk outline, off other
  parts'
  courtyards and off already-placed references.
  If none is free, it is hidden.
Same scratch-dir rule as the other tools.
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
board = pcbnew.LoadBoard(src)
_f = board.Footprints()
FPS = [_f[i] for i in range(len(_f))]
SIZE, THICK, GAP = 1.0, 0.15, 0.2
MIN_LINE = 0.15


def mm(v):
    return pcbnew.ToMM(v)


def box(b):
    return (mm(b.GetLeft()), mm(b.GetTop()), mm(b.GetRight()), mm(b.GetBottom()))


def crt(fp):
    c = fp.GetCourtyard(pcbnew.F_CrtYd)
    return box(c.BBox()) if c.OutlineCount() else box(fp.GetBoundingBox(False))


def overlap(a, b, g=GAP):
    return not (a[2] + g <= b[0] or b[2] + g <= a[0] or a[3] + g <= b[1] or b[3] + g <= a[1])


edge = box(board.GetBoardEdgesBoundingBox())
pads, courts, segs = [], {}, []
for fp in FPS:
    courts[fp.GetReference()] = crt(fp)
    p = fp.Pads()
    for i in range(len(p)):
        pads.append(box(p[i].GetBoundingBox()))
    g = fp.GraphicalItems()
    for i in range(len(g)):
        it = g[i].Cast()
        if it.GetLayer() not in (pcbnew.F_SilkS, pcbnew.B_SilkS) or not isinstance(it, pcbnew.PCB_SHAPE):
            continue
        if it.GetWidth() < pcbnew.FromMM(MIN_LINE):
            it.SetWidth(pcbnew.FromMM(MIN_LINE))
        if it.GetLayer() != pcbnew.F_SilkS:
            continue
        st = it.GetShape()
        if st == pcbnew.SHAPE_T_SEGMENT:
            q = [it.GetStart(), it.GetEnd()]
            segs.append(((mm(q[0].x), mm(q[0].y)), (mm(q[1].x), mm(q[1].y))))
        elif st == pcbnew.SHAPE_T_POLY:
            o = it.GetPolyShape().Outline(0)
            pts = [o.CPoint(k) for k in range(o.PointCount())]
            for k in range(len(pts)):
                a1, a2 = pts[k], pts[(k + 1) % len(pts)]
                segs.append(((mm(a1.x), mm(a1.y)), (mm(a2.x), mm(a2.y))))
        else:                                   # rect, arc, circle: bbox edges
            x0, y0, x1, y1 = box(it.GetBoundingBox())
            segs += [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)),
                     ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))]


def seg_hits(t, sg, g=GAP):
    """Liang-Barsky: does segment sg cross box t grown by g?"""
    x0, y0, x1, y1 = t[0] - g, t[1] - g, t[2] + g, t[3] + g
    (ax, ay), (bx, by) = sg
    dx, dy = bx - ax, by - ay
    u0, u1 = 0.0, 1.0
    for pp, qq in ((-dx, ax - x0), (dx, x1 - ax), (-dy, ay - y0), (dy, y1 - ay)):
        if pp == 0:
            if qq < 0:
                return False
            continue
        r = qq / pp
        if pp < 0:
            u0 = max(u0, r)
        else:
            u1 = min(u1, r)
        if u0 > u1:
            return False
    return True

taken = []
hidden = moved = 0
# connectors first (their labels matter for cabling), then ICs, then the rest
ORDER = sorted(FPS, key=lambda f: {"J": 0, "U": 1}.get(f.GetReference()[0], 2))
for fp in ORDER:
    val = fp.Value()
    if val.GetLayer() == pcbnew.F_SilkS:
        val.SetLayer(pcbnew.F_Fab)
    ref = fp.Reference()
    r = fp.GetReference()
    c = courts[r]
    if r.startswith("H"):
        ref.SetVisible(False)
        continue
    on_board = edge[0] <= c[0] and c[2] <= edge[2] + 7  # WROOM may overhang the bottom edge
    area = (c[2] - c[0]) * (c[3] - c[1])
    if area < 8.0 or not on_board:
        ref.SetVisible(False)
        hidden += 1
        continue
    ref.SetVisible(True)
    ref.SetLayer(pcbnew.F_SilkS)
    ref.SetTextSize(pcbnew.VECTOR2I_MM(SIZE, SIZE))
    ref.SetTextThickness(pcbnew.FromMM(THICK))
    ref.SetTextAngle(pcbnew.EDA_ANGLE(0, pcbnew.DEGREES_T))
    cx, cy = (c[0] + c[2]) / 2, (c[1] + c[3]) / 2
    cands = [(cx, c[1] - 0.7), (cx, c[3] + 0.7), (cx, c[3] + 0.95), (cx, c[3] + 1.2), (c[2] + 0.4 + len(r) * 0.35, cy),
             (c[0] - 0.4 - len(r) * 0.35, cy)]
    # then anywhere inside the part's own courtyard, clear of pads (connector bodies)
    inside = []
    yy = c[1] + 0.6
    while yy <= c[3] - 0.6:
        inside.append((cx, yy))
        yy += 0.25
    cands += sorted(inside, key=lambda q: -abs(q[1] - cy))
    def fits(x, y, ang, strict):
        ref.SetTextAngle(pcbnew.EDA_ANGLE(ang, pcbnew.DEGREES_T))
        ref.SetTextPos(pcbnew.VECTOR2I_MM(x, y))
        t = box(ref.GetBoundingBox())
        if not (edge[0] + 0.3 <= t[0] and t[2] <= edge[2] - 0.3 and
                edge[1] + 0.3 <= t[1] and t[3] <= edge[3] - 0.3):
            return False
        if any(overlap(t, p) for p in pads) or any(seg_hits(t, sg) for sg in segs):
            return False
        if strict and any(overlap(t, cc, 0.0) for rr, cc in courts.items() if rr != r):
            return False
        if any(overlap(t, o) for o in taken):
            return False
        taken.append(t)
        return True

    tries = [(x, y, 0, True) for x, y in cands]
    if r.startswith("J"):
        # connectors: their labels matter for cabling, so relax step by step
        side = [(c[0] - 0.8, cy, 90), (c[2] + 0.8, cy, 90)]
        tries += [(x, y, a, True) for x, y, a in side]
        tries += [(x, y, 0, False) for x, y in cands]
        tries += [(x, y, a, False) for x, y, a in side]
    ok = any(fits(*t) for t in tries)
    if ok:
        moved += 1
    else:
        ref.SetVisible(False)
        hidden += 1

board.Save(out)
print(f"references placed: {moved}, hidden: {hidden}")
