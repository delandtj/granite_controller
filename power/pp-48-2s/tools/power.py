#!/usr/bin/env python3
"""High-current copper for the PP-48-2S plane, group "power" (kept by
krt-route; the GND zones from gndpour are moved into the same group so KRT
treats GND as a plane net). Run after place.py and gndpour:
    power.py <in.kicad_pcb> <out.kicad_pcb>

VBUS (8-9 A on the 2-slot plane) lives on B.Cu: a trunk under the input
and buck caps (y 108), the artery between the brick sockets (x 169) and a
bar under the four shunts (y 150); each shunt's VBUS pad reaches the bar
through two vias beside the pad. The slot OUT (4 A) runs on F.Cu from the
FET source up the channel beside the artery to the brick's +VIN pin; NODE
(4 A, 19 V) runs down the outer edge to the Mini-Fit on F.Cu and B.Cu.
Endpoints are pad references (REF.PAD), resolved from the placed board.
"""
import os
import sys

import pcbnew

GROUP = 'power'
VIA = (0.6, 0.3)

# (net, layer, width, points); a point is (x, y) or "REF.PAD"
TRACKS = [
    ('/VBUS_IN', 'F.Cu', 3.0, ['J1.1', (110.0, 115.0), (110.0, 106.5), 'F1.1']),
    ('/VBUS', 'F.Cu', 3.0, ['F1.2', (119.8, 106.5), (119.8, 112.5), 'D1.2']),
    ('/VBUS', 'F.Cu', 2.5, [(119.8, 108.0), (123.5, 108.0)]),
    ('/VBUS', 'F.Cu', 2.0, [(123.5, 108.0), (123.5, 106.0), 'C1.1']),
    ('/VBUS', 'F.Cu', 2.0, [(123.5, 108.0), (123.5, 110.5), 'C2.1']),
    ('/VBUS', 'B.Cu', 5.0, [(123.5, 108.0), (177.0, 108.0)]),          # trunk
    ('/VBUS', 'F.Cu', 1.5, ['C7.1', (133.0, 109.5)]),                   # buck input caps tap the trunk through a via
    ('/VBUS', 'F.Cu', 1.5, ['C14.1', (177.0, 109.5)]),
    ('/VBUS', 'B.Cu', 6.0, [(169.0, 108.0), (169.0, 150.0)]),          # artery
    ('/VBUS', 'B.Cu', 5.5, [(152.7, 150.25), (187.3, 150.25)]),        # bar under the shunts, via to via
    # slot 2 (right of the artery)
    ('/S2_SENSE', 'F.Cu', 2.5, ['R201.2', (178.5, 158.5), (181.55, 158.5), 'Q201.5']),
    ('/S2_SENSE', 'F.Cu', 2.5, ['R202.2', (184.5, 158.5), (181.55, 158.5)]),
    ('/S2_OUT', 'F.Cu', 2.5, ['Q201.2', (174.5, 161.37), (174.5, 129.38), 'X201.1']),
    ('/S2_OUT', 'F.Cu', 2.5, [(174.5, 161.37), (174.5, 164.5)]),             # down to the B.Cu via
    # OUT to the TVS and cap: on B.Cu under the slot (F.Cu is taken by SENSE and the ICs)
    ('/S2_OUT', 'B.Cu', 2.0, [(174.5, 164.5), (203.2, 164.5)]),
    ('/S2_OUT', 'F.Cu', 1.5, [(203.2, 164.5), (203.2, 162.0), (203.2, 'C202.1'), 'C202.1']),   # x 203.2 lands on D201.2 and clears the latch escapes
    ('/S2_NODE', 'F.Cu', 2.5, ['X201.8', (232.9, 129.38), (232.9, 181.5), (222.0, 181.5), 'J201.1']),
    ('/S2_NODE', 'B.Cu', 2.5, ['X201.8', (232.9, 129.38), (232.9, 181.5), (222.0, 181.5), 'J201.1']),
    ('/S2_NODE', 'F.Cu', 2.5, [(226.2, 181.5), 'J201.2']),
    ('/S2_NODE', 'B.Cu', 2.5, [(226.2, 181.5), 'J201.2']),
    ('/S2_NODE', 'F.Cu', 1.5, [(232.9, 'C208.1'), 'C208.1']),
    # slot 1 (mirror)
    ('/S1_SENSE', 'F.Cu', 2.5, ['R101.2', (161.5, 158.5), (158.45, 158.5), 'Q101.5']),
    ('/S1_SENSE', 'F.Cu', 2.5, ['R102.2', (155.5, 158.5), (158.45, 158.5)]),
    ('/S1_OUT', 'F.Cu', 2.5, ['Q101.2', (165.5, 161.37), (165.5, 144.62), 'X101.1']),
    ('/S1_OUT', 'F.Cu', 2.5, [(165.5, 161.37), (165.5, 164.5)]),
    ('/S1_OUT', 'B.Cu', 2.0, [(165.5, 164.5), (136.8, 164.5)]),
    ('/S1_OUT', 'F.Cu', 1.5, [(136.8, 164.5), (136.8, 162.0), (136.8, 'C102.1'), 'C102.1']),
    ('/S1_NODE', 'F.Cu', 2.5, ['X101.8', (107.1, 144.62), (107.1, 181.5), (122.2, 181.5), 'J101.2']),
    ('/S1_NODE', 'B.Cu', 2.5, ['X101.8', (107.1, 144.62), (107.1, 181.5), (122.2, 181.5), 'J101.2']),
    ('/S1_NODE', 'F.Cu', 2.5, [(118.0, 181.5), 'J101.1']),
    ('/S1_NODE', 'B.Cu', 2.5, [(118.0, 181.5), 'J101.1']),
    ('/S1_NODE', 'F.Cu', 1.5, [(107.1, 'C108.1'), 'C108.1']),
]
# shunt VBUS pads -> two vias beside the pad into the B.Cu bar
SHUNT_VIAS = [('R201.1', 181.3), ('R202.1', 187.3), ('R101.1', 158.7), ('R102.1', 152.7)]
# fine-pitch GND pads the pour's thermal spokes cannot reach: a 0.2 mm stub out of the pad
# into the pour (the refill joins it). Direction: away from the footprint centre.
GND_STUBS = ['U102.1', 'U102.2', 'U102.7', 'U202.1', 'U202.7',          # U202.2 (A0) is strapped to 3V3 'U6.10', 'U101.5', 'U201.5',
             'U103.1', 'U103.2', 'U103.4', 'U203.1', 'U203.2', 'U203.4', 'U3.7', 'U4.5', 'U5.5', 'U1.1', 'U2.1',
             'R5.2', 'R11.2']
# fine-pitch signal pads the router cannot escape: a straight 0.2 mm stub outward, KRT picks it up from there
ESCAPES = [('U103.5', None, 1.6), ('U103.6', None, 1.6), ('U103.7', None, 1.6), ('U103.8', None, 1.6),
           ('U203.5', None, 1.3), ('U203.6', None, 1.3), ('U203.7', None, 1.3), ('U203.8', None, 1.3),
           ('U102.8', None, 1.6), ('U202.8', None, 1.6), ('U6.12', 'x', 1.6)]   # (pad, forced axis, length)
VIAS = [('/VBUS', 123.5, 108.0), ('/VBUS', 133.0, 109.5), ('/VBUS', 177.0, 109.5), ('/S2_OUT', 174.5, 164.5), ('/S2_OUT', 203.2, 164.5), ('/S1_OUT', 165.5, 164.5), ('/S1_OUT', 136.8, 164.5)]


def mm(v):
    return pcbnew.FromMM(v)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for fp in board.GetFootprints():
        for pad in fp.Pads():
            key = fp.GetReference() + '.' + pad.GetNumber()
            if key in GND_STUBS and pad.GetNetname() != 'GND':
                raise SystemExit('%s is on %s, not GND' % (key, pad.GetNetname()))
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            # an earlier run: drop its tracks and vias (zones stay, gndpour owns them), save,
            # and start over in a fresh process (removed items leave stale SWIG proxies behind)
            for it in list(g.GetItems()):
                if not isinstance(it, pcbnew.ZONE):
                    board.Remove(it)
            board.Remove(g)
            board.Save(dst)
            os.execv(sys.executable, [sys.executable, __file__, dst, dst])
    nets = board.GetNetsByName()
    grp = pcbnew.PCB_GROUP(board)
    grp.SetName(GROUP)
    board.Add(grp)
    pads = {}
    centres = {}
    for fp in board.GetFootprints():
        c = fp.GetPosition()
        centres[fp.GetReference()] = (c.x / 1e6, c.y / 1e6)
        for pad in fp.Pads():
            p = pad.GetPosition()
            pads[fp.GetReference() + '.' + pad.GetNumber()] = (p.x / 1e6, p.y / 1e6, pad.GetNetname())

    def pt(p, net):
        if isinstance(p, str):
            x, y, n = pads[p]
            if n != net:
                raise SystemExit('%s is on %s, not %s' % (p, n, net))
            return (x, y)
        x, y = p
        if isinstance(y, str):          # (x, "REF.PAD"): x given, y from the pad
            y = pads[y][1]
        return (x, y)

    tracks = []
    for net, layer, w, pts in TRACKS:
        tracks.append((net, layer, w, [pt(p, net) for p in pts]))
    vias = list(VIAS)
    for padref, vx in SHUNT_VIAS:
        x, y, n = pads[padref]
        tracks.append((n, 'F.Cu', 1.3, [(x, y), (vx, y)]))
        tracks.append((n, 'F.Cu', 1.3, [(vx, y - 0.8), (vx, y + 0.8)]))
        vias += [(n, vx, y - 0.8), (n, vx, y + 0.8)]
    def stub(padref, length, axis=None):
        x, y, n_ = pads[padref]
        cx, cy = centres[padref.split('.')[0]]
        dx, dy = x - cx, y - cy
        if axis == 'x' or (axis is None and abs(dx) >= abs(dy)):
            ex, ey = x + (length if dx > 0 else -length), y
        else:
            ex, ey = x, y + (length if dy > 0 else -length)
        tracks.append((n_, 'F.Cu', 0.2, [(x, y), (ex, ey)]))
    for padref in GND_STUBS:
        stub(padref, 1.3)
    for padref, axis, length in ESCAPES:
        stub(padref, length, axis)
    n = 0
    for net, layer, w, pts in tracks:
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(board)
            t.SetStart(pcbnew.VECTOR2I(mm(x0), mm(y0)))
            t.SetEnd(pcbnew.VECTOR2I(mm(x1), mm(y1)))
            t.SetWidth(mm(w))
            t.SetLayer(board.GetLayerID(layer))
            t.SetNet(nets[net])
            board.Add(t)
            grp.AddItem(t)
            n += 1
    for net, x, y in vias:
        v = pcbnew.PCB_VIA(board)
        v.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        v.SetWidth(mm(VIA[0]))
        v.SetDrill(mm(VIA[1]))
        v.SetViaType(pcbnew.VIATYPE_THROUGH)
        v.SetLayerPair(board.GetLayerID('F.Cu'), board.GetLayerID('B.Cu'))
        v.SetNet(nets[net])
        board.Add(v)
        grp.AddItem(v)
    zones = 0
    for z in board.Zones():
        if not z.GetIsRuleArea() and z.GetNetname() == 'GND':
            grp.AddItem(z)
            zones += 1
    board.Save(dst)
    print('power tracks', n, 'vias', len(vias), 'GND zones grouped', zones)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
