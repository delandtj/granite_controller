#!/usr/bin/env python3
"""Placement of the PP-48 unit board (docs/adr/0005-chained-plane-boards.md decision 2).
Usage: place_unit.py <in.kicad_pcb> <out.kicad_pcb>

DRAFT for review (2026-10-29). 78.5 x 72 mm, board x 91-169.5, y 100-172:

- Left margin x 91-100: chain IN header J301 on B, mating face at the left
  edge, pins at v 35..61 (pin 1 = 3V3 at the bottom, the own segment on
  pins 2 / 3 above it); holes H1 / H2 at the corners (pcbkit outline puts H2
  top right; moved).
- Slots A (x 100) and B (x 125): the rev1 slot table of tools/place.py
  unchanged (unit references +0, slot B +50), unit parts as rev1.
- Input strip x 150-169.5 (19.5 mm: two FETs side by side plus a hole) (decision 2, ADR 0005): XT60 J201 at the top edge
  (cable from the top), hole H3 under it, bulk C201, TVS D201, shunts
  R201 / R202 and FETs Q202 / Q203 down the F side (VIN -> USENSE -> VBUS
  in signal-flow order), the LM5069 U207, INA238 U208, dividers and small
  parts on B behind them; chain OUT header J302 on B at the right edge at
  the same height as J301 (pin 14 = 3V3 at the bottom), so the cable to the
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
LEFT, RIGHT = X0 - 9.0, X0 + 69.5        # board edges
SX = X0 + 50.0                           # strip origin
PIN_TOP, PIN_BOT = Y0 + 35.0, Y0 + 61.0  # chain pin rows v 35..61 (14 pins, pitch 2.0)

# strip, (u, v) from the strip's top-left corner (x = SX). Power path top-down on F (VIN column on
# the left, tools/route_unit.py): XT60 -> C201 -> D201 -> shunts R201 / R202 stacked (VIN pads left,
# USENSE pads right) -> USENSE column on the right -> FET drain tabs (Q202 / Q203 side by side under
# J302, tabs up) -> sources at the bottom edge -> VBUS pour along the bottom and up the column right of
# J302's pins, via field to In2. The chain band's via ladder sits at u 0.4-3.4, v 57-59.4 (F / B free).
STRIP = {
    'C201': (6.4, 29.3, 0, F), 'H3': (16.3, 26.5, 0, F),
    'D201': (6.5, 40.5, 90, F),
    'R201': (4.5, 47.7, 0, F), 'R202': (4.5, 51.7, 0, F),
    'Q202': (3.8, 66.2, 90, F), 'Q203': (9.6, 66.2, 90, F),
    'H4': (16.0, 68.5, 0, F),
    # B: hot-swap controller and monitor behind the shunts, dividers above them (VIN pads of R233 /
    # R235 at u 1.2 / 3.0, under the VIN column)
    'C202': (7.6, 27.0, 90, B),
    # LM5069 U207 right under the UVLO / OVLO dividers (their outputs drop straight to its left pins), its
    # TIMER cap and PWR / PGD resistors above its right pins; INA238 U208 below it, I2C / +3V3 pins facing
    # slot B (free B space between the slot and the strip), Kelvin taps by R201 / R202's pads
    'U207': (4.0, 44.0, 0, B), 'C203': (6.0, 41.0, 0, B),
    # C216 under U208's +3V3 strap pin 1 (tied to VS pin 6 under the body): U208's right side stays free for
    # EXP_RSTN / PRESS_EN, which climb from the band ladder past it to J302's top pins
    'U208': (4.0, 50.0, 0, B), 'C216': (1.6, 53.0, 0, B),
    'R233': (1.1, 34.5, 90, B), 'R235': (3.2, 34.5, 90, B), 'R234': (5.1, 34.5, 90, B), 'R236': (6.6, 34.5, 90, B),
    'R238': (4.6, 38.5, 90, B), 'R237': (6.4, 38.5, 90, B),
    'Q204': (2.4, 30.2, 0, B), 'R214': (5.2, 30.4, 90, B),     # U_OFF: on the slot B side, by the UVLO divider
    'C214': (2.6, 64.0, 0, B), 'C215': (7.4, 64.0, 0, B),
    'R210': (7.0, 71.0, 180, B), 'R211': (12.8, 71.0, 0, B),
}
HOLES = {'H1': (LEFT + 4.0, Y0 + 4.0), 'H2': (LEFT + 4.0, Y0 + 68.0)}   # left margin corners
# press deadline (docs/press-deadline-line.md): LED return FET Q10 and PRESS_EN pull-down R31 on F in the
# margin, above J301's PRESS_EN pin (13, v 37)
MARGIN = {'Q10': (LEFT + 4.5, Y0 + 30.0, 0, F), 'R31': (LEFT + 4.5, Y0 + 27.2, 0, F)}
# slot B's LED resistors (rev1 COL R21 / R22 + 50) sit right under the expander's bottom row in slot B: down
# 1.8 mm on B, so U_OFF (pin 12) gets its escape via there (tools/route_unit.py escapes())
SLOT_B = {'R71': (X0 + rev1.PITCH + 11.4, Y0 + 43.4, 90, B), 'R72': (X0 + rev1.PITCH + 12.8, Y0 + 43.4, 90, B)}
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
    for ref, (u, v, rot, side) in STRIP.items():
        put(board, ref, SX + u, Y0 + v, rot, side)
    for ref, (x, y) in HOLES.items():
        put(board, ref, x, y, 0, F)
    for ref, (x, y, rot, side) in list(MARGIN.items()) + list(SLOT_B.items()):
        put(board, ref, x, y, rot, side)
    n += len(STRIP) + 4
    for ref, num in SOLID:
        solid(board, ref, num)
    left = [fp.GetReference() for fp in board.GetFootprints()
            if pcbnew.ToMM(fp.GetPosition().x) < LEFT and not fp.GetReference().startswith(('MOD', 'H'))]
    board.Save(dst)
    print('unit: placed', n, '; not placed:', ' '.join(sorted(left)) or '-')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
