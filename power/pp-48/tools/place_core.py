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
SOLID = [('J1', '1'), ('J1', '2'), ('R4', '2'), ('U6', '21')]


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    table = {r: p for r, p in rev1.CORE.items() if r not in TILE}
    table.update(MOVED)
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
