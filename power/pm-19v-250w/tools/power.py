#!/usr/bin/env python3
"""Power copper for PM-19V-250W: outer-layer pours and via fields for the
high-current nets, in a group "power" that krt-route keeps.
Usage: power.py <in.kicad_pcb> <out.kicad_pcb>

Run after tools/place.py. The inner planes (In1 VIN, In2 GND) and every
small-signal net are left to krt-route; the GND pours on the outer layers
to pcbkit gndpour after routing.

Current paths (10.5 A out, ~4 A in at 54 V):
  VIN   J1.1 -> B.Cu pour -> C1-C4 -> Q1 drain; In1 -> C5-C8 -> Q3 drain
  SW1   Q1 source + Q2 drain (B.Cu) -> 8 vias -> L1 pad 1 (F.Cu)
  SW2   Q4 drain + Q3 source (B.Cu) -> 12 vias -> L2 pad 1 (F.Cu)
  SNSx  L1/L2 pad 2 -> F.Cu pour -> R9/R10 pad 1
  VOUT  R9/R10 pad 2 -> F.Cu pour -> C20-C25 -> J1.8
  GND   Q2/Q4 sources -> via fields -> In2 plane -> caps, J1.3/J1.4
"""
import sys

import pcbnew

GROUP = 'power'
CLEAR = 0.2          # copper clearance
VIA = (0.6, 0.3)     # power via diameter, drill

B, F = pcbnew.B_Cu, pcbnew.F_Cu

# net, layer, priority, polygon (mm)
ZONES = [
    # bottom edge dips to 108.3 left of C27 so the pour wraps under C3.2 to
    # J1.1 (the C1/C3 GND pads leave no room between them); R1.2 and D1.1
    # sit just below 108.3
    ('/VIN', B, 3, [(102.4, 100.3), (120.6, 100.3), (120.6, 107.0), (108.6, 107.0), (108.6, 108.3),
                    (102.4, 108.3)]),
    ('/VIN', B, 3, [(135.2, 114.6), (150.6, 114.6), (150.6, 122.6), (135.2, 122.6)]),
    ('/SW1', B, 4, [(121.0, 100.3), (127.9, 100.3), (127.9, 106.6), (126.0, 106.6), (126.0, 110.0),
                    (123.9, 110.0), (123.9, 106.6), (122.6, 106.6), (122.6, 105.3), (121.0, 105.3)]),
    ('/SW2', B, 4, [(127.9, 113.3), (132.7, 113.3), (132.7, 117.6), (135.0, 117.6), (135.0, 122.6),
                    (127.9, 122.6)]),
    ('/SW1', F, 4, [(121.4, 106.7), (125.95, 106.7), (125.95, 116.2), (121.4, 116.2)]),
    ('/SW2', F, 4, [(126.25, 106.7), (132.55, 106.7), (132.55, 116.2), (126.25, 116.2)]),
    ('/SNS1P', F, 4, [(114.8, 100.3), (140.1, 100.3), (140.1, 106.4), (117.7, 106.4), (117.7, 116.2),
                      (114.8, 116.2)]),
    ('/SNS2P', F, 4, [(132.8, 106.7), (136.0, 106.7), (136.0, 116.4), (140.2, 116.4), (140.2, 122.6),
                      (132.8, 122.6)]),
    ('/VOUT', F, 3, [(140.5, 100.3), (154.2, 100.3), (154.2, 122.6), (140.5, 122.6)]),
]

# net, candidate via centres (kept only where they clear every pad and via)
def grid(xs, ys):
    return [(x, y) for x in xs for y in ys]


VIAS = [
    ('/SW1', grid([124.6, 125.5], [107.0, 107.9, 108.8, 109.7])),
    ('/SW2', grid([129.2, 130.1, 131.0, 131.9], [113.6, 114.5, 115.4])),
    ('/VIN', grid([136.0, 137.0, 138.0, 139.0, 140.0], [115.25])),
    ('GND', grid([130.45, 131.35], [101.4, 102.3, 103.2, 104.1, 105.0])),
    ('GND', grid([125.4], [118.2, 119.1, 120.0, 120.9, 121.8])),
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


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Remove(it)
            board.Remove(g)
    grp = pcbnew.PCB_GROUP(board)
    grp.SetName(GROUP)
    board.Add(grp)
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

    placed = {}
    for net, cands in VIAS:
        n = 0
        for x, y in cands:
            if not pad_clear(board, x, y, net, VIA[0] / 2):
                continue
            v = pcbnew.PCB_VIA(board)
            v.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
            v.SetWidth(mm(VIA[0]))
            v.SetDrill(mm(VIA[1]))
            v.SetNet(nets[net])
            board.Add(v)
            grp.AddItem(v)
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
    """Thin tracks inside each pour joining its pads and vias (minimum
    spanning tree, straight or L-shaped legs that stay inside the polygon
    and clear other nets). The pour carries the current; these only tell
    the router (and DRC before a refill) that the pads are one node."""
    skipped = []
    others = []    # (x0, y0, x1, y1, net, layer) of every pad
    for fp in board.GetFootprints():
        for p in fp.Pads():
            if not p.IsOnCopperLayer():
                continue      # paste-only apertures (TDSON-8)
            bb = p.GetBoundingBox()
            lay = 'TH' if p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH else ('B' if fp.IsFlipped() else 'F')
            others.append((tomm(bb.GetLeft()), tomm(bb.GetTop()), tomm(bb.GetRight()), tomm(bb.GetBottom()),
                           p.GetNetname(), lay, p))
    vias = [(tomm(t.GetPosition().x), tomm(t.GetPosition().y), t.GetNetname())
            for t in board.GetTracks() if t.Type() == pcbnew.PCB_VIA_T]
    nets = board.GetNetsByName()
    for net, layer, prio, pts in ZONES:
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

        # Prim's MST over the nodes; take the shortest edge that has a clear leg
        done, todo = [nodes[0]], nodes[1:]
        while todo:
            cands = sorted(((a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2, a, b) for a in done for b in todo)
            for _, a, b in cands:
                path = leg(a, b)
                if path:
                    for p, q in zip(path, path[1:]):
                        t = pcbnew.PCB_TRACK(board)
                        t.SetStart(pcbnew.VECTOR2I(mm(p[0]), mm(p[1])))
                        t.SetEnd(pcbnew.VECTOR2I(mm(q[0]), mm(q[1])))
                        t.SetWidth(mm(width))
                        t.SetLayer(layer)
                        t.SetNet(nets[net])
                        board.Add(t)
                        grp.AddItem(t)
                    done.append(b)
                    todo.remove(b)
                    break
            else:
                skipped.append((net, side, len(todo)))
                break
    return skipped


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
