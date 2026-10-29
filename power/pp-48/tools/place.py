#!/usr/bin/env python3
"""Placement of the PP-48 plane columns (docs/adr/0004-plane-column-layout.md).
Usage: place.py <in.kicad_pcb> <out.kicad_pcb> [N]

DRAFT for review (2026-10-28): one column table, copied to every column by
translation only (references +100 per column, x + 25 mm). Column-local
coordinates (u, v) in mm from the column's top-left corner; column c starts
at x = X0 + 25 (c - 1). The column is 25 x 72 mm.

Module (PM-19V-250W) centred at (12.5, 30.7), host rotation 270, input end
up: the standoffs and the aux socket come from power/tools/pmmount.py
(S1 VIN top right, S2 GND top left, S4 VOUT bottom right, S3 GND bottom
left, socket right of centre in the lower half). Under the module on F
(max 2.0 mm): the front end top right (shunts, FET into S1, LM5069), the
INA238 top left, the enable chain and OV latch in the middle, expander, ID
ADC and the module-signal pull-ups in the lower half. On B under the module:
the parts over 2.0 mm (SMB clamp, 1210 cap, photoMOS, opto) and their
resistors, in the upper half (the lower half stays free for vias). Bottom edge: GH front panel left, DG128 node terminal right with
+19 V (pin 2) straight under S4.
"""
import os
import sys

import pcbnew

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'tools'))
import pmmount  # noqa: E402

PITCH = 25.0
X0, Y0 = 100.0, 100.0
MOD = (12.5, 30.7)            # module centre, column-local
LIB = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'pp48.pretty')

# sheet-local ref: (u, v, rotation, side)
F, B = 'F', 'B'
COL = {
    # front end, top right: VBUS (In2) -> shunts -> FET -> S1
    'R1': (12.5, 11.5, 0, F), 'R2': (12.5, 15.5, 0, F),
    'Q1': (20.5, 13.0, 270, F),         # source pins up (to S1), drain tab down (SENSE), gate top left
    'U1': (19.5, 19.5, 0, F),
    'R3': (14.0, 20.5, 90, F), 'R4': (14.0, 23.5, 90, F),
    'R5': (11.5, 20.5, 90, F), 'R6': (11.5, 23.5, 90, F),
    'R7': (21.5, 23.0, 0, F), 'C1': (18.5, 23.0, 0, F), 'R8': (23.8, 25.2, 90, F),
    # INA238, top left
    'U2': (4.5, 13.0, 270, F), 'C3': (1.6, 13.0, 90, F),   # IN+ / IN- pins face the shunts' bottom ends
    # enable chain and OV latch: right of the expander, under the LM5069 (UVLO spine) and above
    # the socket (OV_KILL comes up from its top row)
    'Q2': (15.5, 27.0, 0, F), 'R9': (15.5, 29.8, 0, F),
    'Q3': (19.5, 27.0, 0, F), 'R10': (19.5, 29.8, 0, F),
    'U3': (18.0, 33.5, 0, F), 'C4': (15.2, 33.5, 90, F),
    'R11': (21.2, 32.8, 90, F), 'C5': (22.8, 32.8, 90, F),
    'Q4': (21.5, 36.6, 0, F), 'R12': (18.0, 36.8, 0, F),
    'R13': (23.6, 39.6, 90, F),
    # expander in the middle: outputs / inputs on the side they go to (tools/sch/blocks/column.py U5)
    'U5': (10.0, 36.5, 0, F), 'C9': (12.0, 31.2, 0, F),
    # ID ADC, RUN FET and module-signal pull-ups around the socket (pins: top row ID GND OVK,
    # bottom row RUN GND PG, at u 19.08 / 20.35 / 21.62)
    'R15': (17.2, 39.4, 90, F), 'R24': (16.2, 41.6, 0, F),
    'U4': (14.6, 44.2, 0, F), 'C6': (17.2, 44.6, 90, F),
    'C10': (12.4, 47.2, 90, F), 'C11': (13.8, 47.2, 90, F),
    'Q5': (19.4, 49.2, 0, F), 'R23': (16.4, 49.6, 90, F),
    'R14': (23.6, 46.6, 90, F),
    'R26': (11.0, 57.0, 270, F), 'R27': (12.6, 57.0, 270, F),     # segment pull-ups at the In2 channel tap
    # B side under the module: clamp and cap by S1; photoMOS and opto in the lower left, next to
    # the expander's PWRD / RSTD / LED pins and above the GH connector
    'D1': (13.0, 5.0, 180, B), 'C2': (13.0, 9.0, 180, B),       # clamp (cathode pad 1 to the OUT side) and cap between S2 (GND) and S1 (VIN)
    # PGOOD-loss trip: arm FF U10 with its set stack Q8 / Q9 and DISARM FET Q7 on F at the left edge
    # (free on both sides; B behind U3 and the socket stays free for their escape vias); trip FET Q6
    # on B behind the socket's PG / OVK
    'Q7': (2.6, 21.8, 0, F), 'R29': (5.6, 21.8, 90, F),
    'U10': (2.8, 25.6, 0, F), 'C12': (5.6, 25.6, 90, F), 'R28': (7.6, 25.6, 90, F),
    'Q8': (2.4, 29.8, 0, F), 'Q9': (6.6, 29.8, 0, F),
    'Q6': (17.0, 40.5, 0, B),
    'U8': (6.0, 41.0, 0, B), 'R21': (11.4, 41.0, 90, B), 'R22': (12.8, 41.0, 90, B),
    'U6': (6.0, 45.6, 0, B), 'R19': (11.4, 45.6, 90, B),
    'U7': (6.0, 50.2, 0, B), 'R20': (11.4, 50.2, 90, B),
    # bottom edge, entry faces down
    'J2': (6.0, 68.5, 0, F),
    'J1': (15.5, 66.8, 0, F),
}
mount = pmmount.host_parts(*MOD, 270)
COL.update({'SO%s' % n: (*mount['S' + n], 0, F) for n in '1234'})
COL['J3'] = (*mount['AUX'], 270, F)

# high-current pads join their pours solidly (no thermal spokes)
SOLID = [('SO1', '1'), ('SO2', '1'), ('SO3', '1'), ('SO4', '1'), ('J1', '1'), ('J1', '2'),
         # small GND pads whose thermal spokes are blocked by their neighbours (starved_thermal)
         ('U3', '1'), ('U3', '4'), ('C4', '2'), ('C10', '2'),
         ('U2', '2'), ('U10', '1'), ('U10', '2'), ('U10', '4'), ('Q9', '2'),
         ('R29', '2'), ('U5', '21')]


def mm(v):
    return pcbnew.FromMM(v)


def col_ref(ref, c):
    i = len(ref.rstrip('0123456789'))
    return '%s%d' % (ref[:i], int(ref[i:]) + 100 * c)


def main(src, dst, n):
    outline = pcbnew.FootprintLoad(LIB, 'PM_Module_Outline')    # before LoadBoard
    board = pcbnew.LoadBoard(src)
    placed = 0
    for c in range(1, n + 1):
        x0 = X0 + PITCH * (c - 1)
        for ref, (u, v, rot, side) in COL.items():
            fp = board.FindFootprintByReference(col_ref(ref, c))
            if fp is None:
                raise SystemExit('no footprint %s' % col_ref(ref, c))
            if fp.IsFlipped() != (side == B):
                fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
            fp.SetPosition(pcbnew.VECTOR2I(mm(x0 + u), mm(Y0 + v)))
            fp.SetOrientationDegrees(rot)
            placed += 1
        for ref, num in SOLID:
            for pad in board.FindFootprintByReference(col_ref(ref, c)).Pads():
                if pad.GetNumber() == num:
                    pad.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
        mref = 'MOD%d' % c
        old = board.FindFootprintByReference(mref)
        if old:
            board.Delete(old)
        o = pcbnew.FOOTPRINT(outline)
        o.SetFPID(pcbnew.LIB_ID('pp48', 'PM_Module_Outline'))
        o.SetReference(mref)
        board.Add(o)
        o.SetPosition(pcbnew.VECTOR2I(mm(x0 + MOD[0]), mm(Y0 + MOD[1])))
        o.SetOrientationDegrees(270)       # pp48's outline is Fab only (pmmount host_outline(silk=False))
    left = [fp.GetReference() for fp in board.GetFootprints()
            if pcbnew.ToMM(fp.GetPosition().x) < X0 and not fp.GetReference().startswith('MOD')]
    board.Save(dst)
    print('placed', placed, 'in', n, 'columns; not placed:', ' '.join(sorted(left)) or '-')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], int(sys.argv[3]) if len(sys.argv) > 3 else 2)
