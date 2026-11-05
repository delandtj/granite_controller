#!/usr/bin/env python3
"""unit-fd: the right-end termination from two solder jumpers (JP301 / JP302) to the 2x2 jumper-cap header JP301
(ADR 0006 decision 8), on the routed board, after fdsync.py. Usage: unit_term.py <in.kicad_pcb> <out.kicad_pcb>

Deletes the margin's termination copper (TERM_*, CANH / CANL on F and their vias, the LINK_GND F run and via),
places the TERM table of tools/place_unit.py and redraws it with tools/route_unit.py link_term() (group "hand").
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, HERE)
import place_unit as pu                  # noqa: E402
import route_unit as ru                  # noqa: E402

src, dst = sys.argv[1:3]
b = pcbnew.LoadBoard(src)
mm = pcbnew.ToMM
TERM = ('/Chain/TERM_H', '/Chain/TERM_L', '/Chain/TERM_MID')
t = b.Tracks()
gone = 0
for it in [t[i] for i in range(len(t))]:
    n = it.GetNetname()
    via = it.Type() == pcbnew.PCB_VIA_T
    if via:
        x, y = mm(it.GetPosition().x), mm(it.GetPosition().y)
    else:
        x = min(mm(it.GetStart().x), mm(it.GetEnd().x))
        y = min(mm(it.GetStart().y), mm(it.GetEnd().y))
    front = via or it.GetLayer() == pcbnew.F_Cu
    if n in TERM or (front and x < 100 and (n in ('/Chain/CANH', '/Chain/CANL')
                                            or (n == '/Chain/LINK_GND' and y < 125))):
        b.Delete(it)
        gone += 1
print('deleted', gone)
for ref, (x, y, rot, side) in pu.TERM.items():
    pu.put(b, ref, x, y, rot, side)
grp = [g for g in b.Groups() if g.GetName() == ru.GROUP][0]
ru.link_term(ru.Abs(b, grp))
b.Save(dst)
