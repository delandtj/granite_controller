#!/usr/bin/env python3
"""Power copper for PM-19V-250W-B: pours on F / In1 / B, via fields, thin
backbones inside the pours (so the router sees one node), in a group
"power" that krt-route keeps; the CS / CSG Kelvin stubs and vias under R4,
in a group "hand".
Usage: power.py <in.kicad_pcb> <out.kicad_pcb>

Run after tools/place.py. In2 (solid GND) and the small-signal nets are left
to krt-route; GND on the outer layers to pcbkit gndpour after routing.

Current paths (13.2 A out, ~5.5 A in at 48 V):
  VIN   S1 (J1.1, all layers) -> B pour -> C1 and Q1's drain; F pour (top
        strip, left column, bottom strip) -> C2-C6; In1 under both; via row
        along the top edge
  SW    Q1 sources + Q2 drain (B) -> 14 vias -> F pour over L1's body ->
        L1 pad 1
  LSS   Q2 sources -> R4 pad 1 (B); NT2 under R4's body taps CS
  GND   R4 pad 2 (B) -> 10 vias -> In2; NT1 taps CSG. C1 (B) and the F cap
        band -> via row -> In2
  VOUT  L1 pad 2 -> F pour -> polymers C13-C15 (+ pads) -> strip over C13's
        GND pad -> S4; In1 pour (x > 132) and a B strip to S4 in parallel;
        B MLCCs C9-C12 through the via columns
"""
import sys

import pcbnew

GROUP, HAND = 'power', 'hand'
CLEAR = 0.2          # copper clearance
VIA = (0.6, 0.3)     # power via diameter, drill
SVIA = (0.5, 0.2)    # signal via (Kelvin taps)

B, F, IN1 = pcbnew.B_Cu, pcbnew.F_Cu, pcbnew.In1_Cu


def rect(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


# net, layer, priority, polygon (mm)
ZONES = [
    # input
    ('/VIN', B, 3, rect(100.4, 100.4, 117.0, 106.3)),
    ('GND', B, 5, rect(107.5, 104.35, 110.8, 107.05)),          # C1's GND pad and its vias
    ('/VIN', F, 3, [(100.4, 100.4), (116.9, 100.4), (116.9, 103.15), (107.0, 103.15), (107.0, 110.15),
                    (116.9, 110.15), (116.9, 112.4), (100.4, 112.4)]),
    ('GND', F, 5, rect(107.0, 104.4, 116.9, 108.75)),           # the cap GND band
    ('/VIN', IN1, 3, rect(100.4, 100.4, 117.2, 107.3)),
    # switch node
    ('/SW', B, 4, [(117.3, 100.5), (124.45, 100.5), (124.45, 108.95), (119.1, 108.95), (119.1, 105.25),
                   (117.3, 105.25)]),
    ('/SW', F, 4, rect(117.3, 100.5, 124.95, 109.4)),
    # low-side source to R4, R4's GND pad and its via field
    ('/LSS', B, 4, [(125.35, 100.5), (130.95, 100.5), (130.95, 102.3), (126.7, 102.3), (126.7, 105.15),
                    (125.35, 105.15)]),
    ('GND', B, 4, [(131.2, 100.5), (132.35, 100.5), (132.35, 110.0), (126.9, 110.0), (126.9, 106.55),
                   (131.2, 106.55)]),
    # output
    ('/VOUT', F, 3, [(128.8, 100.4), (158.0, 100.4), (158.0, 107.0), (146.95, 107.0), (146.95, 122.5),
                     (128.8, 122.5)]),
    ('GND', F, 5, rect(147.2, 102.95, 151.3, 106.8)),            # C13's GND pad island
    ('GND', F, 5, rect(147.2, 107.4, 158.0, 122.5)),             # C14 / C15 GND pads to S3
    ('/VOUT', IN1, 3, [(132.2, 100.4), (158.0, 100.4), (158.0, 113.9), (151.4, 113.9), (151.4, 122.5),
                       (132.2, 122.5)]),
    ('/VOUT', B, 3, [(145.8, 100.4), (158.0, 100.4), (158.0, 106.9), (146.35, 106.9), (146.35, 121.3),
                     (144.6, 121.3), (144.6, 106.1), (145.8, 106.1)]),
    ('GND', B, 4, [(146.65, 107.2), (151.4, 107.2), (151.4, 114.2), (158.0, 114.2), (158.0, 122.5),
                   (146.65, 122.5)]),
]


def grid(xs, ys):
    return [(x, y) for x in xs for y in ys]


def frange(a, b, step):
    out, v = [], a
    while v <= b + 1e-9:
        out.append(round(v, 3))
        v += step
    return out


# net, candidate via centres (kept only where they clear every pad and via)
VIAS = [
    ('/VIN', grid(frange(107.6, 116.6, 0.9), [100.95])),
    ('GND', grid(frange(108.0, 116.1, 0.9), [106.7])),
    ('/SW', grid([119.1], [100.95, 101.8, 102.65, 103.5, 104.35])),
    ('/SW', grid([122.4, 123.3, 124.2], [106.85, 107.75, 108.65])),
    ('GND', grid([127.4, 128.3, 129.2, 130.1, 131.0], [108.75, 109.6])),
    ('GND', grid([131.8], frange(101.0, 109.1, 0.9))),
    ('/VOUT', grid([133.15, 134.05], frange(101.0, 122.0, 0.9))),
    ('/VOUT', grid([140.6, 141.5], frange(106.3, 122.0, 0.9))),
    ('/VOUT', grid([144.2], [106.3, 107.2, 108.1, 108.9, 113.4, 114.3, 115.2, 116.1, 117.0, 120.6, 121.5])),
    ('/VOUT', grid([147.5, 148.4, 149.3, 150.2, 151.1], [101.2, 102.1])),
    ('GND', grid([147.8, 148.7, 149.6, 150.5], [106.05])),
    ('GND', grid([149.75, 150.65], frange(107.7, 122.1, 0.9))),
]

# Kelvin stubs: (net, [(x, y), ...] B.Cu track points, via at the last point)
KELVIN = [
    ('/CS', [(128.4, 103.05), (128.4, 103.7)]),
    ('/CSG', [(129.6, 105.85), (128.9, 105.15)]),
]


def mm(v):
    return pcbnew.FromMM(v)


def tomm(v):
    return pcbnew.ToMM(v)


def pad_clear(board, x, y, net, r):
    """True when a via at (x, y) clears every pad (all nets: no via in a pad)
    and every via and track already on the board."""
    for fp in board.GetFootprints():
        for p in fp.Pads():
            if not p.IsOnCopperLayer():
                continue
            bb = p.GetBoundingBox()
            gap = CLEAR if p.GetNetname() != net else 0.15
            if (tomm(bb.GetLeft()) - gap - r < x < tomm(bb.GetRight()) + gap + r and
                    tomm(bb.GetTop()) - gap - r < y < tomm(bb.GetBottom()) + gap + r):
                return False
    for t in board.GetTracks():
        if t.Type() == pcbnew.PCB_VIA_T:
            q = t.GetPosition()
            if (tomm(q.x) - x) ** 2 + (tomm(q.y) - y) ** 2 < (2 * r + CLEAR) ** 2:
                return False
    return True


def regroup(board, name):
    for g in list(board.Groups()):
        if g.GetName() == name:
            for it in list(g.GetItems()):
                board.Remove(it)
            board.Remove(g)
    grp = pcbnew.PCB_GROUP(board)
    grp.SetName(name)
    board.Add(grp)
    return grp


def add_via(board, grp, net, x, y, size):
    v = pcbnew.PCB_VIA(board)
    v.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
    v.SetWidth(mm(size[0]))
    v.SetDrill(mm(size[1]))
    v.SetNet(net)
    board.Add(v)
    grp.AddItem(v)


def add_track(board, grp, net, a, b, layer, width):
    t = pcbnew.PCB_TRACK(board)
    t.SetStart(pcbnew.VECTOR2I(mm(a[0]), mm(a[1])))
    t.SetEnd(pcbnew.VECTOR2I(mm(b[0]), mm(b[1])))
    t.SetWidth(mm(width))
    t.SetLayer(layer)
    t.SetNet(net)
    board.Add(t)
    grp.AddItem(t)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    grp = regroup(board, GROUP)
    hand = regroup(board, HAND)
    nets = board.GetNetsByName()

    for net, layer, prio, pts in ZONES:
        z = pcbnew.ZONE(board)
        z.SetLayer(layer)
        z.SetNet(nets[net])
        z.SetAssignedPriority(prio)
        z.SetLocalClearance(mm(CLEAR))
        z.SetMinThickness(mm(0.25))
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)
        z.SetIslandRemovalMode(pcbnew.ISLAND_REMOVAL_MODE_ALWAYS)
        ol = z.Outline()
        ol.NewOutline()
        for x, y in pts:
            ol.Append(mm(x), mm(y))
        board.Add(z)
        grp.AddItem(z)

    for net, pts in KELVIN:
        for a, b in zip(pts, pts[1:]):
            add_track(board, hand, nets[net], a, b, B, 0.2)
        add_via(board, hand, nets[net], pts[-1][0], pts[-1][1], SVIA)

    placed = {}
    for net, cands in VIAS:
        n = 0
        for x, y in cands:
            if not pad_clear(board, x, y, net, VIA[0] / 2):
                continue
            add_via(board, grp, nets[net], x, y, VIA)
            n += 1
        placed[net] = placed.get(net, 0) + n

    skipped = backbones(board, grp)
    pcbnew.ZONE_FILLER(board).Fill(board.Zones())
    board.Save(dst)
    print('zones', len(ZONES), 'vias', placed, 'backbone gaps', skipped)


def inside(poly, x, y):
    """Point in polygon (ray casting)."""
    n, c = len(poly), False
    for i in range(n):
        (x1, y1), (x2, y2) = poly[i], poly[(i + 1) % n]
        if (y1 > y) != (y2 > y) and x < (x2 - x1) * (y - y1) / (y2 - y1) + x1:
            c = not c
    return c


def backbones(board, grp, width=0.3):
    """Thin tracks inside each outer-layer pour joining its pads and vias
    (minimum spanning tree, straight or L legs that stay inside the polygon
    and clear other nets). The pour carries the current; these only tell
    the router (and DRC before a refill) that the pads are one node.
    (As PM-19V-250W rev A tools/power.py.)"""
    skipped = []
    others = []
    for fp in board.GetFootprints():
        for p in fp.Pads():
            if not p.IsOnCopperLayer():
                continue
            bb = p.GetBoundingBox()
            lay = ('TH' if p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH else
                   'B' if p.IsOnLayer(B) else 'F')
            others.append((tomm(bb.GetLeft()), tomm(bb.GetTop()), tomm(bb.GetRight()), tomm(bb.GetBottom()),
                           p.GetNetname(), lay, p))
    vias = [(tomm(t.GetPosition().x), tomm(t.GetPosition().y), t.GetNetname())
            for t in board.GetTracks() if t.Type() == pcbnew.PCB_VIA_T]
    nets = board.GetNetsByName()
    for net, layer, prio, pts in ZONES:
        if layer not in (B, F):
            continue
        side = 'B' if layer == B else 'F'
        nodes = [((o[0] + o[2]) / 2, (o[1] + o[3]) / 2) for o in others
                 if o[4] == net and o[5] in (side, 'TH') and inside(pts, (o[0] + o[2]) / 2, (o[1] + o[3]) / 2)]
        nodes += [(x, y) for x, y, n in vias if n == net and inside(pts, x, y)]
        if len(nodes) < 2:
            continue

        def clear(a, b):
            steps = max(2, int(((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2) ** 0.5 / 0.1))
            for i in range(steps + 1):
                x = a[0] + (b[0] - a[0]) * i / steps
                y = a[1] + (b[1] - a[1]) * i / steps
                if not inside(pts, x, y):
                    return False
                m = width / 2 + CLEAR
                for o in others:
                    if o[4] != net and o[5] in (side, 'TH') and o[0] - m < x < o[2] + m and o[1] - m < y < o[3] + m:
                        return False
                for vx, vy, vn in vias:
                    if vn != net and (vx - x) ** 2 + (vy - y) ** 2 < (VIA[0] / 2 + m) ** 2:
                        return False
            return True

        def leg(a, b):
            if clear(a, b):
                return [a, b]
            for c in ((a[0], b[1]), (b[0], a[1])):
                if clear(a, c) and clear(c, b):
                    return [a, c, b]
            return None

        done, todo = [nodes[0]], nodes[1:]
        while todo:
            cands = sorted(((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2, a, b) for a in done for b in todo)
            for _, a, b in cands:
                path = leg(a, b)
                if path:
                    for p, q in zip(path, path[1:]):
                        add_track(board, grp, nets[net], p, q, layer, width)
                    done.append(b)
                    todo.remove(b)
                    break
            else:
                skipped.append((net, side, len(todo)))
                break
    return skipped


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
