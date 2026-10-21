#!/usr/bin/env python3
"""First placement of PM-19V-200W in a DOSA eighth-brick outline
(58.4 x 22.9 mm). Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Top: the two inductors side by side, sense resistors, output capacitors,
supervision and input-side dividers. Bottom: input capacitors and the
four MOSFETs (the hot loops stay on one side), controller and its
passives. Board origin (100, 100), brick centre (129.2, 111.45). Big
parts are fixed by courtyard centre; small parts are packed into zones.
"""
import sys

import pcbnew

W, H = 58.4, 22.9
X0, Y0 = 100.0, 100.0
CX, CY = X0 + W / 2, Y0 + H / 2
GAP = 0.25

# ref: (courtyard centre x, y, rotation, side)
FIXED = {
    'J1': (CX, CY, 0, 'F'),
    'J2': (107.0, CY, 0, 'F'),
    # top: magnetics and output
    'L1': (120.6, CY, 0, 'F'),
    'L2': (131.8, CY, 0, 'F'),
    'R9': (142.4, 102.5, 0, 'F'),
    'R10': (142.4, 120.4, 0, 'F'),
    'C24': (149.7, 105.9, 90, 'F'),
    'C25': (149.7, 117.0, 90, 'F'),
    'C20': (141.0, 106.45, 0, 'F'),
    'C21': (141.0, 109.75, 0, 'F'),
    'C22': (141.0, 113.05, 0, 'F'),
    'C23': (141.0, 116.35, 0, 'F'),
    # bottom: input capacitors and MOSFETs
    'C1': (110.75, 102.0, 0, 'B'),
    'C2': (110.75, 105.3, 0, 'B'),
    'C3': (110.75, 108.6, 0, 'B'),
    'C5': (110.75, 114.3, 0, 'B'),
    'C6': (110.75, 117.6, 0, 'B'),
    'C7': (110.75, 120.9, 0, 'B'),
    'Q1': (118.9, 104.0, 0, 'B'),
    'Q2': (126.3, 104.0, 0, 'B'),
    'Q3': (118.9, 118.9, 0, 'B'),
    'Q4': (126.3, 118.9, 0, 'B'),
    'C4': (131.75, 104.0, 90, 'B'),
    'C8': (131.75, 118.9, 90, 'B'),
    'U1': (140.2, CY, 0, 'B'),
}

# side, zone list (x0, y0, x1, y1), refs in order, rotation
PACK = [
    ('F', [(108.4, 100.3, 114.8, 122.6), (143.6, 104.8, 147.0, 118.2)],
     ['U2', 'U3', 'R3', 'R11', 'D3', 'R12', 'R13', 'R14', 'R15', 'C26', 'C27', 'C28',
      'R4', 'R5', 'R16'], 0),
    ('B', [(133.5, 100.3, 152.6, 106.1)], ['D1', 'C18'], 0),
    ('B', [(133.5, 116.8, 152.6, 122.6)], ['D2', 'C19'], 0),
    ('B', [(145.6, 106.2, 152.6, 116.7), (133.5, 100.3, 152.6, 106.1), (133.5, 116.8, 152.6, 122.6)],
     ['R6', 'R7', 'C12', 'R8', 'C13', 'C14', 'C16', 'C17'], 90),
    ('B', [(113.4, 107.1, 134.8, 115.8)], ['R1', 'C9', 'C10', 'C11', 'C15', 'TH1'], 0),
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
    fp.SetPosition(pcbnew.VECTOR2I(0, 0))
    return cbox(fp)


def hit(bx, boxes):
    return any(not (bx[2] + GAP <= o[0] or o[2] + GAP <= bx[0] or
                    bx[3] + GAP <= o[1] or o[3] + GAP <= bx[1]) for o in boxes)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    outline(board)
    fps = {f.GetReference(): f for f in board.GetFootprints()}
    boxes = {'F': [], 'B': []}
    for ref, (x, y, rot, side) in FIXED.items():
        fp = fps[ref]
        x0, y0, x1, y1 = orient(fp, rot, side)
        fp.SetPosition(pcbnew.VECTOR2I(mm(x - (x0 + x1) / 2), mm(y - (y0 + y1) / 2)))
        bx = cbox(fp)
        if fp.GetAttributes() & pcbnew.FP_THROUGH_HOLE:
            boxes['F'].append(bx)
            boxes['B'].append(bx)
        else:
            boxes[side].append(bx)
    # J1 courtyard is the pin circles only, not a box: add each pin
    for pad in fps['J1'].Pads():
        p, r = pad.GetPosition(), tomm(pad.GetSizeX()) / 2 + 0.25
        pb = (tomm(p.x) - r, tomm(p.y) - r, tomm(p.x) + r, tomm(p.y) + r)
        boxes['F'].append(pb)
        boxes['B'].append(pb)
    boxes['F'] = [b for b in boxes['F'] if b[2] - b[0] < 40]
    boxes['B'] = [b for b in boxes['B'] if b[2] - b[0] < 40]
    left = []
    for side, zones, refs, rot in PACK:
        for ref in refs:
            fp = fps[ref]
            x0, y0, x1, y1 = orient(fp, rot, side)
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
    board.Save(dst)
    print('fixed', len(FIXED), 'packed', sum(len(p[2]) for p in PACK) - len(left), 'did not fit', left)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
