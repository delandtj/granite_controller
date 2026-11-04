#!/usr/bin/env python3
"""Tidy the routers' copper: fewer, longer track segments, so a trace can be dragged or rerouted by hand.
Usage: mergetracks.py <in.kicad_pcb> <out.kicad_pcb> [--drc <project.kicad_pro>]

KRT and tools/mazefix.py draw an angled run as a 45-degree staircase of 0.1-0.4 mm pieces. Here each run
of the router's (ungrouped) segments between fixed points (pad, via, branch, width change) is redrawn
greedily: from a point, the farthest later point reachable with one straight segment (0 / 45 / 90 degrees)
or two (straight + diagonal, either order), checked for

- clearance: other-net tracks (GND pads 0.3 mm more, for the thermal spokes), vias, pads, holes and zone outlines on the layer (not the GND pour, which
  yields), keepout areas;
- connection: every same-net pad, via or track the replaced pieces connected to, the new ones connect to;
  no new piece lies wholly on another same-net track.

F / B only (the In1 hops stay). Zero-length pieces go first. Hand copper (grouped, tools/route_unit.py) is not touched.

With --drc (the project file, for its net classes) the result is checked: DRC on the input and the output,
and the router's runs within 1.5 mm of a copper violation the input does not have stay as drawn (what KiCad
counts as connected or a starved thermal is not all modelled here); up to four rounds, then the input as is.
"""
import math
import os
import re
import shutil
import subprocess
import sys
import tempfile
from collections import defaultdict

import pcbnew

V = pcbnew.VECTOR2I
CELL = 2000000      # nm, spatial hash cell
KEEP_R = 1500000    # nm, around a new DRC violation the router's copper stays as drawn
ROUNDS = 4
IGNORE = ('silk', 'lib_', 'courtyard', 'pth_inside_courtyard', 'npth_inside_courtyard')
SPOKE = 300000      # nm, extra room by GND pads (their thermal spokes into the pour)


def key(p):
    return (p.x, p.y)


def seg_dist(a, b, c, d):
    """Distance between segments ab and cd (points as (x, y))."""
    def pt(p, a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]
        l2 = dx * dx + dy * dy
        t = 0.0 if l2 == 0 else max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2))
        return math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy)

    def side(p, q, r):
        return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    if a != b and c != d:
        d1, d2, d3, d4 = side(a, b, c), side(a, b, d), side(c, d, a), side(c, d, b)
        if d1 and d2 and d3 and d4 and ((d1 > 0) != (d2 > 0)) and ((d3 > 0) != (d4 > 0)):
            return 0.0
    return min(pt(a, c, d), pt(b, c, d), pt(c, a, b), pt(d, a, b))


def octo(a, b):
    """True if ab runs at a multiple of 45 degrees."""
    dx, dy = abs(b[0] - a[0]), abs(b[1] - a[1])
    return dx == 0 or dy == 0 or dx == dy


def paths(a, b):
    """Candidate 45-degree paths a -> b: one segment, or straight + diagonal in either order."""
    if octo(a, b):
        return [[a, b]]
    dx, dy = b[0] - a[0], b[1] - a[1]
    m = min(abs(dx), abs(dy))
    sx, sy = (1 if dx > 0 else -1), (1 if dy > 0 else -1)
    diag = (a[0] + sx * m, a[1] + sy * m)              # diagonal first
    straight = (b[0] - sx * m, b[1] - sy * m)          # straight first
    return [[a, diag, b], [a, straight, b]]


class Geometry:
    """Copper on F / B and what the router's tracks must keep clear of, in a spatial hash."""

    def __init__(self, board):
        self.hole_cl = board.GetDesignSettings().m_HoleClearance
        self.gnd = board.GetNetcodeFromNetname('GND')
        self.grid = defaultdict(list)        # (layer, cx, cy) -> obstacles
        self.obs = []                        # [kind, net, layer, geometry, clearance, alive, item]
        for t in board.Tracks():
            self.add_track(t)
        for p in board.GetPads():
            for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
                if p.IsOnLayer(layer):
                    self.add('pad', p.GetNetCode(), layer, p.GetEffectiveShape(layer), p.GetOwnClearance(layer),
                             p.GetBoundingBox(), p)
                if p.HasHole():
                    self.add('hole', -1, layer, p.GetEffectiveHoleShape(), self.hole_cl, p.GetBoundingBox(), p)
        for z in board.Zones():
            for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
                if not z.IsOnLayer(layer):
                    continue
                if z.GetIsRuleArea():
                    if z.GetDoNotAllowTracks():
                        self.add('zone', -1, layer, z.Outline(), 0, z.GetBoundingBox(), z)
                elif z.GetNetname() != 'GND':       # the GND pour yields (refilled after); other pours do not
                    self.add('zone', z.GetNetCode(), layer, z.Outline(), z.GetLocalClearance() or 0,
                             z.GetBoundingBox(), z)

    @staticmethod
    def cells(layer, x0, y0, x1, y1):
        for cx in range(int(x0 // CELL), int(x1 // CELL) + 1):
            for cy in range(int(y0 // CELL), int(y1 // CELL) + 1):
                yield (layer, cx, cy)

    def add(self, kind, net, layer, geom, cl, box, item):
        o = [kind, net, layer, geom, cl, True, item]
        for c in self.cells(layer, box.GetLeft(), box.GetTop(), box.GetRight(), box.GetBottom()):
            self.grid[c].append(o)
        self.obs.append(o)
        return o

    def add_track(self, t):
        a, b = key(t.GetStart()), key(t.GetEnd())
        via = t.Type() == pcbnew.PCB_VIA_T
        w = (t.GetWidth(pcbnew.F_Cu) if via else t.GetWidth()) / 2
        box = pcbnew.BOX2I(V(int(min(a[0], b[0]) - w), int(min(a[1], b[1]) - w)),
                           V(int(abs(a[0] - b[0]) + 2 * w), int(abs(a[1] - b[1]) + 2 * w)))
        if via:
            for layer in (pcbnew.F_Cu, pcbnew.B_Cu):
                self.add('via', t.GetNetCode(), layer, (a, a, w), t.GetOwnClearance(layer), box, t)
                self.add('hole', -1, layer, (a, a, t.GetDrillValue() / 2), self.hole_cl, box, t)
            return None
        if t.Type() != pcbnew.PCB_TRACE_T:
            return None
        return self.add('track', t.GetNetCode(), t.GetLayer(), (a, b, w), t.GetOwnClearance(t.GetLayer()), box, t)

    def near(self, layer, a, b, margin):
        seen = set()
        for c in self.cells(layer, min(a[0], b[0]) - margin, min(a[1], b[1]) - margin,
                            max(a[0], b[0]) + margin, max(a[1], b[1]) + margin):
            for o in self.grid.get(c, ()):
                if o[5] and id(o) not in seen:
                    seen.add(id(o))
                    yield o

    @staticmethod
    def hits(o, a, b, w, gap):
        """Copper segment ab (half width w) within gap of obstacle o."""
        geom = o[3]
        if isinstance(geom, tuple):
            c, d, r = geom
            return seg_dist(a, b, c, d) < w + r + gap
        return geom.Collide(pcbnew.SHAPE_SEGMENT(V(*a), V(*b), int(2 * w)), int(gap))

    def clear(self, net, layer, pts, w, cl, skip):
        for a, b in zip(pts, pts[1:]):
            for o in self.near(layer, a, b, w + cl + 1000000):
                if id(o) in skip or o[1] == net:
                    continue
                gap = max(cl, o[4]) + (SPOKE if o[0] == 'pad' and o[1] == self.gnd else 0)
                if self.hits(o, a, b, w, gap):
                    return False
        return True

    def touched(self, net, layer, pts, w, skip):
        """Same-net anchors the copper pts connects to the way KiCad counts it: a pad its copper overlaps; a
        track or via whose end / centre lies on the copper, or that one of the path's points lies on."""
        out = set()
        for a, b in zip(pts, pts[1:]):
            for o in self.near(layer, a, b, w + 1000000):
                if id(o) in skip or o[1] != net or o[0] not in ('track', 'via', 'pad'):
                    continue
                if o[0] == 'pad':
                    if self.hits(o, a, b, w, 0):
                        out.add(id(o))
                    continue
                c, d, r = o[3]
                if (seg_dist(a, b, c, c) <= w or seg_dist(a, b, d, d) <= w
                        or seg_dist(a, a, c, d) <= r or seg_dist(b, b, c, d) <= r):
                    out.add(id(o))
        return out

    def covered(self, net, layer, pts, skip):
        """A segment of pts lies wholly on another same-net track (KiCad flags it as dangling)."""
        for a, b in zip(pts, pts[1:]):
            for o in self.near(layer, a, b, 1000000):
                if id(o) not in skip and o[1] == net and o[0] == 'track':
                    c, d, r = o[3]
                    if seg_dist(a, a, c, d) <= r and seg_dist(b, b, c, d) <= r:
                        return True
        return False


def chains(segs):
    """Runs of the router's segments through points where exactly two of them meet (same width)."""
    ends = defaultdict(list)
    for i, s in enumerate(segs):
        for p in (s.GetStart(), s.GetEnd()):
            ends[(s.GetNetCode(), s.GetLayer(), key(p))].append(i)

    def far(i, p):
        s = segs[i]
        return key(s.GetEnd()) if key(s.GetStart()) == p else key(s.GetStart())

    def passable(k):
        at = ends[k]
        return (len(at) == 2 and segs[at[0]].GetWidth() == segs[at[1]].GetWidth()
                and segs[at[0]].GetParentGroup() is None and segs[at[1]].GetParentGroup() is None)
    used = set()
    out = []
    for i, s in enumerate(segs):
        if i in used or s.GetParentGroup() is not None:
            continue
        net, layer = s.GetNetCode(), s.GetLayer()
        # walk back to a fixed end, then forward
        cur, p = i, key(s.GetStart())
        loop = False
        while passable((net, layer, p)):
            nxt = [j for j in ends[(net, layer, p)] if j != cur][0]
            if nxt == i:
                loop = True
                break
            cur, p = nxt, far(nxt, p)
        if loop:
            continue
        pts, idx = [p], []
        while True:
            used.add(cur)
            idx.append(cur)
            p = far(cur, p)
            pts.append(p)
            if not passable((net, layer, p)):
                break
            cur = [j for j in ends[(net, layer, p)] if j != cur][0]
        out.append((idx, pts))
    return out


def straighten(geo, vias, net, layer, w, cl, pts, skip):
    """Greedy: from each point the farthest later one a clear, connection-keeping path reaches."""
    new = [pts[0]]
    i = 0
    while i < len(pts) - 1:
        best = None
        for j in range(len(pts) - 1, i + 1, -1):
            if any((pts[k], net) in vias for k in range(i + 1, j)):
                continue
            old = geo.touched(net, layer, pts[i:j + 1], w, skip)
            for cand in paths(pts[i], pts[j]):
                if (geo.clear(net, layer, cand, w, cl, skip) and not geo.covered(net, layer, cand, skip)
                        and old <= geo.touched(net, layer, cand, w, skip)):
                    best = (j, cand)
                    break
            if best:
                break
        j, cand = best or (i + 1, [pts[i], pts[i + 1]])
        new.extend(cand[1:])
        i = j
    # drop the points that ended up co-linear
    out = [new[0]]
    for p in new[1:]:
        if len(out) >= 2:
            a, m = out[-2], out[-1]
            if ((m[0] - a[0]) * (p[1] - m[1]) == (m[1] - a[1]) * (p[0] - m[0])
                    and (m[0] - a[0]) * (p[0] - m[0]) + (m[1] - a[1]) * (p[1] - m[1]) > 0 and (m, net) not in vias):
                out[-1] = p
                continue
        out.append(p)
    return out


def tidy(src, dst, keep):
    """One pass; chains with a point within KEEP_R of a point in keep stay as drawn. Returns (before, after)."""
    board = pcbnew.LoadBoard(src)
    tr = board.Tracks()
    vias = {(key(t.GetPosition()), t.GetNetCode()) for t in tr if t.Type() == pcbnew.PCB_VIA_T}
    zero = [t for t in tr if t.Type() == pcbnew.PCB_TRACE_T and t.GetParentGroup() is None
            and key(t.GetStart()) == key(t.GetEnd())]
    for t in zero:
        board.Delete(t)
    tr = board.Tracks()
    segs = [t for t in tr if t.Type() == pcbnew.PCB_TRACE_T and t.GetLayer() in (pcbnew.F_Cu, pcbnew.B_Cu)]
    before = len(segs) + len(zero)
    geo = Geometry(board)
    track_obs = {o[6].m_Uuid.AsString(): o for o in geo.obs if o[0] == 'track'}
    saved = 0
    for idx, pts in chains(segs):
        if len(idx) < 2 or any(math.hypot(p[0] - q[0], p[1] - q[1]) < KEEP_R for p in pts for q in keep):
            continue
        s0 = segs[idx[0]]
        net, layer, w = s0.GetNetCode(), s0.GetLayer(), s0.GetWidth() / 2
        mine = [track_obs[segs[i].m_Uuid.AsString()] for i in idx]
        skip = {id(o) for o in mine}
        out = straighten(geo, vias, net, layer, w, s0.GetOwnClearance(layer), pts, skip)
        if len(out) - 1 >= len(idx):
            continue
        for o in mine:
            o[5] = False
            board.Delete(o[6])
        for a, b in zip(out, out[1:]):
            t = pcbnew.PCB_TRACK(board)
            t.SetStart(V(*a))
            t.SetEnd(V(*b))
            t.SetWidth(int(2 * w))
            t.SetLayer(layer)
            t.SetNetCode(net)
            board.Add(t)
            geo.add_track(t)
        saved += len(idx) - (len(out) - 1)
    board.Save(dst)
    return before, before - len(zero) - saved


def drc(pcb, pro):
    """Copper violations of a DRC run: [(type, [(x, y) nm, ...])] (silk, courtyard, library ones left out)."""
    base = os.path.splitext(pcb)[0]
    shutil.copy(pro, base + '.kicad_pro')
    subprocess.run(['kicad-cli', 'pcb', 'drc', '--refill-zones', '--severity-all', '-o', base + '.rpt', pcb],
                   capture_output=True)
    out = []
    for block in re.split(r'\n(?=\[)', open(base + '.rpt').read()):
        m = re.match(r'\[(\w+)\]', block)
        if m and not m.group(1).startswith(IGNORE):
            out.append((m.group(1), [(round(float(x) * 1e6), round(float(y) * 1e6))
                                     for x, y in re.findall(r'@\(([\d.-]+) mm, ([\d.-]+) mm\)', block)]))
    return out


def main(src, dst, pro=None):
    if not pro:
        before, after = tidy(src, dst, [])
        print('mergetracks: %d -> %d segments on F / B' % (before, after))
        return
    # with DRC: leave the chains alone near any violation the input board does not have, and redo
    work = os.path.join(tempfile.mkdtemp(prefix='mergetracks-'), 'b.kicad_pcb')
    shutil.copy(src, work)
    old = [(t, sorted(pts)) for t, pts in drc(work, pro)]
    keep = []
    for rnd in range(1, ROUNDS + 1):
        before, after = tidy(src, dst, keep)
        shutil.copy(dst, work)
        new = [(t, pts) for t, pts in drc(work, pro)
               if (t, sorted(pts)) not in old]
        print('mergetracks round %d: %d -> %d segments on F / B, %d new violations' % (rnd, before, after, len(new)))
        if not new:
            return
        keep += [p for _, pts in new for p in pts]
    shutil.copy(src, dst)
    print('mergetracks: still new violations, board left as it was')


if __name__ == '__main__':
    args = sys.argv[1:]
    pro = None
    if '--drc' in args:
        i = args.index('--drc')
        pro = args[i + 1]
        del args[i:i + 2]
    main(args[0], args[1], pro)
