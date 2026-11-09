#!/usr/bin/env python3
"""Placement of the PP-48 core board, CAN FD link generation (docs/adr/0006-plane-link-can-fd.md decisions
2-8; docs/adr/0005-chained-plane-boards.md decision 6).
Usage: place_core.py <in.kicad_pcb> <out.kicad_pcb>

CAN FD (2026-11-04): 68 x 72 mm, board x 43.5-111.5 (the rev D core grown 8.5 mm left and 3.5 mm right).
The link domain (LINK_GND, CAN_P / CAN_N, AUX_12V, ISO_12V, LINK_5V: the rev D CTRL island) must join the
left link plug J2 to the right link socket J3 without cutting the bus world in two, so it now runs along
the whole top edge: the island block top left (ISO1044 U4 straddling its bottom edge where the ISO1640
was), a strip along the top edge to the right edge, a channel down each edge to the link connector
(pin 1 at v 30.0, both on B, pin 1 at the top: the common link height of ADR 0006 decision 3).
- Left: J2 plug (body flush with the edge), NUP2105L D8 above it; the new 8.5 mm keeps 2 mm between the
  link channel and the crystal Y1 / U3.
- Right: J3 socket (face 4 mm past the edge) above the chain socket J301, D9 and the split termination
  (R25 / R26 / C32 over the 2x2 jumper-cap header JP1) above it; 3.5 mm more keep 2 mm from L1.
- The Fly-Buck primary (U2 and its parts) moved from the top right corner into the bus world under the
  strip: the whole cluster turned 90 deg clockwise about L2, which now straddles the strip's bottom edge
  (secondary pads up, primary down) at v 10.
- Link supply in the island block: HT7550-1 U7 (tab on ISO_12V), C27 / C26, the OR diode D10.
- MCP2518FD U5 bottom left (SPI1 at U3's bottom pins, PA0), 40 MHz Y2 under it, pull-ups R33-R36.

Rev D notes follow.

DRAFT for review (2026-10-29). 56 x 72 mm, board x 52-108, y 100-172:
the rev1 core block (tools/core.py, x 52-100) without the tile input
(XT90, hot-swap, shunts, FETs, INA238 U8), plus an 8 mm right margin:

- Chain OUT socket J301 (2x7 right angle) on B at the right edge, face 4 mm
  past it, pins v 42.5..57.74 like the units' (3V3 at the bottom), holes H3 / H4 in the margin
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
  9-11. The coupled inductor L2 (Coilcraft LPD5030V, one winding per side)
  straddles the vertical band, rot 180: its secondary L2 (4-3: SEC2 top,
  CTRL_GND bottom) in the strip, its primary L1 (1-2: VOUT1 top, SW
  bottom) on the bus side; the band narrows to the 1.48 mm between the
  pad columns under the part.
  Secondary on F in the strip: D6 anode by pad 3, C30 / C31 under its
  cathode, the TVS D7 above the link headers J2 / J3. Primary: U2 right of
  L2 rot 180 (SW pin 8 level with L2 pad 1), C11 / C12 at its VIN / RTN
  pins, R12 (RON) above them, the UVLO divider on B behind them, the VOUT1
  cap C29 above L2 pad 2, the FB divider / Cac between L2 and U2 (R17 and
  the ripple R13 / C14 on B), BST C13 and VCC C16 on B under U2's left
  pins, D2 below it.
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import core as rev1                       # noqa: E402  (rev1 core table)
from boardfit import F, B, put, fit, link, solid, nosilk   # noqa: E402

CX, Y0 = rev1.CX, rev1.Y0                 # core origin (52, 100): the rev1 / rev D table's frame
LEFT, RIGHT = CX - 8.5, CX + 59.5         # board edges (CAN FD: 68 mm)
PIN1 = Y0 + 42.5                          # chain socket pin 1 (top) v 42.5, 7 rows at 2.54 to v 57.74
LINK1 = Y0 + 30.0                         # link plug / socket pin 1 (top) v 30.0 on every board (ADR 0006)
PAST = 4.0                                # socket face past the board edge (tools/sch/blocks/chain.py)
GONE = ('J4', 'Q1', 'R27', 'R28', 'R29', 'R30')     # rev D link parts without a CAN FD successor

TILE = ('R1', 'R2', 'Q2', 'Q3', 'R10', 'R11', 'C14', 'C15', 'U8', 'C16', 'U7', 'C3', 'R37',
        'R33', 'R34', 'R35', 'R36', 'Q4', 'R14', 'R12', 'R38', 'J1')
MOVED = {
    # mux by the header: rot 180 puts pins 1-12 (channels 0-3) on the right, pin 1 lowest
    'U6': (42.0, 51.0, 180, F), 'C28': (42.0, 45.6, 0, F),
    'Q6': (34.6, 51.6, 0, F), 'R31': (36.4, 47.4, 90, F), 'R32': (34.6, 55.0, 0, F),
    # press deadline (docs/press-deadline-line.md): monostable U301 and its RC, bottom right by J301's PRESS_EN pin
    'U301': (43.0, 60.0, 0, F), 'C302': (43.0, 57.2, 0, F), 'R301': (46.6, 60.0, 90, F), 'C301': (43.0, 63.6, 0, F),
    'R302': (39.6, 59.4, 90, F), 'R303': (39.6, 62.6, 90, F),
    # crystal turned so OSC_IN (pad 1) faces U3's OSC_IN pin and OSC_OUT runs round the bottom: no crossing
    'Y1': (3.0, 33.0, 270, B),
    # ARM 10-pin SWD header beside the Tag-Connect J5: right of U3, below the address switch SW1
    'J6': (25.0, 39.5, 0, F),
    'H1': (-4.5, 4.0, 0, F), 'H2': (-4.5, 68.0, 0, F), 'H3': (55.5, 4.0, 0, F), 'H4': (55.5, 68.0, 0, F),
}
# CAN FD link domain (island block, strip, edge channels) and the MCP2518FD
LINKP = {
    # ISO1044 where the ISO1640 was (side 2, pins 5-8, up in the island), VCC2 cap C25 island side, VCC1 cap
    # C24 bus side
    'U4': (6.0, 22.0, 90, F), 'C25': (2.3, 19.525, 180, F), 'C24': (2.3, 24.475, 180, F),
    # 5 V for side 2: HT7550-1 from ISO_12V, in / out caps; OR diode ISO_12V -> AUX_12V by the secondary
    'U7': (9.0, 13.0, 0, F), 'C27': (9.0, 8.6, 0, F), 'C26': (5.0, 17.0, 180, F),
    'D10': (17.0, 13.0, 180, F),
    # link ESD at each connector, the termination above the socket
    # the termination: 2x2 jumper-cap header JP1 (origin = pin 1 CAN_P; rot 90: pin 2 TERM_H above it, pins 3 / 4
    # CAN_N / TERM_L to the right), R25 / R26 over its TERM row, C32 above them (courtyards: D9 0.6 mm down)
    'D8': (-5.0, 25.0, 270, F), 'D9': (53.6, 25.6, 90, F),
    'JP1': (51.5, 21.8, 90, F),
    'R25': (51.5, 15.9, 90, F), 'R26': (54.04, 15.9, 270, F), 'C32': (52.77, 13.3, 0, F),
    # MCP2518FD bottom left (SPI on its right column, toward U3's PA4-PA7), crystal under OSC1 / OSC2
    'U5': (-2.0, 48.0, 0, F), 'C35': (2.6, 42.8, 0, F),
    'Y2': (-2.0, 55.6, 0, F), 'C33': (-5.6, 55.6, 90, F), 'C34': (1.6, 55.6, 90, F),
    'R33': (3.2, 46.0, 90, F), 'R34': (4.4, 49.0, 90, F), 'R35': (3.2, 51.0, 90, F), 'R36': (4.4, 53.0, 90, F),
}
# 12 V aux Fly-Buck: rev D placed the primary right of L2 (top right corner, table FLY_D); CAN FD turns L2
# and the primary 90 deg clockwise about L2, moved down to v 10 (fly()); the secondary stays in the strip
L2_D, L2_FD = (32.0, 5.0), (32.0, 10.0)
SECONDARY = ('D6', 'C30', 'C31', 'D7')
FLY_D = {
    'L2': (32.0, 5.0, 180, F),
    # secondary, in the island strip
    'D6': (25.6, 3.0, 0, F), 'C30': (21.0, 6.7, 270, F), 'C31': (23.1, 6.7, 270, F), 'D7': (14.5, 1.6, 0, F),
    # primary: U2 and its input caps, RON, UVLO divider
    'U2': (41.0, 5.4, 180, F), 'C11': (46.5, 6.0, 270, F), 'C12': (45.6, 9.8, 0, F), 'R12': (46.3, 1.6, 180, F),
    'R10': (46.6, 4.8, 90, B), 'R11': (44.6, 4.8, 90, B),
    # VOUT1 cap above L2 pad 2, FB divider / Cac between L2 and U2, ripple injection on B; BST / VCC caps under
    # U2's left pins
    'C29': (35.4, 1.2, 0, F), 'R14': (35.4, 3.7, 90, F), 'C15': (36.6, 3.7, 90, F), 'R17': (35.6, 6.8, 0, B),
    'R13': (35.2, 2.0, 0, B), 'C14': (35.0, 4.2, 0, B),
    'C13': (38.3, 6.7, 90, B), 'C16': (38.3, 3.9, 90, B),
    'D2': (41.0, 9.6, 0, F),
}


def fly():
    out = {}
    for ref, (u, v, rot, side) in FLY_D.items():
        if ref in SECONDARY:
            out[ref] = (u, v, rot, side)
            continue
        du, dv = u - L2_D[0], v - L2_D[1]
        out[ref] = (L2_FD[0] - dv, L2_FD[1] + du, (rot - 90) % 360, side)
    return out


FLY = fly()
SOLID = [('J1', '1'), ('J1', '2'), ('R4', '2'), ('U6', '21'), ('C20', '2'), ('R5', '2'),   # small GND pads: starved spokes
         ('U2', '9'),                     # U2's exposed pad and its thermal vias: solid to the GND pours
         ('U4', '7'), ('C23', '2')]       # CAN FD: LINK_GND / GND pads between the link tracks: starved spokes


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    table = {r: p for r, p in rev1.CORE.items() if r not in TILE + GONE}
    table.update(MOVED)
    table.update(FLY)
    table.update(LINKP)
    for ref, (u, v, rot, side) in table.items():
        put(board, ref, CX + u, Y0 + v, rot, side)
    link(board, 'J301', RIGHT, 'right', PIN1, PAST)
    link(board, 'J2', LEFT, 'left', LINK1)
    link(board, 'J3', RIGHT, 'right', LINK1, PAST)
    for ref in ('J301', 'J2', 'J3'):
        nosilk(board, ref)
    fit(board, 'J1', 180, F, cx=CX + 30.0, bottom=Y0 + 72.4)
    for ref, num in SOLID:
        solid(board, ref, num)
    left = [fp.GetReference() for fp in board.GetFootprints() if pcbnew.ToMM(fp.GetPosition().x) < LEFT - 1]
    board.Save(dst)
    print('core: placed', len(table) + 4, '; not placed:', ' '.join(sorted(left)) or '-')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
