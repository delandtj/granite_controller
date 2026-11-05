#!/usr/bin/env python3
"""Close the opens a DRC report lists with a small grid maze router (F / B, vias).
Usage: mazefix.py <in.kicad_pcb> <drc.rpt> <out.kicad_pcb> [--in1]

For every [unconnected_items] pair in the report: a Dijkstra search on a 0.05 mm grid around the two
items, on F.Cu and B.Cu with vias, at the net class track width / via size. Obstacles: copper of other
nets on the layer (pads, tracks, vias, non-GND zones; GND pours refill around the new copper), inflated
by clearance + half the new track (via) size; rule areas that forbid tracks / vias; for vias also the
inner-layer tracks and every hole. The router's copper is left ungrouped. KRT misses these short links
(power nets at their power width, a pin hemmed in by its neighbours); run DRC on the result.
"""
import heapq
import re
import sys

import numpy as np
from matplotlib.path import Path
import pcbnew

STEP = 0.05                # grid, mm
MARGINS = (4.0, 10.0)      # search windows around the two items, mm (the wider one when the first fails)
VIA_COST = 30              # in grid steps
# the router's (ungrouped) GND tracks are soft: the search may cross them at a cost per step, crossed ones
# are deleted (the pours take over; what they do not reach is an open for the next round). Ripping signals
# as well thrashed around the expander's pins
SOFT_GND = 20
CLR = 0.17                 # clearance used (board 0.15, a little margin for the grid)
TW, VD, VH = 0.2, 0.5, 0.2  # track width, via diameter / drill
H2H = 0.42                 # hole to hole (edge; board minimum 0.4)
LAYERS = (pcbnew.F_Cu, pcbnew.B_Cu)
INNER = (pcbnew.In1_Cu, pcbnew.In2_Cu)
STEP_COST = (1, 1)         # per layer; --in1 adds In1 (the GND plane) for short hops at a high cost


def mm(v):
    return pcbnew.FromMM(float(v))


def opens(rpt):
    txt = open(rpt).read()
    out = []
    for block in re.split(r'\n(?=\[)', txt):
        if not block.startswith('[unconnected_items]'):
            continue
        pts = re.findall(r'@\(([\d.]+) mm, ([\d.]+) mm\): .*?\[([^\]]+)\].* on (\S+)', block)
        if len(pts) == 2:
            out.append([(x, y, n, l.rstrip(',')) for x, y, n, l in pts])
    return out


class Grid:
    def __init__(self, x0, y0, x1, y1):
        self.x0, self.y0 = x0, y0
        self.nx, self.ny = int((x1 - x0) / STEP) + 1, int((y1 - y0) / STEP) + 1
        xs = x0 + STEP * np.arange(self.nx)
        ys = y0 + STEP * np.arange(self.ny)
        self.X, self.Y = np.meshgrid(xs, ys)
        self.pts = np.column_stack([self.X.ravel(), self.Y.ravel()])

    def cell(self, x, y):
        return (min(max(int(round((y - self.y0) / STEP)), 0), self.ny - 1),
                min(max(int(round((x - self.x0) / STEP)), 0), self.nx - 1))

    def xy(self, r, c):
        return self.x0 + STEP * c, self.y0 + STEP * r

    def paint(self, mask, item, layer, inflate):
        ps = pcbnew.SHAPE_POLY_SET()
        item.TransformShapeToPolygon(ps, layer, mm(inflate), mm(0.005), pcbnew.ERROR_OUTSIDE)
        self.paint_poly(mask, ps)

    def paint_poly(self, mask, ps):
        for i in range(ps.OutlineCount()):
            ol = ps.COutline(i)
            v = np.array([(pcbnew.ToMM(ol.CPoint(j).x), pcbnew.ToMM(ol.CPoint(j).y)) for j in range(ol.PointCount())])
            if len(v) < 3:
                continue
            if v[:, 0].max() < self.x0 or v[:, 0].min() > self.x0 + STEP * self.nx \
                    or v[:, 1].max() < self.y0 or v[:, 1].min() > self.y0 + STEP * self.ny:
                continue
            mask |= Path(v).contains_points(self.pts).reshape(mask.shape)

    def paint_circle(self, mask, x, y, r):
        mask |= (self.X - x) ** 2 + (self.Y - y) ** 2 <= r * r


def items(board):
    for t in board.Tracks():
        yield t
    for fp in board.GetFootprints():
        for p in fp.Pads():
            yield p
    for z in board.Zones():
        yield z


def blocked(board, g, net):
    """masks: track[layer] (track centre not allowed), via (via centre not allowed)"""
    track = {l: np.zeros((g.ny, g.nx), bool) for l in LAYERS}
    via = np.zeros((g.ny, g.nx), bool)
    soft = []
    for it in items(board):
        if isinstance(it, pcbnew.PCB_TRACK) and net != 'GND' and it.GetNetname() == 'GND' \
                and it.GetParentGroup() is None:
            soft.append(it)
            continue
        is_zone = isinstance(it, pcbnew.ZONE)
        if is_zone and it.GetIsRuleArea():
            # the new copper's edge, not just its centre, stays out
            area = np.zeros_like(via)
            g.paint_poly(area, it.Outline())
            for l in LAYERS:
                if it.IsOnLayer(l) and it.GetDoNotAllowTracks():
                    track[l] |= dilate(area, int(np.ceil(TW / 2 / STEP)) + 1)
            if it.GetDoNotAllowVias():
                via |= dilate(area, int(np.ceil(VD / 2 / STEP)) + 1)
            continue
        if it.GetNetname() == net:
            continue
        if is_zone and it.GetNetname() == 'GND':
            continue
        for l in LAYERS:
            if it.IsOnLayer(l):
                if is_zone:
                    # the hand pours (VBUS, USENSE, ...): their filled copper, else the outline
                    ps = it.GetFilledPolysList(l) if it.IsFilled() else it.Outline()
                    tmp = np.zeros_like(via)
                    g.paint_poly(tmp, ps)
                    grow = int(np.ceil((CLR + TW / 2) / STEP))
                    track[l] |= dilate(tmp, grow)
                    via |= dilate(tmp, int(np.ceil((CLR + VD / 2) / STEP)))
                else:
                    g.paint(track[l], it, l, CLR + TW / 2)
                    g.paint(via, it, l, CLR + VD / 2)
        if not is_zone:
            for l in INNER:
                if it.IsOnLayer(l) and isinstance(it, pcbnew.PCB_TRACK) and not isinstance(it, pcbnew.PCB_VIA):
                    g.paint(via, it, l, CLR + VD / 2)
    # holes (any net): hole to hole
    for it in items(board):
        if isinstance(it, pcbnew.PCB_VIA):
            x, y, d = pcbnew.ToMM(it.GetX()), pcbnew.ToMM(it.GetY()), pcbnew.ToMM(it.GetDrillValue())
            g.paint_circle(via, x, y, d / 2 + H2H + VH / 2)
        elif isinstance(it, pcbnew.PAD) and it.HasHole():
            x, y = pcbnew.ToMM(it.GetX()), pcbnew.ToMM(it.GetY())
            d = max(pcbnew.ToMM(it.GetDrillSizeX()), pcbnew.ToMM(it.GetDrillSizeY()))
            g.paint_circle(via, x, y, d / 2 + H2H + VH / 2)
    # board edge
    edge = 0.35
    bb = board.GetBoardEdgesBoundingBox()
    ex0, ey0, ex1, ey1 = (pcbnew.ToMM(v) for v in (bb.GetLeft(), bb.GetTop(), bb.GetRight(), bb.GetBottom()))
    out = (g.X < ex0 + edge) | (g.X > ex1 - edge) | (g.Y < ey0 + edge) | (g.Y > ey1 - edge)
    for m in list(track.values()) + [via]:
        m |= out
    return track, via, soft


def soft_masks(g, soft):
    """per soft item: its track masks per layer and its via mask"""
    out = []
    for it in soft:
        tm = {l: np.zeros((g.ny, g.nx), bool) for l in LAYERS}
        vm = np.zeros((g.ny, g.nx), bool)
        for l in LAYERS:
            if it.IsOnLayer(l):
                g.paint(tm[l], it, l, CLR + TW / 2)
                g.paint(vm, it, l, CLR + VD / 2)
        if any(tm[l].any() for l in LAYERS):
            out.append((it, tm, vm))
    return out


def dilate(m, n):
    out = m.copy()
    ys, xs = np.nonzero(m)
    if not len(ys):
        return out
    for dy in range(-n, n + 1):
        for dx in range(-n, n + 1):
            if dx * dx + dy * dy > n * n:
                continue
            yy, xx = ys + dy, xs + dx
            ok = (yy >= 0) & (yy < m.shape[0]) & (xx >= 0) & (xx < m.shape[1])
            out[yy[ok], xx[ok]] = True
    return out


def cluster(conn, item):
    """the copper connected to item (pcbnew connectivity)"""
    seen = {item.m_Uuid.AsString(): item}
    todo = [item]
    while todo:
        for y in conn.GetConnectedItems(todo.pop()):
            k = y.m_Uuid.AsString()
            if k not in seen:
                seen[k] = y
                todo.append(y)
    return seen


def at(board, x, y, net, lname):
    """the net's pad / track / via at a report position"""
    p = pcbnew.VECTOR2I(mm(x), mm(y))
    best = None
    for it in items(board):
        if isinstance(it, pcbnew.ZONE) or it.GetNetname() != net:
            continue
        if it.HitTest(p, mm(0.03)):
            if best is None or (lname in ('F.Cu', 'B.Cu') and board.GetLayerName(it.GetLayer()) == lname):
                best = it
    return best


def cells(board, g, its):
    m = {l: np.zeros((g.ny, g.nx), bool) for l in LAYERS}
    for it in its:
        for l in LAYERS:
            if it.IsOnLayer(l):
                if isinstance(it, pcbnew.ZONE):
                    if it.IsFilled():
                        g.paint_poly(m[l], it.GetFilledPolysList(l))
                else:
                    g.paint(m[l], it, l, -0.01)
    return m


def route(board, a, b, margin):
    net = a[2]
    xa, ya, xb, yb = float(a[0]), float(a[1]), float(b[0]), float(b[1])
    g = Grid(min(xa, xb) - margin, min(ya, yb) - margin, max(xa, xb) + margin, max(ya, yb) + margin)
    track, via, soft = blocked(board, g, net)
    soft = soft_masks(g, soft)
    stm = {l: np.zeros((g.ny, g.nx)) for l in LAYERS}
    svm = np.zeros((g.ny, g.nx))
    for it, tm, vm in soft:
        for l in LAYERS:
            stm[l][tm[l]] = SOFT_GND
        svm[vm] = SOFT_GND
    board.BuildConnectivity()
    conn = board.GetConnectivity()
    ia = at(board, xa, ya, net, a[3])
    if ia is None:
        return None
    ca = cluster(conn, ia)
    rest = [it for it in items(board) if it.GetNetname() == net and it.m_Uuid.AsString() not in ca
            and not (isinstance(it, pcbnew.ZONE) and it.GetIsRuleArea())]
    am, bm = cells(board, g, ca.values()), cells(board, g, rest)
    A = set((k, r, c) for k, l in enumerate(LAYERS) for r, c in zip(*np.nonzero(am[l])))
    Bset = set((k, r, c) for k, l in enumerate(LAYERS) for r, c in zip(*np.nonzero(bm[l]))) - A
    if not A or not Bset:
        return None
    own = {l: am[l] | bm[l] for l in LAYERS}
    # the search may enter own copper freely
    free = {k: ~track[l] | own[l] for k, l in enumerate(LAYERS)}
    vfree = ~via | np.logical_and.reduce([own[l] for l in LAYERS])
    dist = {s: 0 for s in A}
    prev = {}
    pq = [(0, s) for s in A]
    heapq.heapify(pq)
    moves = [(dr, dc, 1.0 if dr == 0 or dc == 0 else 1.414) for dr in (-1, 0, 1) for dc in (-1, 0, 1) if dr or dc]
    end = None
    while pq:
        d, s = heapq.heappop(pq)
        if d > dist.get(s, 1e18):
            continue
        if s in Bset:
            end = s
            break
        k, r, c = s
        nbs = []
        for dr, dc, w in moves:
            rr, cc = r + dr, c + dc
            if 0 <= rr < g.ny and 0 <= cc < g.nx and free[k][rr, cc]:
                # diagonal: both orthogonal neighbours free too (no corner cutting)
                if dr and dc and not (free[k][r + dr, c] and free[k][r, c + dc]):
                    continue
                nbs.append(((k, rr, cc), w * STEP_COST[k]))
        if vfree[r, c]:
            nbs.extend(((k2, r, c), VIA_COST) for k2 in range(len(LAYERS)) if k2 != k)
        for t, w in nbs:
            k2, r2, c2 = t
            w += svm[r2, c2] * 4 if t[0] != k else stm[LAYERS[k2]][r2, c2]
            nd = d + w
            if nd < dist.get(t, 1e18):
                dist[t] = nd
                prev[t] = s
                heapq.heappush(pq, (nd, t))
    if end is None:
        return None
    path = [end]
    while path[-1] in prev:
        path.append(prev[path[-1]])
    path.reverse()
    # remove the soft (GND) items the path runs through
    gone = []
    for it, tm, vm in soft:
        hit = False
        for i, (k, r, c) in enumerate(path):
            if tm[LAYERS[k]][r, c] and not own[LAYERS[k]][r, c]:
                hit = True
            if i and path[i - 1][0] != k and vm[r, c]:
                hit = True
            if hit:
                break
        if hit:
            gone.append(it)
    for it in gone:
        board.Delete(it)
    return net, g, path, len(gone)


def emit(board, net, g, path):
    nc = board.FindNet(net)
    n = 0
    # split into layer runs, vias at layer changes
    runs, cur = [], [path[0]]
    for s in path[1:]:
        if s[0] != cur[-1][0]:
            runs.append(cur)
            x, y = g.xy(s[1], s[2])
            v = pcbnew.PCB_VIA(board)
            v.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
            v.SetWidth(mm(VD))
            v.SetDrill(mm(VH))
            v.SetNet(nc)
            board.Add(v)
            n += 1
            cur = [s]
        else:
            cur.append(s)
    runs.append(cur)
    for run in runs:
        if len(run) < 2:
            continue
        # corners only
        pts = [run[0]]
        for i in range(1, len(run) - 1):
            d1 = (run[i][1] - run[i - 1][1], run[i][2] - run[i - 1][2])
            d2 = (run[i + 1][1] - run[i][1], run[i + 1][2] - run[i][2])
            if d1 != d2:
                pts.append(run[i])
        pts.append(run[-1])
        for p, q in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(board)
            t.SetStart(pcbnew.VECTOR2I(*(mm(v) for v in g.xy(p[1], p[2]))))
            t.SetEnd(pcbnew.VECTOR2I(*(mm(v) for v in g.xy(q[1], q[2]))))
            t.SetWidth(mm(TW))
            t.SetLayer(LAYERS[p[0]])
            t.SetNet(nc)
            board.Add(t)
            n += 1
    return n


def main(src, rpt, dst, *opts):
    global LAYERS, STEP_COST
    if '--in1' in opts:
        LAYERS, STEP_COST = LAYERS + (pcbnew.In1_Cu,), STEP_COST + (60,)
    board = pcbnew.LoadBoard(src)
    for a, b in opens(rpt):
        for margin in MARGINS:
            res = route(board, a, b, margin)
            if res is not None:
                break
        if res is None:
            print('mazefix: no path %s (%s,%s) - (%s,%s)' % (a[2], a[0], a[1], b[0], b[1]))
            continue
        net, g, path, gone = res
        print('mazefix: %s (%s,%s) - (%s,%s): %d items, %d router GND items removed'
              % (net, a[0], a[1], b[0], b[1], emit(board, net, g, path), gone))
    board.Save(dst)


if __name__ == '__main__':
    main(*sys.argv[1:])
