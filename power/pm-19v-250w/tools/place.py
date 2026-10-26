#!/usr/bin/env python3
"""Placement of PM-19V-250W in a DOSA eighth-brick outline (58.4 x 22.9 mm).
Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Board origin (100, 100), brick centre (129.2, 111.45); input pins on the
left, output pins on the right.

Bottom: phase 1 along the top edge, left to right: input caps C1-C4 (VIN
pad facing the MOSFETs), Q1 high side (drain left), Q2 low side. Phase 2
mirrored along the bottom edge, right of the controller: Q4 low side, Q3
high side (drain right), input caps C5-C8. So both switch nodes sit under
their inductor, and every gate pin faces the controller. U1 bottom left.

Top: L1 and L2 side by side, switch pads over the switch-node vias; sense
resistors R9/R10 at the top and bottom edge; output caps and the VOUT
pour at the output end; the output supervision in the free strip beside
J2 at the input end.

Fixed parts are placed by footprint origin (MOSFETs, inductors, caps,
sense resistors: origin = body centre) or by courtyard centre (U1); the
small parts are packed into zones that keep the switch-node via fields
clear (tools/power.py puts the vias there).
"""
import sys

import pcbnew

W, H = 58.4, 22.9
X0, Y0 = 100.0, 100.0
CX, CY = X0 + W / 2, Y0 + H / 2
GAP = 0.25

# ref: (x, y, rotation, side) by footprint origin
ORIGIN = {
    'J1': (CX, CY, 0, 'F'),
    'J2': (107.0, CY, 0, 'F'),
    # bottom, phase 1 (top edge)
    'C1': (107.75, 102.0, 180, 'B'),
    'C2': (112.65, 102.0, 180, 'B'),
    'C3': (107.75, 105.35, 180, 'B'),
    'C4': (112.65, 105.35, 180, 'B'),
    'Q1': (118.9, 104.0, 180, 'B'),
    'Q2': (126.3, 104.0, 180, 'B'),
    # bottom, phase 2 (bottom edge, mirrored)
    'Q4': (129.6, 118.9, 0, 'B'),
    'Q3': (137.0, 118.9, 0, 'B'),
    'C5': (143.2, 117.55, 0, 'B'),
    'C6': (147.95, 117.55, 0, 'B'),
    'C7': (143.2, 120.9, 0, 'B'),
    'C8': (147.95, 120.9, 0, 'B'),
    # top
    'L1': (119.5, CY, 180, 'F'),
    'L2': (130.7, CY, 0, 'F'),
    'R9': (142.2, 102.5, 0, 'F'),
    'R10': (142.2, 120.4, 0, 'F'),
    'C20': (142.25, 107.15, 90, 'F'),
    'C21': (145.65, 107.15, 90, 'F'),
    'C22': (142.25, 112.05, 90, 'F'),
    'C23': (145.65, 112.05, 90, 'F'),
    'C24': (149.9, 105.9, 90, 'F'),
    'C25': (149.9, 117.0, 270, 'F'),
}
# by courtyard centre; U1 rotation is chosen in main()
COURTYARD = {
    'U1': (114.0, 116.0, 180, 'B'),
}

# keep-clear boxes (switch-node via fields), per side
RESERVED = {
    'B': [(123.9, 106.9, 126.0, 110.0), (128.9, 113.3, 132.6, 116.0)],
    'F': [],
}

# side, zones (x0, y0, x1, y1), refs, rotation
B6 = (105.3, 114.9, 108.7, 122.6)     # below J2, left of U1
B1 = (108.5, 107.2, 123.8, 110.5)     # above U1, below phase 1
B2 = (119.8, 110.6, 125.9, 122.6)     # right of U1
B3 = (130.2, 100.3, 153.0, 106.9)     # right of Q2
B4 = (133.0, 107.2, 153.0, 113.2)     # above phase 2 and its caps
B5 = (150.6, 113.6, 153.0, 122.6)     # right of C6/C8
T = (108.4, 100.3, 114.6, 110.3)      # top, beside J2, clear of U1 below
T2 = (126.4, 117.9, 132.6, 122.6)     # top, below L2; x < 126 stays free for Q4's GND vias

# U1's own parts: nearest free spot to the U1 pin they serve, pushed out
# from the package (ref: (pin, rotations to try))
ANCHOR = {
    'C9': ('35', (90, 0)), 'R1': ('35', (90, 0)), 'C11': ('26', (90, 0)),
    'C10': ('3', (0, 90)), 'C12': ('47', (90, 0)), 'R6': ('47', (90, 0)), 'R7': ('47', (90, 0)),
    'R8': ('46', (90, 0)), 'C13': ('46', (90, 0)), 'C14': ('46', (90, 0)), 'C15': ('45', (90, 0)),
    'C16': ('15', (90, 0)), 'C17': ('16', (90, 0)),
    'D1': ('40', (0, 90)), 'C18': ('40', (90, 0)), 'D2': ('21', (0, 90)), 'C19': ('21', (90, 0)),
    'R4': ('32', (90, 0)), 'R5': ('9', (90, 0)), 'R3': ('32', (90, 0)), 'TH1': ('3', (0, 90)),
}

# side, zones, refs, rotation for the rest
PACK = [
    ('F', [T, T2], ['U2', 'U3', 'C27', 'C28', 'R11', 'D3', 'C26', 'R12', 'R13', 'R14', 'R15', 'R16'], 0),
    ('B', [B6, B1, B2, B4, B3, B5], ['R3'], 90),
    ('B', [B4, B3, B5], ['U2', 'U3', 'C27', 'C28', 'R11', 'D3', 'C26', 'R12', 'R13', 'R14', 'R15', 'R16'], 0),
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
            if bx[2] - bx[0] < 40:
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
    # J1 is only pins: each pin is an obstacle on both sides
    for pad in fps['J1'].Pads():
        q, r = pad.GetPosition(), tomm(pad.GetSizeX()) / 2 + 0.3
        pb = (tomm(q.x) - r, tomm(q.y) - r, tomm(q.x) + r, tomm(q.y) + r)
        boxes['F'].append(pb)
        boxes['B'].append(pb)

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
                    import math
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
