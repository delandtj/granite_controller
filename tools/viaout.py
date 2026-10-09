#!/usr/bin/env python3
"""Move vias out of SMD pads (KRT drops its plane welds inside pads).

Usage: viaout.py <in.kicad_pcb> <out.kicad_pcb>

For every via whose centre lies in an SMD pad of the same net (except the
U1 pad 29 thermal vias), try spots just outside the pad in 16 directions,
nearest first. A spot is taken when the via and its stubs clear every
other-net item by CLR on both outer layers. The pad gets an F.Cu stub to the
new via; B.Cu tracks that ended on the old via get a B.Cu stub. Vias with no
free spot are listed and left alone. Refill zones and run DRC afterwards.
Same scratch-dir rule as mkboard/outline.
"""
import math
import sys
import pcbnew

src, out = sys.argv[1:3]
CLR = 0.21   # mm
KEEP = {('U1', '29')}

b = pcbnew.LoadBoard(src)
mm, nm = pcbnew.ToMM, pcbnew.FromMM
LAYERS = (pcbnew.F_Cu, pcbnew.B_Cu)

t = b.Tracks()
tracks = [t[i].Cast() for i in range(len(t))]
pads = []
for f in b.GetFootprints():
    P = f.Pads()
    pads += [(f.GetReference(), P[i]) for i in range(len(P))]


def in_pad(v):
    for ref, p in pads:
        if (p.GetAttribute() == pcbnew.PAD_ATTRIB_SMD and p.GetNetCode() == v.GetNetCode()
                and (ref, p.GetNumber()) not in KEEP and p.HitTest(v.GetPosition())):
            return ref, p
    return None


def _pt_seg(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    L = dx * dx + dy * dy
    u = 0.0 if L == 0 else max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / L))
    return math.hypot(px - ax - u * dx, py - ay - u * dy)


def _seg_seg(a, b2, c, d):
    """Distance between segments a-b2 and c-d (points as (x, y))."""
    def ccw(p, q, r):
        return (r[1] - p[1]) * (q[0] - p[0]) - (q[1] - p[1]) * (r[0] - p[0])
    if (ccw(a, b2, c) > 0) != (ccw(a, b2, d) > 0) and (ccw(c, d, a) > 0) != (ccw(c, d, b2) > 0):
        return 0.0
    return min(_pt_seg(*a, *c, *d), _pt_seg(*b2, *c, *d), _pt_seg(*c, *a, *b2), _pt_seg(*d, *a, *b2))


def _seg_box(a, b2, box):
    x0, y0, x1, y1 = box
    if x0 <= a[0] <= x1 and y0 <= a[1] <= y1:
        return 0.0
    edges = [((x0, y0), (x1, y0)), ((x1, y0), (x1, y1)), ((x1, y1), (x0, y1)), ((x0, y1), (x0, y0))]
    return min(_seg_seg(a, b2, e0, e1) for e0, e1 in edges)


def clear(a, z, half, layer, net):
    """Segment a-z (mm tuples) of half-width `half` clears other-net copper on layer."""
    lo = (min(a[0], z[0]) - 2, min(a[1], z[1]) - 2, max(a[0], z[0]) + 2, max(a[1], z[1]) + 2)
    for it in tracks:
        if it.GetNetCode() == net or not it.IsOnLayer(layer):
            continue
        s, e = it.GetStart(), it.GetEnd()
        p, q = (mm(s.x), mm(s.y)), (mm(e.x), mm(e.y))
        if max(p[0], q[0]) < lo[0] or min(p[0], q[0]) > lo[2] or max(p[1], q[1]) < lo[1] or min(p[1], q[1]) > lo[3]:
            continue
        w = mm(it.GetWidth(layer)) / 2 if it.GetClass() == 'PCB_VIA' else mm(it.GetWidth()) / 2
        if _seg_seg(a, z, p, q) < half + w + CLR:
            return False
    for _, pd in pads:
        if pd.GetNetCode() == net or not pd.IsOnLayer(layer):
            continue
        bb = pd.GetBoundingBox()
        box = (mm(bb.GetLeft()), mm(bb.GetTop()), mm(bb.GetRight()), mm(bb.GetBottom()))
        if box[2] < lo[0] or box[0] > lo[2] or box[3] < lo[1] or box[1] > lo[3]:
            continue
        if _seg_box(a, z, box) < half + CLR:
            return False
    return True


moved, stuck = 0, []
for v in [x for x in tracks if x.GetClass() == 'PCB_VIA']:
    hit = in_pad(v)
    if not hit:
        continue
    ref, p = hit
    c = v.GetPosition()
    pb = p.GetBoundingBox()
    r = v.GetWidth(pcbnew.F_Cu) // 2
    ends_b = [x for x in tracks if x.GetClass() == 'PCB_TRACK' and x.GetLayer() == pcbnew.B_Cu
              and (x.GetStart() == c or x.GetEnd() == c)]
    best = None
    for k in range(16):
        a = k * math.pi / 8
        dx, dy = math.cos(a), math.sin(a)
        # distance from the via to the pad edge along (dx, dy), then out by r + 0.2
        for extra in (0.2, 0.35, 0.5, 0.7, 0.9, 1.2, 1.5):
            q = pcbnew.VECTOR2I(int(c.x + dx * nm(0.05)), int(c.y + dy * nm(0.05)))
            while pb.Contains(q):
                q = pcbnew.VECTOR2I(int(q.x + dx * nm(0.05)), int(q.y + dy * nm(0.05)))
            n = pcbnew.VECTOR2I(int(q.x + dx * (r + nm(extra))), int(q.y + dy * (r + nm(extra))))
            dist = math.hypot(n.x - c.x, n.y - c.y)
            if best and dist >= best[0]:
                break
            cm, nmm, net = (mm(c.x), mm(c.y)), (mm(n.x), mm(n.y)), v.GetNetCode()
            ok = all(clear(nmm, nmm, mm(r), L, net) for L in LAYERS)
            ok = ok and clear(cm, nmm, 0.1, pcbnew.F_Cu, net)
            if ok and ends_b:
                ok = clear(cm, nmm, 0.1, pcbnew.B_Cu, net)
            if ok:
                best = (dist, n)
                break
    if not best:
        stuck.append(f'{ref}.{p.GetNumber()}')
        continue
    n = best[1]
    v.SetPosition(n)
    for L in (pcbnew.F_Cu,) + ((pcbnew.B_Cu,) if ends_b else ()):
        s = pcbnew.PCB_TRACK(b)
        s.SetStart(c); s.SetEnd(n); s.SetWidth(nm(0.2)); s.SetLayer(L); s.SetNet(v.GetNet())
        b.Add(s)
        tracks.append(s)
    moved += 1

b.Save(out)
print('moved', moved, 'vias out of pads')
if stuck:
    print('no free spot:', ' '.join(stuck))
