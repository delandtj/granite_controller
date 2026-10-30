#!/usr/bin/env python3
"""Placement of PM-19V-250W-B in the 58.4 x 22.9 mm module outline.
Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Board origin (100, 100), module centre (129.2, 111.45); input end left,
output end right (docs/pm-interface.md: J1 screw pads at the centre, J2 aux
header on B at (+12.70, -7.85)). Nothing but J1's own copper inside the
screw keep-outs (6.8 mm square boxes here, both sides).

B (bottom, <= 2.7 mm parts), top band, left to right: C1 at Q1's drain,
Q1 high side (drain left), Q2 low side (drain left), R4 upright (LSS pad
up). SW between Q1's source pins and Q2's drain, vias to F under L1's body.
U1 below the FETs, drive pins (11-20) up, analog pins (1-10) down; its
passives packed round it. Output MLCCs C9-C12 and the supervision on the
right half.

F (top, fluid side, <= 12 mm): input MLCCs C2-C6 in two rows at the input
end (VIN pads out, GND pads in a shared band with vias), L1 in the middle
(SW pad left), the three polymers in a column at the output end.

Small parts are packed greedily near the pad they serve (ANCHOR) outside
the reserved boxes (via fields, pours, gate-drive lanes).
"""
import math
import sys

import pcbnew

W, H = 58.4, 22.9
X0, Y0 = 100.0, 100.0
CX, CY = X0 + W / 2, Y0 + H / 2
GAP = 0.2            # courtyard to courtyard
SCREW_KEEP = 3.4     # keep-out half size around each screw, both sides
EDGE = 0.3           # courtyard to board edge

# ref: (x, y, side, pad, direction): footprint origin at (x, y), rotated so
# that `pad` lies in `direction` from the origin ((-1, 0) = left, ...)
FIXED = {
    'J1': (CX, CY, 'F', None, None),
    # B, power stage
    'C1': (109.15, 103.6, 'B', '1', (0, -1)),       # VIN pad up, GND pad down
    'Q1': (115.1, 103.9, 'B', '5', (-1, 0)),        # drain left
    'Q2': (123.1, 103.9, 'B', '5', (-1, 0)),
    'R4': (129.0, 104.45, 'B', '1', (0, -1)),       # LSS pad up, GND pad down
    # Kelvin net ties on R4's inner pad edges (under its body), R3 at Q1's gate
    'NT2': (128.4, 102.55, 'B', '1', (0, -1)),      # LSS pad on R4.1, CS below
    'NT1': (129.6, 106.35, 'B', '1', (0, 1)),       # GND pad on R4.2, CSG above
    'R3': (118.2, 107.95, 'B', '2', (0, -1)),        # HG pad up toward Q1's gate
    # U1's drive parts on B under its via rows (tools/power.py HAND routes)
    'C16': (109.4, 111.35, 'B', '1', (1, 0)),       # HB pad right, SW pad left
    'D1': (111.35, 110.3, 'B', '1', (0, 1)),        # K (HB) down beside C16, A (VCC) up
    'C17': (113.35, 110.6, 'B', '1', (-1, 0)),      # VCC pad left
    'C18': (110.2, 114.75, 'B', '1', (1, 0)),       # VCCX pad right, GND left
    'R12': (116.0, 111.0, 'B', '1', (-1, 0)),       # DEMB pad left
    # F, input caps: row 1 VIN up, row 2 VIN down, GND pads in the middle band
    'C2': (108.6, 103.9, 'F', '1', (0, -1)),
    'C3': (111.95, 103.9, 'F', '1', (0, -1)),
    'C4': (115.3, 103.9, 'F', '1', (0, -1)),
    'C5': (108.6, 109.3, 'F', '1', (0, 1)),
    'C6': (111.95, 109.3, 'F', '1', (0, 1)),
    # F, inductor and polymers (+ pad left, toward L1)
    'L1': (125.8, CY, 'F', '1', (-1, 0)),
    'C13': (146.5, 104.0, 'F', '1', (-1, 0)),
    'C14': (146.5, 111.45, 'F', '1', (-1, 0)),
    'C15': (146.5, 118.9, 'F', '1', (-1, 0)),
    # B, output MLCCs under the polymers (VOUT pad left)
    'C9': (147.0, 108.3, 'B', '1', (-1, 0)),
    'C10': (147.0, 111.7, 'B', '1', (-1, 0)),
    'C11': (147.0, 115.1, 'B', '1', (-1, 0)),
    'C12': (147.0, 118.5, 'B', '1', (-1, 0)),
}
# U1 on F, left of L1 under the input caps: drive pins (11-20) up, pin 20
# (SW) left toward Q1, pin 11 right toward R4 (the order of the stage; on B
# the mirror would reverse it)
U1_AT = (111.8, 117.5)
AUX = ('J2', CX + 12.70, CY - 7.85)

# keep-clear boxes (x0, y0, x1, y1) per side: via fields, pours, lanes
RESERVED = {
    'B': [(106.95, 100.3, 111.45, 106.9),     # VIN pour + C1 (left of Q1's drain)
          (107.3, 107.2, 117.0, 113.7),       # U1 via rows and drive parts (hand)
          (112.0, 113.6, 125.3, 114.6),       # LO lane along y 114.1
          (116.9, 109.2, 119.0, 110.7),       # U1's SW / HO vias
          (124.5, 106.3, 125.4, 114.6),       # LO lane up to Q2's gate
          (107.6, 105.9, 116.9, 107.2),       # GND vias under the F cap band
          (117.3, 100.3, 119.8, 106.3),       # SW strip and via column
          (117.3, 106.3, 119.0, 109.3),       # HG lane, R3
          (119.3, 106.4, 124.6, 109.2),       # SW vias under Q2
          (124.6, 106.4, 126.9, 112.6),       # LO lane down from Q2's gate
          (126.9, 108.2, 132.4, 110.0),       # R4 GND vias
          (130.9, 100.3, 132.4, 110.0),       # R4 GND vias, right column
          (132.4, 100.3, 134.4, 122.6),       # VOUT via column (F / In1 / B)
          (144.4, 106.2, 151.4, 121.6),       # output MLCCs and their vias
          (145.6, 100.3, 151.6, 106.2)],      # B VOUT strip to S4
    'F': [(106.95, 100.3, 117.0, 112.3)],     # input caps and their pours
}

# small parts: ref -> (side, anchor ref, anchor pad, rotations)
ANCHOR = [
    # U1 drive side (top)
    # U1 analog side (bottom)
    ('R7', 'B', 'U1', '3', (90, 0)),
    ('C19', 'B', 'U1', '5', (90, 0)),
    ('R8', 'B', 'U1', '5', (90, 0)),
    ('C20', 'B', 'U1', '7', (90, 0)),
    ('R10', 'B', 'U1', '8', (90, 0)),
    ('R9', 'B', 'U1', '8', (90, 0)),
    ('R11', 'B', 'U1', '9', (90, 0)),
    ('C21', 'B', 'U1', '9', (90, 0)),
    ('C22', 'B', 'U1', '9', (90, 0)),
    ('C7', 'B', 'U1', '1', (90, 0)),
    ('R2', 'B', 'U1', '1', (90, 0)),
    # UVLO / RUN / OVLO / VAUX, left of U1
    ('R14', 'B', 'U1', '2', (90, 0)),
    ('C23', 'B', 'U1', '2', (90, 0)),
    ('R13', 'B', 'U1', '2', (90, 0)),
    ('D3', 'B', 'U1', '2', (90, 0)),
    ('U4', 'B', 'J2', '4', (0, 90)),
    ('R15', 'B', 'U4', '4', (90, 0)),
    ('R16', 'B', 'U4', '4', (90, 0)),
    ('C24', 'B', 'U4', '4', (90, 0)),
    ('C25', 'B', 'U4', '5', (90, 0)),
    ('R17', 'B', 'U4', '5', (90, 0)),
    ('D4', 'B', 'U4', '5', (90, 0)),
    ('C26', 'B', 'U4', '5', (90, 0)),
    # VCCX follower right of U1
    ('Q3', 'B', 'J2', '2', (0, 90, 180, 270)),
    ('R6', 'B', 'Q3', '1', (0, 90)),
    ('D2', 'B', 'Q3', '1', (0, 90)),
    # supervision, right half
    ('R1', 'B', 'J2', '3', (90, 0)),
    ('U2', 'B', 'J2', '1', (0, 90)),
    ('C27', 'B', 'U2', '6', (0, 90)),
    ('R18', 'B', 'U2', '4', (0, 90)),
    ('R19', 'B', 'U2', '4', (0, 90)),
    ('U3', 'B', 'J2', '2', (0, 90)),
    ('C28', 'B', 'U3', '6', (0, 90)),
    ('R20', 'B', 'U3', '3', (0, 90)),
    ('R21', 'B', 'U3', '3', (0, 90)),
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


def side(fp, s):
    if (s == 'B') != fp.IsFlipped():
        fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)


def pad_xy(fp, n):
    for p in fp.Pads():
        if p.GetNumber() == n:
            q = p.GetPosition()
            return tomm(q.x), tomm(q.y)
    raise KeyError(n)


def point(fp, pad, d):
    """rotate fp so that pad lies in direction d from its origin"""
    best = None
    for rot in (0, 90, 180, 270):
        fp.SetOrientationDegrees(rot)
        o = fp.GetPosition()
        px, py = pad_xy(fp, pad)
        dx, dy = px - tomm(o.x), py - tomm(o.y)
        n = math.hypot(dx, dy) or 1
        score = (dx * d[0] + dy * d[1]) / n
        if best is None or score > best[0]:
            best = (score, rot)
    fp.SetOrientationDegrees(best[1])


def hit(bx, boxes):
    return any(not (bx[2] + GAP <= o[0] or o[2] + GAP <= bx[0] or
                    bx[3] + GAP <= o[1] or o[3] + GAP <= bx[1]) for o in boxes)


def inside(bx):
    return (bx[0] >= X0 + EDGE and bx[2] <= X0 + W - EDGE and
            bx[1] >= Y0 + EDGE and bx[3] <= Y0 + H - EDGE)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    outline(board)
    fps = {f.GetReference(): f for f in board.GetFootprints()}
    boxes = {'F': list(RESERVED['F']), 'B': list(RESERVED['B'])}
    fixed = set()

    for ref, (x, y, s, pad, d) in FIXED.items():
        fp = fps[ref]
        side(fp, s)
        fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        if pad:
            point(fp, pad, d)
        fixed.add(ref)
    # U1: pins 11-20 up
    u1 = fps['U1']
    side(u1, 'F')
    u1.SetPosition(pcbnew.VECTOR2I(mm(U1_AT[0]), mm(U1_AT[1])))
    best = None
    for rot in (0, 90, 180, 270):
        u1.SetOrientationDegrees(rot)
        y15 = pad_xy(u1, '15')[1]
        y5 = pad_xy(u1, '5')[1]
        x20 = pad_xy(u1, '20')[0]
        x11 = pad_xy(u1, '11')[0]
        score = (pad_xy(u1, '5')[1] - pad_xy(u1, '15')[1]) + (pad_xy(u1, '11')[0] - pad_xy(u1, '20')[0])
        if best is None or score > best[0]:
            best = (score, rot)
    u1.SetOrientationDegrees(best[1])
    x0, y0, x1, y1 = cbox(u1)
    p = u1.GetPosition()
    u1.SetPosition(pcbnew.VECTOR2I(p.x + mm(U1_AT[0] - (x0 + x1) / 2), p.y + mm(U1_AT[1] - (y0 + y1) / 2)))
    fixed.add('U1')
    ref, x, y = AUX
    fp = fps[ref]
    side(fp, 'B')
    fp.SetOrientationDegrees(0)
    fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
    fixed.add(ref)

    for ref in fixed:
        fp = fps[ref]
        s = 'B' if fp.IsFlipped() else 'F'
        if ref == 'J1':
            continue
        if ref == 'J2':
            c = fp.GetCourtyard(pcbnew.B_CrtYd).BBox()
            boxes['B'].append((tomm(c.GetLeft()), tomm(c.GetTop()), tomm(c.GetRight()), tomm(c.GetBottom())))
            continue
        boxes[s].append(cbox(fp))
    boxes['B'].append(cbox(u1))            # its thermal vias and B pad go through
    for pad in fps['J1'].Pads():
        if tomm(pad.GetSizeX()) < 3:
            continue
        q, r = pad.GetPosition(), SCREW_KEEP
        pb = (tomm(q.x) - r, tomm(q.y) - r, tomm(q.x) + r, tomm(q.y) + r)
        boxes['F'].append(pb)
        boxes['B'].append(pb)

    left = []
    for ref, s, aref, apad, rots in ANCHOR:
        fp = fps[ref]
        side(fp, s)
        ax, ay = pad_xy(fps[aref], apad)
        best = None
        for rot in rots:
            fp.SetOrientationDegrees(rot)
            fp.SetPosition(pcbnew.VECTOR2I(0, 0))
            x0, y0, x1, y1 = cbox(fp)
            w, h = x1 - x0, y1 - y0
            found = None
            for i in range(1, 260):
                r = i * 0.1
                n = max(8, int(r * 12))
                for k in range(n):
                    a = 2 * math.pi * k / n
                    cx, cy = ax + r * math.cos(a), ay + r * math.sin(a)
                    bx = (cx - w / 2, cy - h / 2, cx + w / 2, cy + h / 2)
                    if inside(bx) and not hit(bx, boxes[s]):
                        found = (r, bx)
                        break
                if found:
                    break
            if found and (best is None or found[0] < best[0]):
                best = (found[0], rot, found[1], x0, y0)
        if best is None:
            left.append(ref)
            continue
        r, rot, bx, x0, y0 = best
        fp.SetOrientationDegrees(rot)
        fp.SetPosition(pcbnew.VECTOR2I(mm(bx[0] - x0), mm(bx[1] - y0)))
        boxes[s].append(bx)
    board.Save(dst)
    print('U1 rotation', u1.GetOrientationDegrees(), '| not placed:', left)
    for ref in ('Q1', 'Q2'):
        print(ref, 'gate', pad_xy(fps[ref], '4'), 'drain', pad_xy(fps[ref], '5'))
    print('U1 pins', {n: tuple(round(v, 2) for v in pad_xy(u1, n)) for n in ('1', '10', '11', '20')})


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
