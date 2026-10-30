#!/usr/bin/env python3
"""Placement of the PP-48 core block at the left end (docs/adr/0004 decision 5, layout sketch).
Usage: core.py <in.kicad_pcb> <out.kicad_pcb>

DRAFT for review (2026-10-29). Core-local coordinates (u, v) in mm from the
core's top-left corner; the core is CORE_W wide, left of column 1 (x = X0 -
CORE_W .. X0). Floor plan:

- Controller world (CTRL_*), top left and along the top edge: link
  connectors J2 / J3 (JST PH, vertical), 12 V out J4, the isolators U4 / U5
  straddling the barrier at v = 22 (CTRL pins 1-4 up), the 12 V module U2's
  outputs (pins 3 / 5 at the top). CTRL small parts on B behind them.
- 12 V module U2 top right, rotated 90 so its inputs (pins 1 / 2) face down
  into the bus world and its outputs face the top edge.
- MCU U3 left middle (SWD J5 and RUN LED below it), its small parts on B.
- 3V3 buck: L1 and the 12 V module's input bulk C11 on F between U2 and the
  XT90; U1 and its small parts on B behind them.
- XT90 J1 bottom right, mating face at the bottom edge. The tile hot-swap
  sits on B under the XT90 body (F under the body takes no parts): VIN pad ->
  shunts R1 / R2 -> FETs Q2 / Q3 (drain tab up) -> VBUS via field to In2 at
  the bottom; LM5069 U7, INA238 U8 and the dividers beside them; TVS D1 and
  C2 on B just above the XT90 pads.
- Bottom left on F: input bulk C1 (tall) and the mux U6 by the In2 channel
  (rows 59.85 + 0.3 k arrive from column 1 at x = X0).
- Corner holes H1 / H2 (pcbkit outline, 4 mm inset) at the core's left edge.
"""
import sys

import pcbnew

X0, Y0 = 100.0, 100.0           # column 1 origin (tools/place.py)
CORE_W = 48.0
CX = X0 - CORE_W
F, B = 'F', 'B'

CORE = {
    # ---- controller world (CTRL_*): top left, on F; small parts on B behind
    'J2': (10.5, 13.5, 90, F), 'J3': (16.5, 13.5, 90, F), 'J4': (4.5, 14.5, 90, F),
    'U4': (6.0, 22.0, 270, F), 'U5': (14.0, 22.0, 270, F),
    'C24': (7.9, 17.4, 0, B), 'C26': (15.9, 17.4, 0, B),
    'Q1': (19.6, 18.6, 90, B), 'R30': (19.6, 15.2, 90, B),
    'R28': (12.6, 9.0, 90, B), 'R29': (14.4, 9.0, 90, B),
    'R9': (2.0, 11.0, 90, B), 'D5': (2.0, 14.4, 90, B),
    'C13': (40.0, 2.4, 0, B),
    # ---- 12 V module, outputs (3 / 5) up, inputs (1 / 2) down
    'U2': (34.5, 13.5, 90, F), 'C12': (40.6, 23.6, 0, B),
    # ---- MCU and its parts
    'U3': (10.0, 32.0, 0, F), 'J5': (10.0, 41.6, 0, F),
    'R19': (18.6, 37.4, 90, F), 'D4': (18.6, 40.4, 90, F),
    'Y1': (3.0, 33.0, 90, B), 'C22': (3.0, 28.8, 0, B), 'C23': (3.0, 37.2, 0, B),
    'C18': (7.0, 28.2, 0, B), 'C20': (13.0, 28.2, 0, B), 'C19': (13.4, 35.8, 0, B), 'C21': (6.4, 36.0, 0, B),
    'SW1': (21.6, 30.4, 0, F),
    'R20': (20.6, 27.6, 90, B), 'R21': (20.6, 30.4, 90, B), 'R22': (20.6, 33.2, 90, B),
    'R26': (5.0, 25.8, 0, B), 'C25': (8.0, 25.8, 0, B), 'R27': (11.0, 25.8, 0, B), 'C27': (15.0, 25.8, 0, B),
    'R23': (5.6, 39.4, 90, B), 'R24': (7.2, 39.4, 90, B),
    'R15': (14.0, 39.8, 90, B), 'R16': (15.8, 39.8, 90, B), 'C17': (17.4, 39.8, 90, B),
    'R12': (19.0, 36.2, 90, B), 'R38': (20.6, 36.2, 90, B),
    # ---- 3V3 buck: tall parts on F, the rest on B behind them
    'C11': (27.8, 34.0, 0, F), 'L1': (41.0, 35.0, 0, F),
    'U1': (42.4, 29.5, 0, B), 'C7': (36.8, 29.0, 90, B), 'C8': (33.6, 29.0, 90, B),
    'R3': (35.4, 34.0, 90, B), 'R4': (37.0, 34.0, 90, B), 'R5': (38.6, 34.0, 90, B),
    'C4': (46.2, 25.4, 90, B), 'R8': (46.6, 34.0, 90, B), 'C5': (45.0, 34.0, 90, B), 'C6': (43.4, 34.0, 90, B),
    'R6': (41.8, 34.0, 90, B), 'R7': (40.2, 34.0, 90, B),
    'C9': (24.4, 26.4, 90, B), 'C10': (28.0, 26.4, 90, B),
    'R18': (22.6, 31.0, 90, B), 'D3': (22.6, 34.0, 90, B),
    # ---- tile input: XT90 bottom right (mating face down), hot-swap on B under its body
    'J1': (42.0, 47.0, 180, F),
    'D1': (36.5, 40.6, 0, B), 'C2': (28.4, 40.6, 90, B),
    'R1': (31.2, 54.05, 270, B), 'R2': (36.2, 54.05, 270, B),
    'Q2': (31.6, 61.6, 90, B), 'Q3': (38.0, 61.6, 90, B),
    'R10': (28.1, 63.4, 270, B), 'R11': (35.35, 65.85, 180, B),
    'C14': (22.8, 44.2, 0, F), 'C15': (22.8, 50.5, 0, F),
    'U8': (42.4, 52.6, 180, B), 'C16': (41.0, 55.4, 0, B),
    'U7': (25.5, 53.5, 180, B), 'C3': (20.4, 55.6, 180, B), 'R37': (20.6, 53.8, 180, B),
    'R33': (19.6, 58.4, 90, B), 'R34': (21.4, 58.4, 90, B), 'R35': (23.2, 58.4, 90, B), 'R36': (25.0, 58.4, 90, B),
    'Q4': (21.5, 62.3, 0, B), 'R14': (18.6, 62.3, 90, B),
    # ---- bottom left on F: input bulk and the mux by the In2 channel
    'C1': (14.0, 49.4, 180, F),
    'U6': (15.0, 61.0, 180, F), 'C28': (10.3, 63.6, 90, F),
    'Q6': (8.4, 57.8, 0, F), 'R31': (8.4, 61.2, 90, F), 'R32': (8.4, 64.2, 90, F),
}

# high-current / XT90 pads join their pours solidly
SOLID = [('J1', '1'), ('J1', '2'),
         ('R4', '2'), ('U6', '21')]          # small GND pads with starved thermal spokes


def mm(v):
    return pcbnew.FromMM(v)


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for ref, (u, v, rot, side) in CORE.items():
        fp = board.FindFootprintByReference(ref)
        if fp is None:
            raise SystemExit('no footprint %s' % ref)
        if fp.IsFlipped() != (side == B):
            fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
        fp.SetPosition(pcbnew.VECTOR2I(mm(CX + u), mm(Y0 + v)))
        fp.SetOrientationDegrees(rot)
    for ref, num in SOLID:
        for pad in board.FindFootprintByReference(ref).Pads():
            if pad.GetNumber() == num:
                pad.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
    left = [fp.GetReference() for fp in board.GetFootprints()
            if pcbnew.ToMM(fp.GetPosition().x) < CX - 5 and not fp.GetReference().startswith(('MOD', 'H'))]
    board.Save(dst)
    print('core: placed', len(CORE), '; not placed:', ' '.join(sorted(left)) or '-')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
