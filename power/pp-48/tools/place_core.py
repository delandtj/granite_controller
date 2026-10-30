#!/usr/bin/env python3
"""Placement of the PP-48 core board (docs/adr/0005-chained-plane-boards.md decision 6).
Usage: place_core.py <in.kicad_pcb> <out.kicad_pcb>

DRAFT for review (2026-10-29). 56 x 72 mm, board x 52-108, y 100-172:
the rev1 core block (tools/core.py, x 52-100) without the tile input
(XT90, hot-swap, shunts, FETs, INA238 U8), plus an 8 mm right margin:

- Chain OUT header J301 on B at the right edge, pins at v 35..61 like the
  units' headers (pin 14 = 3V3 at the bottom), holes H3 / H4 in the margin
  corners above and below it.
- Mux U6 moved from the bottom left into the freed tile area, rot 180 so
  channels 0-3 (pins 4-11) face the header, channel 0 lowest (as P0 on
  the header); its reset FET and caps beside it.
- XT30 J1 at the bottom edge (mating face down) where the XT90 was; the
  TVS D1 and C2 stay on B above its pads, C1 bulk on F next to it.
- 12 V aux (ADR 0003 decision 14): the LM5017 Fly-Buck where the module
  was, top right. The controller island (CTRL_GND) keeps its block top left
  (u 0-21.6, v 0-21) and now runs as a strip along the top edge to u 31
  (v 0-9); its band (2 mm, all layers) runs down u 31-33 and left along v
  9-11. The coupled inductor L2 sits on the strip's corner, rot 270: its
  windings are on diagonal pads (N1 1-3, N2 2-4), so no straight barrier
  splits them. SEC2 (pad 4, top left) is in the strip, SW (pad 1, top
  right) and VOUT1 (pad 3, bottom left) are on the bus side, and CTRL_GND
  (pad 2, bottom right) is an island pocket on F (>= 1.5 mm to pads 1 / 3,
  2 mm band round it) with vias at ENCLAVE to CTRL_GND on In1 / B, which
  reach under L2 from the strip (no bus copper under L2 on In1 / In2 / B).
  Secondary on F in the strip: D6 anode by pad 4, C30 / C31 under its
  cathode, the TVS D7 above the link headers J2 / J3. Primary: U2 right of L2 rot 180 (SW pin 8 next to
  pad 1), C11 / C12 at its VIN / RTN pins, R12 (RON) above them, the
  UVLO divider on B behind them, the FB / ripple network left of U2 (F and
  B, clear of L2's footprint), BST C13 and VCC C16 on B under its left
  pins, D2 below it, the VOUT1 cap C29 below pad 3.
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import core as rev1                       # noqa: E402  (rev1 core table)
from boardfit import F, B, put, fit, chain, solid   # noqa: E402

CX, Y0 = rev1.CX, rev1.Y0                 # core origin (52, 100)
RIGHT = CX + 56.0
PIN_TOP = Y0 + 35.0                       # chain pins v 35..61 (14 pins)

TILE = ('R1', 'R2', 'Q2', 'Q3', 'R10', 'R11', 'C14', 'C15', 'U8', 'C16', 'U7', 'C3', 'R37',
        'R33', 'R34', 'R35', 'R36', 'Q4', 'R14', 'R12', 'R38', 'J1')
MOVED = {
    # mux by the header: rot 180 puts pins 1-12 (channels 0-3) on the right, pin 1 lowest
    'U6': (42.0, 51.0, 180, F), 'C28': (42.0, 45.6, 0, F),
    'Q6': (34.6, 51.6, 0, F), 'R31': (36.4, 47.4, 90, F), 'R32': (34.6, 55.0, 0, F),
    # press deadline (docs/press-deadline-line.md): monostable U301 and its RC, bottom right by J301's PRESS_EN pin
    'U301': (43.0, 60.0, 0, F), 'C302': (43.0, 57.2, 0, F), 'R301': (46.6, 60.0, 90, F), 'C301': (43.0, 63.6, 0, F),
    'R302': (39.6, 59.4, 90, F), 'R303': (39.6, 62.6, 90, F),
    'H1': (4.0, 4.0, 0, F), 'H2': (4.0, 68.0, 0, F), 'H3': (52.0, 4.0, 0, F), 'H4': (52.0, 68.0, 0, F),
}
# 12 V aux Fly-Buck (the module's place, top right); see the docstring
FLY = {
    'L2': (32.0, 10.0, 270, F),
    # secondary, in the island strip
    'D6': (26.5, 3.0, 0, F), 'C30': (21.0, 6.5, 270, F), 'C31': (23.1, 6.5, 270, F), 'D7': (14.5, 1.6, 0, F),
    # primary: U2 and its input caps, RON, UVLO divider
    'U2': (41.0, 5.4, 180, F), 'C11': (46.5, 6.0, 270, F), 'C12': (45.6, 9.8, 0, F), 'R12': (46.3, 1.6, 180, F),
    'R10': (46.6, 4.8, 90, B), 'R11': (44.6, 4.8, 90, B),
    # FB divider and ripple injection left of U2; BST / VCC caps under its left pins
    'R17': (34.8, 1.4, 0, F), 'R14': (34.8, 3.0, 0, F), 'C15': (34.8, 4.6, 0, F),
    'R13': (35.2, 2.0, 0, B), 'C14': (35.0, 4.2, 0, B),
    'C13': (38.3, 6.7, 90, B), 'C16': (38.3, 3.9, 90, B),
    'D2': (41.0, 9.6, 0, F), 'C29': (29.5, 16.1, 270, F),
}
ENCLAVE = [(33.5, 13.2), (34.9, 13.2)]     # L2 pad 2 (CTRL_GND) vias to In1 / B (island there: u 28-35.5, v 7.5-14)
SOLID = [('J1', '1'), ('J1', '2'), ('R4', '2'), ('U6', '21')]


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    table = {r: p for r, p in rev1.CORE.items() if r not in TILE}
    table.update(MOVED)
    table.update(FLY)
    for ref, (u, v, rot, side) in table.items():
        put(board, ref, CX + u, Y0 + v, rot, side)
    chain(board, 'J301', RIGHT + 0.2, 'right', PIN_TOP, True)
    fit(board, 'J1', 180, F, cx=CX + 30.0, bottom=Y0 + 72.4)
    for ref, num in SOLID:
        solid(board, ref, num)
    left = [fp.GetReference() for fp in board.GetFootprints() if pcbnew.ToMM(fp.GetPosition().x) < CX - 1]
    board.Save(dst)
    print('core: placed', len(table) + 2, '; not placed:', ' '.join(sorted(left)) or '-')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
