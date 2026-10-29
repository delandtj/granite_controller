#!/usr/bin/env python3
"""Placement of the PP-48 unit board (docs/adr/0005-chained-plane-boards.md decision 2).
Usage: place_unit.py <in.kicad_pcb> <out.kicad_pcb>

DRAFT for review (2026-10-29). 76 x 72 mm, board x 91-167, y 100-172:

- Left margin x 91-100: chain IN header J301 on B, mating face at the left
  edge, pins at v 39..61 (pin 1 = 3V3 at the bottom, the own segment on
  pins 2 / 3 above it); holes H1 / H2 at the corners (pcbkit outline puts H2
  top right; moved).
- Slots A (x 100) and B (x 125): the rev1 slot table of tools/place.py
  unchanged (unit references +0, slot B +50), unit parts as rev1.
- Input strip x 150-167 (decision 2, ADR 0005): XT60 J201 at the top edge
  (cable from the top), hole H3 under it, bulk C201, TVS D201, shunts
  R201 / R202 and FETs Q202 / Q203 down the F side (VIN -> USENSE -> VBUS
  in signal-flow order), the LM5069 U207, INA238 U208, dividers and small
  parts on B behind them; chain OUT header J302 on B at the right edge at
  the same height as J301 (pin 12 = 3V3 at the bottom), so the cable to the
  next unit runs flat; H4 at the bottom right corner.
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import place as rev1                     # noqa: E402  (slot table, module outline)
from boardfit import F, B, put, fit, chain, solid   # noqa: E402

X0, Y0 = rev1.X0, rev1.Y0                # slot A origin (100, 100)
LEFT, RIGHT = X0 - 9.0, X0 + 67.0        # board edges
SX = X0 + 50.0                           # strip origin
PIN_TOP, PIN_BOT = Y0 + 39.0, Y0 + 61.0  # chain pin rows (pin pitch 2.0)

# strip, (u, v) from the strip's top-left corner (x = SX)
STRIP = {
    'H3': (8.5, 22.8, 0, F),
    'C202': (2.4, 22.8, 90, F),
    'D201': (4.0, 42.4, 90, F),
    'R201': (2.3, 51.3, 90, F), 'R202': (6.5, 51.3, 90, F),
    'Q202': (4.5, 59.0, 270, F), 'Q203': (4.5, 66.6, 270, F),
    # B: hot-swap controller and monitor behind the shunts, dividers above them
    'U208': (4.0, 44.0, 0, B), 'C216': (4.0, 41.0, 0, B),
    'U207': (4.0, 50.0, 0, B), 'C203': (4.0, 53.0, 0, B),
    'R233': (1.2, 34.5, 90, B), 'R234': (3.0, 34.5, 90, B), 'R235': (4.8, 34.5, 90, B), 'R236': (6.6, 34.5, 90, B),
    'R237': (1.2, 38.5, 90, B), 'R238': (3.0, 38.5, 90, B),
    'Q204': (12.0, 30.0, 0, B), 'R214': (15.0, 30.0, 90, B),
    'R210': (1.6, 58.0, 90, B), 'R211': (1.6, 66.6, 90, B),
    'C214': (2.6, 69.2, 0, B), 'C215': (7.4, 69.2, 0, B),
    'H4': (13.0, 68.0, 0, F),
}
HOLES = {'H1': (LEFT + 4.0, Y0 + 4.0), 'H2': (LEFT + 4.0, Y0 + 68.0)}   # left margin corners
SOLID = [('J201', '1'), ('J201', '2')]


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    # slots and unit parts: rev1's place.py, unit offset 0 (references as drawn, slot B + 50)
    n = 0
    for c, s in ((1, 'A'), (2, 'B')):
        x0 = X0 + rev1.PITCH * (c - 1)
        parts = dict(rev1.COL, **(rev1.UNIT_A if s == 'A' else rev1.UNIT_B))
        for ref, (u, v, rot, side) in parts.items():
            rev1.put(board, rev1.unit_ref(ref, 0, 'A' if ref in rev1.UNIT_B else s), u, v, rot, side, x0)
            n += 1
        for ref, num in rev1.SOLID + (rev1.UNIT_SOLID if s == 'B' else []):
            solid(board, rev1.unit_ref(ref, 0, 'A' if (ref, num) in rev1.UNIT_SOLID else s), num)
    outline = pcbnew.FootprintLoad(rev1.LIB, 'PM_Module_Outline')
    for c in (1, 2):
        ref = 'MOD%d' % c
        old = board.FindFootprintByReference(ref)
        if old:
            board.Delete(old)
        o = pcbnew.FOOTPRINT(outline)
        o.SetFPID(pcbnew.LIB_ID('pp48', 'PM_Module_Outline'))
        o.SetReference(ref)
        board.Add(o)
        o.SetPosition(pcbnew.VECTOR2I(pcbnew.FromMM(X0 + rev1.PITCH * (c - 1) + rev1.MOD[0]), pcbnew.FromMM(Y0 + rev1.MOD[1])))
        o.SetOrientationDegrees(270)
    # chain headers on B: IN at the left edge (pin 1 bottom), OUT at the right edge (pin 1 top)
    chain(board, 'J301', LEFT - 0.2, 'left', PIN_BOT, False)
    chain(board, 'J302', RIGHT + 0.2, 'right', PIN_TOP, True)
    # strip: XT60 mating face at the top edge, then the table
    fit(board, 'J201', 0, F, cx=SX + 8.5, top=Y0 - 0.4)
    fit(board, 'C201', 0, F, cx=SX + 6.6, top=Y0 + 26.6)
    for ref, (u, v, rot, side) in STRIP.items():
        put(board, ref, SX + u, Y0 + v, rot, side)
    for ref, (x, y) in HOLES.items():
        put(board, ref, x, y, 0, F)
    n += len(STRIP) + 4
    for ref, num in SOLID:
        solid(board, ref, num)
    left = [fp.GetReference() for fp in board.GetFootprints()
            if pcbnew.ToMM(fp.GetPosition().x) < LEFT and not fp.GetReference().startswith(('MOD', 'H'))]
    board.Save(dst)
    print('unit: placed', n, '; not placed:', ' '.join(sorted(left)) or '-')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
