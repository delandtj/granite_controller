#!/usr/bin/env python3
"""Placement of PM-19V-250W in the 58.4 x 22.9 mm module outline.
Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Board origin (100, 100), module centre (129.2, 111.45); input end on the
left, output end on the right. Mount and aux pads per docs/pm-interface.md
(J1 = PM_Mount_M2 at the module centre, screws S1 VIN top left, S2 GND
bottom left, S3 GND bottom right, S4 VOUT top right; J2 = aux pads on B at
(+12.70, -7.85)). Nothing but J1's own copper inside the screw keep-outs
(6.8 mm diameter, both sides).

Bottom: phase 1 along the top edge, left to right: input caps C1-C4, Q1
high side (drain left), Q2 low side. Phase 2 along the bottom edge, right
of the controller: Q4 low side, Q3 high side (drain right), input caps
C5-C8. U1 bottom left. The output polymers C24/C25 (2.0 mm high) in the
middle band at the output end, between the aux pads and phase 2.

Top: L1 and L2 (XAL1510, 16 x 17 mm) side by side, switch pads facing
each other over the switch-node vias; sense resistors R9/R10 upright at
the output end (pad 2 toward the middle, VOUT); output MLCCs C20-C23 in the
middle band between the S4 and S3 keep-outs; the output supervision at the
input end.

Fixed parts are placed by footprint origin (MOSFETs, inductors, caps,
sense resistors: origin = body centre) or by courtyard centre (U1); the
small parts are packed into zones that keep the switch-node and GND via
fields clear (tools/power.py puts the vias there).
"""
import math
import sys

import pcbnew

W, H = 58.4, 22.9
X0, Y0 = 100.0, 100.0
CX, CY = X0 + W / 2, Y0 + H / 2
GAP = 0.25
SCREW_KEEP = 3.4     # keep-out radius around each screw, both sides (docs/pm-interface.md)

# ref: (x, y, rotation, side) by footprint origin
ORIGIN = {
    'J1': (CX, CY, 0, 'F'),
    # bottom, phase 1 (top edge)
    'C1': (109.6, 101.95, 180, 'B'),
    'C2': (114.45, 101.95, 180, 'B'),
    'C3': (109.6, 105.3, 180, 'B'),
    'C4': (114.45, 105.3, 180, 'B'),
    'Q1': (120.8, 104.0, 180, 'B'),
    'Q2': (128.3, 104.0, 180, 'B'),
    # bottom, phase 2 (bottom edge)
    'Q4': (129.6, 118.9, 0, 'B'),
    'Q3': (137.1, 118.9, 0, 'B'),
    'C5': (143.3, 117.55, 0, 'B'),
    'C6': (148.15, 117.55, 0, 'B'),
    'C7': (143.3, 120.9, 0, 'B'),
    'C8': (148.15, 120.9, 0, 'B'),
    # bottom, phase 2 bootstrap beside SW2 (short BOOST2 loop)
    'D2': (126.0, 112.0, 0, 'B'),
    'C19': (126.0, 114.4, 0, 'B'),
    # bottom, output polymers
    'C24': (142.6, 110.5, 180, 'B'),     # VOUT pads of both polymers in the middle, under the VOUT pour
    'C25': (152.0, 110.5, 0, 'B'),
    # top
    'L1': (118.7, CY, 180, 'F'),
    'L2': (135.95, CY, 0, 'F'),
    'R9': (146.1, 104.3, 270, 'F'),
    'R10': (146.1, 118.6, 90, 'F'),
    'C20': (150.73, 109.02, 0, 'F'),
    'C21': (155.63, 109.02, 180, 'F'),   # GND pads of C20-C23 in the middle column
    'C22': (150.73, 112.47, 0, 'F'),
    'C23': (155.63, 112.47, 180, 'F'),
}
# by courtyard centre; U1 rotation is chosen in main()
COURTYARD = {
    'U1': (116.45, 116.0, 180, 'B'),      # 1.8 mm below for the phase 2 drive fan-out, a channel right
}
AUX = ('J2', CX + 12.70, CY - 7.85)   # pads on B.Cu in an F footprint

# keep-clear boxes (switch-node and GND via fields), per side
RESERVED = {
    'B': [(123.2, 106.2, 128.3, 110.7),      # SW1 pour and vias, below Q1/Q2
          (132.2, 112.2, 134.5, 116.7),      # SW2 vias, above Q3's sources
          (132.6, 104.9, 136.0, 106.9),      # GND vias, Q2 sources
          (135.6, 107.1, 140.7, 113.6),      # C24 GND pour and vias
          (124.2, 116.5, 125.9, 122.6),      # GND vias, Q4 sources
          (121.7, 110.9, 124.1, 122.6),      # routing channel right of U1 (pins 1-12)
          (106.9, 106.8, 117.3, 108.6),      # VIN pour band under C1-C4
          (111.2, 121.2, 121.8, 122.95),     # fan-out below U1 (phase 2 drive pins)
          (135.4, 114.0, 141.3, 116.1)],     # VIN vias (In1 feed), above Q3's drain
    'F': [(106.9, 100.3, 148.1, 104.7),      # SNS1P pour along the top edge
          (106.9, 106.8, 117.0, 108.4),      # VIN backbone vias under C1-C4 (B), keep F clear
          (125.6, 104.8, 128.4, 118.2),      # SW1 vias right of L1 pad 1
          (132.1, 106.9, 134.5, 118.2),      # SW2 vias right of L2 pad 1
          (124.2, 118.2, 125.9, 122.6)],     # Q4 GND vias
}

# side, zones (x0, y0, x1, y1)
B1 = (107.0, 108.7, 123.0, 110.6)     # middle band, above U1, left of the SW1 vias
B2 = (127.2, 107.1, 137.9, 112.2)     # middle band, between SW1 vias and C24
B3 = (133.7, 100.3, 137.3, 106.9)     # top edge, right of Q2's GND vias
B5 = (146.5, 100.3, 151.4, 106.9)     # top edge, right of the aux pads
B6 = (107.0, 108.7, 111.1, 122.6)     # left of U1
B7 = (124.2, 110.9, 129.4, 116.0)     # above Q4, right of U1's routing channel: phase 2 boost
T1 = (100.4, 106.8, 110.4, 114.1)     # top, input end between S1 and S2
T3 = (106.9, 120.0, 139.0, 122.6)     # top, bottom strip under the inductors
T4 = (148.3, 100.3, 151.4, 107.2)     # top, output end, beside S4
T5 = (148.3, 114.3, 151.4, 122.6)     # top, output end, beside S3

# U1's own parts: nearest free spot to the U1 pin they serve, pushed out
# from the package (ref: (pin, rotations to try))
ANCHOR = {
    'C9': ('35', (90, 0)), 'R1': ('35', (90, 0)), 'C11': ('26', (90, 0)),
    'C10': ('3', (0, 90)), 'C12': ('47', (90, 0)), 'R6': ('47', (90, 0)), 'R7': ('47', (90, 0)),
    'R8': ('46', (90, 0)), 'C13': ('46', (90, 0)), 'C14': ('46', (90, 0)), 'C15': ('45', (90, 0)),
    'C16': ('15', (90, 0)), 'C17': ('16', (90, 0)),
    'D1': ('40', (0, 90)), 'C18': ('40', (90, 0)),
    'R5': ('9', (90, 0)),
}

SUPERVISION = ['U2', 'U3', 'C27', 'C28', 'R11', 'D3', 'C26', 'R12', 'R13', 'R14', 'R15', 'R16']
# side, zones, refs, rotation for the rest
PACK = [
    ('F', [T1, T3], ['R3', 'R4'], 90),            # RUN divider: input end, beside VIN and U1's RUN pins
    ('F', [T1, T4, T5, T3], SUPERVISION, 0),
    ('B', [B1, B2, B3, B5, B6], SUPERVISION, 0),
    ('B', [B7, B2, B1, B6, B3, B5], ['C17'], 0),
]


def mm(v):
    return pcbnew.FromMM(v)


def tomm(v):
    return pcbnew.ToMM(v)


def cbox(fp):
    lay = pcbnew.B_CrtYd if fp.IsFlipped() else pcbnew.F_CrtYd
    c = fp.GetCourtyard(lay)
    b = c.BBox() if c.OutlineCount() else fp.GetBoundingBox(False)
    return (tomm(b.GetLeft()), tomm(b.GetTop()), tomm(b.GetRight()), tomm(b.GetBottom()))


def outline(board):
    for d in list(board.GetDrawings()):
        if d.GetLayer() == pcbnew.Edge_Cuts:
            board.Remove(d)
    r = pcbnew.PCB_SHAPE(board)
    r.SetShape(pcbnew.SHAPE_T_RECT)
    r.SetStart(pcbnew.VECTOR2I(mm(X0), mm(Y0)))
    r.SetEnd(pcbnew.VECTOR2I(mm(X0 + W), mm(Y0 + H)))
    r.SetLayer(pcbnew.Edge_Cuts)
    r.SetWidth(mm(0.1))
    board.Add(r)


def orient(fp, rot, side):
    if (side == 'B') != fp.IsFlipped():
        fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
    fp.SetOrientationDegrees(rot)


def hit(bx, boxes):
    return any(not (bx[2] + GAP <= o[0] or o[2] + GAP <= bx[0] or
                    bx[3] + GAP <= o[1] or o[3] + GAP <= bx[1]) for o in boxes)


def pad_xy(fp, n):
    for p in fp.Pads():
        if p.GetNumber() == n:
            q = p.GetPosition()
            return tomm(q.x), tomm(q.y)


def u1_rotation(fp):
    """Phase 1 drive pins (37-42) up, the PGND/DRVCC/RUN edge (25-36) right."""
    best = None
    for rot in (0, 90, 180, 270):
        fp.SetOrientationDegrees(rot)
        y42 = pad_xy(fp, '42')[1]
        x30 = pad_xy(fp, '30')[0]
        score = y42 - x30
        if best is None or score < best[0]:
            best = (score, rot)
    return best[1]


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    outline(board)
    fps = {f.GetReference(): f for f in board.GetFootprints()}
    boxes = {'F': list(RESERVED['F']), 'B': list(RESERVED['B'])}

    def keep(fp, side):
        bx = cbox(fp)
        if fp.GetAttributes() & pcbnew.FP_THROUGH_HOLE:
            if bx[2] - bx[0] < 40:                 # J1 spans the board: its screws are added below
                boxes['F'].append(bx)
                boxes['B'].append(bx)
        else:
            boxes[side].append(bx)

    for ref, (x, y, rot, side) in ORIGIN.items():
        fp = fps[ref]
        orient(fp, rot, side)
        fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        keep(fp, side)
    for ref, (x, y, rot, side) in COURTYARD.items():
        fp = fps[ref]
        orient(fp, 0, side)
        fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        fp.SetOrientationDegrees(u1_rotation(fp) if rot is None else rot)
        x0, y0, x1, y1 = cbox(fp)
        p = fp.GetPosition()
        fp.SetPosition(pcbnew.VECTOR2I(p.x + mm(x - (x0 + x1) / 2), p.y + mm(y - (y0 + y1) / 2)))
        keep(fp, side)
    # J1 is only screw pads: each screw keep-out is an obstacle on both sides
    for pad in fps['J1'].Pads():
        if tomm(pad.GetSizeX()) < 3:
            continue                                   # the via ring inside the pad
        q, r = pad.GetPosition(), SCREW_KEEP
        pb = (tomm(q.x) - r, tomm(q.y) - r, tomm(q.x) + r, tomm(q.y) + r)
        boxes['F'].append(pb)
        boxes['B'].append(pb)
    ref, x, y = AUX
    fp = fps[ref]
    orient(fp, 0, 'F')
    fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
    c = fp.GetCourtyard(pcbnew.B_CrtYd).BBox()
    boxes['B'].append((tomm(c.GetLeft()), tomm(c.GetTop()), tomm(c.GetRight()), tomm(c.GetBottom())))

    left, placed = [], set()
    # anchored parts first, spiral out from a point 2.5 mm outside their pin
    u1 = fps['U1']
    ux0, uy0, ux1, uy1 = cbox(u1)
    ucx, ucy = (ux0 + ux1) / 2, (uy0 + uy1) / 2
    for ref, (pin, rots) in ANCHOR.items():
        fp = fps[ref]
        px, py = pad_xy(u1, pin)
        dx, dy = px - ucx, py - ucy
        if abs(dx) > abs(dy):
            ax, ay = (ux1 + 2.0, py) if dx > 0 else (ux0 - 2.0, py)
        else:
            ax, ay = (px, uy1 + 2.0) if dy > 0 else (px, uy0 - 2.0)
        best = None
        for rot in rots:
            orient(fp, rot, 'B')
            fp.SetPosition(pcbnew.VECTOR2I(0, 0))
            x0, y0, x1, y1 = cbox(fp)
            w, h = x1 - x0, y1 - y0
            for r in [i * 0.25 for i in range(0, 60)]:
                for k in range(max(1, int(r * 8))):
                    a = 2 * math.pi * k / max(1, int(r * 8))
                    cx, cy = ax + r * math.cos(a), ay + r * math.sin(a)
                    bx = (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2)
                    if bx[0] < X0 + 0.4 or bx[2] > X0 + W - 0.4 or bx[1] < Y0 + 0.3 or bx[3] > Y0 + H - 0.3:
                        continue
                    if not hit(bx, boxes['B']):
                        d = r
                        if best is None or d < best[0]:
                            best = (d, rot, bx, x0, y0)
                        break
                if best is not None and best[0] <= r:
                    break
        if best is None:
            left.append(ref)
            continue
        d, rot, bx, x0, y0 = best
        orient(fp, rot, 'B')
        fp.SetPosition(pcbnew.VECTOR2I(mm(bx[0] - x0), mm(bx[1] - y0)))
        boxes['B'].append(bx)
        placed.add(ref)
    for side, zones, refs, rot in PACK:
        for ref in refs:
            if ref in placed:
                continue
            fp = fps[ref]
            orient(fp, rot, side)
            fp.SetPosition(pcbnew.VECTOR2I(0, 0))
            x0, y0, x1, y1 = cbox(fp)
            w, h = x1 - x0, y1 - y0
            done = False
            for zx0, zy0, zx1, zy1 in zones:
                y = zy0
                while not done and y + h <= zy1 + 1e-6:
                    x = zx0
                    while x + w <= zx1 + 1e-6:
                        bx = (x, y, x + w, y + h)
                        if not hit(bx, boxes[side]):
                            fp.SetPosition(pcbnew.VECTOR2I(mm(x - x0), mm(y - y0)))
                            boxes[side].append(bx)
                            done = True
                            break
                        x += 0.25
                    y += 0.25
                if done:
                    break
            if not done:
                left.append(ref)
            else:
                placed.add(ref)
                if ref in left:
                    left.remove(ref)
    board.Save(dst)
    print('U1 rotation', fps['U1'].GetOrientationDegrees(), '| did not fit:', left)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
