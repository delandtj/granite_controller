#!/usr/bin/env python3
"""Placement of the PM module bench carrier (130 x 55 mm at (100, 100)).
Usage: place.py <in.kicad_pcb> <out.kicad_pcb>

Screwed PM mount in the middle (module centre (165, 127.5), rotation 0, input
end left): standoffs SO1-SO4 and the aux socket J3 at the positions
power/tools/pmmount.py derives from docs/pm-interface.md, plus the board-only
silk outline MOD1. Input terminal and bulk caps on the left, output terminal
and caps on the right, status LEDs along the top edge, aux 5 V rail and
signal test points along the bottom edge. Kelvin test points above and below
the corner standoffs. Everything on the F side; nothing under the module
outline but the mount (host parts max 2.0 mm there).
"""
import os
import sys

import pcbnew

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'tools'))
import pmmount  # noqa: E402

W, H = 130.0, 55.0
X0, Y0 = 100.0, 100.0
CX, CY = X0 + 65.0, Y0 + 27.5

# ref: (x, y, rotation) by footprint origin
PLACE = {
    # input, left
    'J1': (105.5, 125.0, 270),     # wire entry (+y of the footprint) faces the left edge
    'F1': (113.0, 106.5, 0),
    'D1': (113.0, 112.5, 0),
    'C1': (120.0, 112.5, 0),
    'C2': (120.0, 124.0, 0),
    'C3': (132.5, 117.0, 90),
    'LED1': (121.0, 105.0, 0),
    'R1': (127.0, 105.0, 0),
    'TP1': (139.3, 112.5, 0),
    'TP2': (139.3, 142.5, 0),
    # output, right
    'J2': (224.5, 124.0, 90),      # wire entry faces the right edge
    'C4': (205.0, 110.0, 90),
    'C5': (198.5, 113.0, 90),
    'LED2': (211.0, 105.0, 0),
    'R2': (217.0, 105.0, 0),
    'TP3': (190.7, 112.5, 0),
    'TP4': (190.7, 142.5, 0),
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
    'TP6': (184.0, 148.5, 0),
    'JP1': (189.0, 148.5, 90),
    'TP5': (200.0, 148.5, 0),
}

MOUNT = pmmount.host_parts(CX, CY, 0)
PLACE.update({'SO%s' % n: (*MOUNT['S' + n], 0) for n in '1234'})
PLACE['J3'] = (*MOUNT['AUX'], 0)

SOLID = [('SO2', '1'), ('SO3', '1'), ('J1', '2'), ('J2', '2'), ('TP2', '1'), ('TP4', '1')]


def mm(v):
    return pcbnew.FromMM(v)


def main(src, dst):
    outline = pcbnew.FootprintLoad(pmmount.LIBS['carrier'], 'PM_Module_Outline')   # before LoadBoard
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
    old = board.FindFootprintByReference('MOD1')
    if old:
        board.Delete(old)
    outline.SetFPID(pcbnew.LIB_ID('carrier', 'PM_Module_Outline'))
    outline.SetReference('MOD1')
    board.Add(outline)
    outline.SetPosition(pcbnew.VECTOR2I(mm(CX), mm(CY)))
    board.Save(dst)
    print('placed', len(PLACE))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
