#!/usr/bin/env python3
"""Placement of the PM-19V-200W bench carrier (130 x 55 mm at (100, 100)).
Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Brick socket X1 in the middle (centre (165, 127.5), input pins left, output
pins right), X2 (aux socket) under the brick where the module's J2 sits.
Input terminal and bulk caps on the left, output terminal and caps on the
right, status LEDs along the top edge, aux 5 V rail and signal test points
along the bottom edge. Kelvin test points above and below the socket's
corner pins. Everything on the F side.
"""
import sys

import pcbnew

W, H = 130.0, 55.0
X0, Y0 = 100.0, 100.0
CX, CY = X0 + 65.0, Y0 + 27.5

# ref: (x, y, rotation) by footprint origin
PLACE = {
    'X1': (CX, CY, 0),
    'X2': (CX - 22.2, CY, 0),
    # input, left
    'J1': (105.5, 125.0, 270),     # wire entry (+y of the footprint) faces the left edge
    'F1': (113.0, 106.5, 0),
    'D1': (113.0, 112.5, 0),
    'C1': (120.0, 112.5, 0),
    'C2': (120.0, 124.0, 0),
    'C3': (132.5, 117.0, 90),
    'LED1': (121.0, 105.0, 0),
    'R1': (127.0, 105.0, 0),
    'TP1': (139.6, 112.5, 0),
    'TP2': (139.6, 142.5, 0),
    # output, right
    'J2': (224.5, 124.0, 90),      # wire entry faces the right edge
    'C4': (205.0, 110.0, 90),
    'C5': (198.5, 113.0, 90),
    'JP2': (197.5, 123.69, 0),
    'JP3': (197.5, 131.31, 0),
    'LED2': (211.0, 105.0, 0),
    'R2': (217.0, 105.0, 0),
    'TP3': (190.4, 112.5, 0),
    'TP4': (190.4, 142.5, 0),
    # status LEDs, top
    'R5': (154.0, 105.0, 0),
    'LED3': (160.0, 105.0, 0),
    'R7': (168.0, 105.0, 0),
    'LED4': (174.0, 105.0, 0),
    'Q1': (180.0, 105.0, 0),
    # aux rail, bottom left
    'R3': (111.0, 144.5, 0),
    'D2': (117.5, 144.5, 0),
    'C6': (122.0, 144.5, 90),
    'TP11': (126.0, 144.5, 0),
    # signals, bottom row
    'R4': (131.0, 148.5, 0),
    'TP7': (136.0, 148.5, 0),
    'R6': (142.0, 148.5, 0),
    'TP8': (147.0, 148.5, 0),
    'R8': (156.0, 148.5, 0),
    'TP9': (161.0, 148.5, 0),
    'R9': (170.0, 148.5, 0),
    'TP10': (175.0, 148.5, 0),
    'TP6': (184.0, 148.5, 0),
    'JP1': (189.0, 148.5, 90),
    'TP5': (200.0, 148.5, 0),
}

SOLID = [('X1', '3'), ('X1', '4'), ('J1', '2'), ('J2', '2'), ('TP2', '1'), ('TP4', '1'), ('X2', '5')]


def mm(v):
    return pcbnew.FromMM(v)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for fp in board.GetFootprints():
        ref = fp.GetReference()
        if ref not in PLACE:
            continue
        x, y, rot = PLACE[ref]
        if fp.IsFlipped():
            fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        fp.SetPosition(pcbnew.VECTOR2I(mm(x), mm(y)))
        fp.SetOrientationDegrees(rot)
    # the 10 A GND pins and the Kelvin GND loops join the GND pour solidly
    # (gndpour uses thermal reliefs; four 0.5 mm spokes are not a 10 A path)
    for ref, num in SOLID:
        fp = board.FindFootprintByReference(ref)
        for pad in fp.Pads():
            if pad.GetNumber() == num:
                pad.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
    board.Save(dst)
    print('placed', len(PLACE))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
