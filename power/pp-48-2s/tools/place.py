#!/usr/bin/env python3
"""Placement of the PP-48-2S power plane (150 x 95 mm at (100, 100)).
Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Rows: A (y 104-124) core: bus input left, 3V3 and 12 V bucks, MCP23017,
MCU, isolators and the controller connectors on the right edge.
B (y 125-149) the two brick sockets, slot 1 rotated 180 so both bricks
have their +VIN pin next to the VBUS artery between them (x 166-172);
X1x2 under the module's J2 position. C (y 150-195) the slot front ends
either side of the artery (shunts, FET, LM5069, INA238, OV latch), node
connectors on the bottom edge at the outer corners with the photoMOS and
LED-sense opto beside them. Slot 1 is slot 2 mirrored about x = 170.
"""
import sys

import pcbnew

X0, Y0, W, H = 100.0, 100.0, 150.0, 95.0
CY = 137.0           # brick row centre
CX1, CX2 = 136.0, 204.0

CORE = {
    'J1': (106.0, 115.0, 270),
    'F1': (117.5, 106.5, 0), 'D1': (117.5, 112.5, 0), 'C1': (127.0, 106.0, 0), 'C2': (127.0, 110.5, 0),
    'R1': (123.0, 116.5, 0), 'R2': (127.5, 116.5, 0), 'C3': (127.5, 119.5, 0),
    # 3V3 buck
    'C7': (133.0, 106.0, 90), 'C8': (133.0, 112.8, 90), 'U1': (139.0, 108.0, 0), 'L1': (154.0, 109.0, 0),
    'C4': (139.0, 113.5, 0), 'R5': (143.0, 113.5, 0),
    'R3': (134.0, 118.0, 0), 'R4': (138.5, 118.0, 0), 'R6': (142.5, 118.0, 0), 'R7': (146.5, 118.0, 0),
    'R8': (150.5, 118.0, 0), 'C5': (154.5, 118.0, 0), 'C6': (158.5, 118.0, 0),
    'C9': (150.0, 122.0, 0), 'C10': (156.0, 122.0, 0),
    # 12 V buck
    'C14': (177.0, 106.0, 90), 'C15': (177.0, 112.8, 90), 'U2': (183.0, 108.0, 0), 'L2': (198.0, 109.0, 0),
    'C11': (183.0, 113.5, 0), 'R11': (187.0, 113.5, 0),
    'R9': (178.0, 118.0, 0), 'R10': (182.5, 118.0, 0), 'R12': (186.5, 118.0, 0), 'R13': (190.5, 118.0, 0),
    'R14': (194.5, 118.0, 0), 'C12': (198.5, 118.0, 0), 'C13': (202.5, 118.0, 0),
    'C16': (194.0, 122.0, 0), 'C17': (200.0, 122.0, 0), 'F2': (207.0, 123.0, 0),
    # expander, MCU
    'U6': (215.0, 113.0, 0), 'C28': (213.0, 124.0, 0),
    'U3': (229.0, 113.0, 0), 'Y1': (229.0, 122.5, 0), 'C22': (224.0, 122.5, 0), 'C23': (234.0, 122.5, 0),
    'C18': (223.0, 104.5, 0), 'C19': (226.5, 104.5, 0), 'C20': (230.0, 104.5, 0), 'C21': (233.5, 104.5, 0),
    'J5': (238.5, 105.5, 0),
    # right strip: isolators, link connectors, aux out, straps, LEDs
    'U4': (239.0, 113.0, 90), 'J2': (245.8, 118.5, 90), 'U5': (239.0, 122.0, 90), 'J3': (245.8, 133.0, 90),
    'Q1': (239.0, 129.0, 0), 'R30': (239.0, 132.5, 0),
    'C24': (236.0, 135.5, 0), 'C25': (239.0, 135.5, 0), 'C26': (242.0, 135.5, 0),
    'C27': (236.0, 138.0, 0), 'R26': (239.0, 138.0, 0), 'R27': (242.0, 138.0, 0),
    'R28': (236.0, 140.5, 0), 'R29': (239.0, 140.5, 0),
    'J4': (245.8, 146.0, 90), 'R15': (236.0, 143.5, 0), 'R16': (239.0, 143.5, 0),
    'R17': (236.0, 146.5, 0), 'D2': (239.5, 146.5, 0),
    'JP1': (237.5, 150.0, 0), 'JP2': (237.5, 153.0, 0), 'JP3': (237.5, 156.0, 0),
    'R20': (242.5, 150.0, 0), 'R21': (242.5, 153.0, 0), 'R22': (242.5, 156.0, 0),
    'R23': (236.0, 159.0, 0), 'R24': (239.0, 159.0, 0), 'R25': (242.0, 159.0, 0),
    'R18': (236.0, 162.0, 0), 'D3': (239.5, 162.0, 0), 'R19': (236.0, 165.0, 0), 'D4': (239.5, 165.0, 0),
}

# slot 2 (right of the artery); slot 1 = mirror about x = 170 with refs 1xx
SLOT2 = {
    'X201': (CX2, CY, 0), 'X202': (CX2 - 22.2, CY, 0),
    'R201': (178.5, 153.0, 270), 'R202': (184.5, 153.0, 270),    # pad 1 (VBUS) up at y 150, pad 2 (SENSE) down at y 156
    'Q201': (180.5, 162.0, 0),
    'U201': (190.0, 163.0, 0), 'U202': (192.0, 156.0, 0), 'C203': (197.0, 156.0, 0),
    'D201': (199.5, 162.0, 0), 'C202': (206.0, 162.0, 90),
    'R203': (177.0, 170.0, 0), 'R204': (181.0, 170.0, 0), 'R205': (185.0, 170.0, 0), 'R206': (189.0, 170.0, 0),
    'R207': (193.0, 170.0, 0), 'C201': (197.0, 170.0, 0), 'R208': (201.0, 170.0, 0),
    'U203': (199.0, 166.0, 0), 'C204': (203.5, 166.0, 0), 'R211': (207.0, 166.0, 0), 'C205': (210.0, 166.0, 0), 'R213': (213.0, 166.0, 0),
    'Q202': (206.0, 170.5, 0), 'R209': (210.0, 170.5, 0), 'Q203': (206.0, 175.0, 0), 'R210': (210.0, 175.0, 0),
    'Q204': (206.0, 179.5, 0), 'R212': (210.0, 179.5, 0),
    'R214': (177.0, 174.0, 0), 'R215': (181.0, 174.0, 0), 'C206': (185.0, 174.0, 0), 'R216': (189.0, 174.0, 0),
    'C207': (193.0, 174.0, 0), 'R217': (197.5, 174.0, 0), 'D202': (202.0, 174.0, 0),
    'J201': (222.0, 186.0, 0), 'C208': (239.5, 178.0, 90), 'R218': (236.0, 169.0, 0), 'D203': (236.0, 172.0, 0),
    'U204': (196.0, 186.0, 0), 'R219': (191.0, 186.0, 0), 'U205': (196.0, 191.0, 0), 'R220': (191.0, 191.0, 0),
    'U206': (205.5, 191.0, 0), 'R221': (205.0, 187.5, 0), 'R222': (208.5, 187.5, 0),
}

PLACE = dict(CORE)
PLACE.update(SLOT2)
for ref, (x, y, rot) in SLOT2.items():
    r1 = ref[0] + '1' + ref[2:]
    PLACE[r1] = (340.0 - x, y, rot)
PLACE['X101'] = (CX1, CY, 180)
PLACE['X102'] = (CX1 + 22.2, CY, 180)
PLACE['J101'] = (118.0, 186.0, 0)
PLACE['C108'] = (103.5, 180.0, 90)
PLACE['Q101'] = (340.0 - 180.5, 162.0, 180)   # source pads toward the OUT channel at x 165.5
PLACE['R118'] = (103.0, 169.0, 0)
PLACE['D103'] = (103.0, 172.0, 0)
PLACE['U104'] = (151.5, 186.0, 0); PLACE['R119'] = (156.5, 186.0, 0)
PLACE['U105'] = (151.5, 191.0, 0); PLACE['R120'] = (156.5, 191.0, 0)
PLACE['U106'] = (140.5, 191.0, 0); PLACE['R121'] = (141.0, 187.5, 0); PLACE['R122'] = (137.5, 187.5, 0)
PLACE['D101'] = (340.0 - 199.5, 162.0, 180)   # OUT pad toward the artery side, like D201

SOLID = [('X101', '3'), ('X101', '4'), ('X201', '3'), ('X201', '4'), ('J1', '2'),
         ('J101', '3'), ('J101', '4'), ('J201', '3'), ('J201', '4')]


def mm(v):
    return pcbnew.FromMM(v)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    seen = set()
    for fp in board.GetFootprints():
        ref = fp.GetReference()
        if ref not in PLACE:
            if not ref.startswith('H'):
                print('not placed:', ref)
            continue
        seen.add(ref)
        x, y, rot = PLACE[ref]
        if fp.IsFlipped():
            fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        fp.SetOrientationDegrees(rot)
    for ref in PLACE:
        if ref not in seen:
            print('no such footprint:', ref)
    for ref, num in SOLID:
        fp = board.FindFootprintByReference(ref)
        for pad in fp.Pads():
            if pad.GetNumber() == num:
                pad.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
    board.Save(dst)
    print('placed', len(seen))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
