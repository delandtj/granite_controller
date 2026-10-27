#!/usr/bin/env python3
"""Power copper for PM-19V-250W: outer-layer pours, an In1 VIN feed and via
fields for the high-current nets, in a group "power" that krt-route keeps.
Usage: power.py <in.kicad_pcb> <out.kicad_pcb>

Run after tools/place.py. In2 (solid GND) and every small-signal net are
left to krt-route; the GND pours on the outer layers to pcbkit gndpour
after routing.

Current paths (13.2 A out, ~5.5 A in at 48 V):
  VIN   S1 (J1.1, all layers) -> B.Cu pour -> C1-C4 -> Q1 drain;
        S1 -> In1 strip along the top edge and down the right -> vias
        -> B.Cu pour -> C5-C8 -> Q3 drain
  SW2   Q3 sources + Q4 drain (B.Cu, top pair) -> 12 vias -> F.Cu beside L2 pad 1
  SW1   Q2 drain + Q1 sources (B.Cu, bottom pair) -> 8 vias -> F.Cu beside L1 pad 1
  SNS1  L1 pad 2 -> F.Cu pour along the bottom edge -> R9 pad 1 (output end)
  SNS2  L2 pad 2 -> short F.Cu bridge -> R10 pad 1 (input end)
  VOUT  R10 pad 2 -> F.Cu riser and strip along the top edge -> output end;
        R9 pad 2 -> F.Cu band -> C20-C23 round their GND island -> S4
        (J1.4); C24/C25 (B.Cu) through a via column right of C20/C22.
        The comparators sit in a hole in the VOUT pour beside J2.
  Sense taps are Kelvin: the pours stop at the inner pad edges of R9/R10,
  the sense traces leave from under the resistor bodies.
  GND   Q2/Q4 sources, C1-C8 (GND islands under the cap bodies), C24 ->
        via fields -> In2 plane -> S2/S3 (J1.2/J1.3)
"""
import sys

import pcbnew

GROUP = 'power'
CLEAR = 0.2          # copper clearance
VIA = (0.6, 0.3)     # power via diameter, drill

B, F, IN1 = pcbnew.B_Cu, pcbnew.F_Cu, pcbnew.In1_Cu


def rect(x0, y0, x1, y1):
    return [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]


# net, layer, priority, polygon (mm)
ZONES = [
    # phase 1 input: round the bottom of C1-C4 (their GND islands leave no
    # path between the rows), up to the VIN pads and Q1's drain
    ('/VIN', B, 3, [(100.4, 100.4), (122.2, 100.4), (122.2, 106.35), (117.2, 106.35), (117.2, 108.2),
                    (100.4, 108.2)]),
    # phase 2 input: over the top of C5-C8, Q3's drain
    ('/VIN', B, 3, [(135.6, 113.9), (147.5, 113.9), (147.5, 122.7), (135.6, 122.7)]),
    # In1 feed from S1 to phase 2
    # (bottom edge at 104.2: RUN runs along under it)
    ('/VIN', IN1, 3, [(100.4, 100.4), (144.0, 100.4), (144.0, 116.3), (137.9, 116.3), (137.9, 104.2),
                      (100.4, 104.2)]),
    # GND islands under the input caps, one via each under the body
    ('GND', B, 5, rect(107.45, 100.5, 110.05, 106.75)),      # C1/C3
    ('GND', B, 5, rect(112.3, 100.5, 114.9, 106.75)),        # C2/C4
    ('GND', B, 5, rect(142.85, 116.1, 145.45, 122.35)),      # C5/C7
    ('GND', B, 5, rect(147.7, 116.1, 150.3, 122.35)),        # C6/C8
    # Q2 sources (BG1 escapes below the left part), Q4 sources, C24 GND
    ('GND', B, 4, [(130.5, 100.4), (137.4, 100.4), (137.4, 106.9), (132.0, 106.9), (132.0, 105.3),
                   (130.5, 105.3)]),
    ('GND', B, 4, rect(124.1, 117.7, 127.5, 122.7)),
    ('GND', B, 4, rect(135.7, 107.2, 140.6, 113.5)),
    # switch nodes: TG1 escapes left of the SW1 tail, TG2 right of SW2's
    ('/SW2', B, 4, [(123.0, 100.4), (129.8, 100.4), (129.8, 106.6), (128.3, 106.6), (128.3, 110.7),
                    (125.4, 110.7), (125.4, 106.6), (124.5, 106.6), (124.5, 105.3), (123.0, 105.3)]),
    ('/SW1', B, 4, [(128.2, 122.7), (134.9, 122.7), (134.9, 117.75), (133.5, 117.75), (133.5, 116.4),
                    (134.4, 116.4), (134.4, 112.2), (132.2, 112.2), (132.2, 116.4), (128.2, 116.4)]),
    ('/SW2', F, 4, rect(122.2, 104.9, 128.4, 118.2)),
    ('/SW1', F, 4, [(128.9, 104.9), (132.3, 104.9), (132.3, 107.2), (134.5, 107.2), (134.5, 118.2),
                    (128.9, 118.2)]),
    # sense nodes: L2 pad 2 and the bridge to R10 pad 1; L1 pad 2 round to
    # R9 pad 1, clear of the space under R9's body (Kelvin vias)
    ('/SNS2P', F, 4, [(106.6, 111.15), (111.6, 111.15), (111.6, 104.65), (115.2, 104.65), (115.2, 118.2),
                      (111.6, 118.2), (111.6, 112.8), (106.6, 112.8)]),
    ('/SNS1P', F, 4, [(139.4, 118.0), (144.3, 118.0), (144.3, 120.85), (148.2, 120.85), (148.2, 122.6),
                      (139.4, 122.6)]),
    # output: R10 pad 2 -> riser left of L2 -> strip along the top edge ->
    # S4 and C20-C23; R9 pad 2 -> band under the comparators -> C20/C22;
    # the GND island between the MLCC columns runs down to S3
    ('/VOUT', F, 3, [(107.1, 100.4), (158.3, 100.4), (158.3, 113.95), (155.4, 113.95), (155.4, 107.1),
                     (151.0, 107.1), (151.0, 116.5), (144.0, 116.5), (144.0, 114.4), (148.55, 114.4),
                     (148.55, 104.6), (111.6, 104.6), (111.6, 106.75), (107.1, 106.75)]),
    ('GND', F, 5, rect(151.4, 107.4, 155.0, 114.6)),
    ('/VOUT', B, 4, rect(144.5, 107.3, 150.9, 113.4)),
]

# net, candidate via centres (kept only where they clear every pad and via)
def grid(xs, ys):
    return [(x, y) for x in xs for y in ys]


# pitch >= 0.85: two vias closer than 2 r + CLEAR (0.8) are refused
VIAS = [
    ('/SW2', grid([126.1, 126.95, 127.8], [107.3, 108.2, 109.1, 110.0])),
    ('/SW1', grid([132.7, 133.6], [112.6, 113.5, 114.4, 115.3])),
    # 107.7 row: stepping stones under the C1-C4 GND islands for the backbone
    # (none at 116.5: RUN runs up there on F to its In1 via between C4 and Q3)
    ('/VIN', grid([138.25, 139.1], [114.6, 115.5]) + grid([143.4], [114.5, 115.4]) +
     grid([107.2, 112.2, 115.6], [107.7])),
    # input cap GND islands: under the cap bodies (C2/C4: not at 101.95,
    # it would cut the VOUT strip and its backbone along the top edge)
    ('GND', grid([109.6], [101.95, 103.6, 105.3]) + grid([114.45], [103.6])),
    ('GND', grid([143.5], [117.55, 119.2, 120.9]) + grid([148.4], [117.55, 119.2, 120.9])),
    # Q2 sources: right of L2 pad 1, below the SNS1P strip
    ('GND', grid([132.9, 133.8, 134.7, 135.6], [105.4, 106.3])),
    # bottom low side (Q2) sources; rows 118.6 and 122.2 left free for BG1's via and TG1's In1 lane
    ('GND', grid([124.6, 125.45], [119.5, 120.4, 121.3])),
    ('GND', grid([136.3, 137.2], [108.0, 109.2, 110.4, 111.6, 112.8])),
    ('/VOUT', grid([150.4], [108.0, 108.9, 109.8, 110.7, 111.6, 112.5])),
]

# boxes (layer, x0, y0, x1, y1) the backbones keep out of: RUN's via
# between C4 and Q3's drain (tools/route_input.py)
BACKBONE_KEEP = [(B, 116.55, 104.45, 117.45, 105.55)]


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
            lay = ('TH' if p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH else
                   'B' if p.IsOnLayer(B) else 'F')      # by pad layer: J2's pads are B.Cu in an F footprint
            others.append((tomm(bb.GetLeft()), tomm(bb.GetTop()), tomm(bb.GetRight()), tomm(bb.GetBottom()),
                           p.GetNetname(), lay, p))
    vias = [(tomm(t.GetPosition().x), tomm(t.GetPosition().y), t.GetNetname())
            for t in board.GetTracks() if t.Type() == pcbnew.PCB_VIA_T]
    nets = board.GetNetsByName()
    for net, layer, prio, pts in ZONES:
        if layer not in (B, F):
            continue                  # In1 feed: joined by S1 and the vias
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
                if any(k[0] == layer and k[1] < x < k[3] and k[2] < y < k[4] for k in BACKBONE_KEEP):
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
