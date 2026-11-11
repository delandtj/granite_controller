#!/usr/bin/env python3
"""core-fd: the right-end termination from two solder jumpers (JP1 / JP2) to the 2x2 jumper-cap header JP1
(ADR 0006 decision 8), on the routed board, after fdsync.py. Usage: core_term.py <in.kicad_pcb> <out.kicad_pcb>

Places D9, JP1, R25 / R26 / C32 from tools/place_core.py LINKP, deletes the old copper of CAN_P / CAN_N / TERM_*
in the right link channel (x 101.6-111.2, y 108-130.5; the arrivals from the strip stay), AUX_12V's B run down
the channel and the LINK_GND via under the old D9, and draws the new copper (group "hand"):
- CAN_P on F left of R25 (x 102.2) down to JP1.1, on past D9.1 to J3.1
- CAN_N on F at x 108.4 down to JP1.3, on past D9.2 to J3.2
- TERM_H / TERM_L straight up from JP1.2 / JP1.4 to R25.1 / R26.2, TERM_MID R25.2 - R26.1 and up to C32.1
- AUX_12V on B at x 109.8, right of the header's pins (not under CAN_N)
- LINK_GND via above D9.3
Also a pin 1 silk mark (triangle + "1", F and B) at the bare link plug J2.
"""
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, HERE)
import place_core as pc                  # noqa: E402
from boardfit import put                 # noqa: E402

mm, MM = pcbnew.ToMM, pcbnew.FromMM
src, dst = sys.argv[1:3]
b = pcbnew.LoadBoard(src)
grp = [g for g in b.Groups() if g.GetName() == 'hand'][0]
TERM = ('/Link/TERM_H', '/Link/TERM_L', '/Link/TERM_MID')


def inside(x, y):
    return 101.6 <= x <= 111.2 and 108.0 <= y <= 130.5


t = b.Tracks()
gone = 0
for it in [t[i] for i in range(len(t))]:
    n = it.GetNetname()
    if it.Type() == pcbnew.PCB_VIA_T:
        x, y = mm(it.GetPosition().x), mm(it.GetPosition().y)
        old = (n in TERM + ('/Link/CAN_P', '/Link/CAN_N') and inside(x, y)) or \
              (n == 'LINK_GND' and 104.0 <= x <= 107.0 and 120.0 <= y <= 124.0)
    else:
        (x1, y1), (x2, y2) = [(mm(p.x), mm(p.y)) for p in (it.GetStart(), it.GetEnd())]
        old = (n in TERM + ('/Link/CAN_P', '/Link/CAN_N') and inside(x1, y1) and inside(x2, y2)) or \
              (n == 'AUX_12V' and it.GetLayer() == pcbnew.B_Cu and min(x1, x2) > 101.6 and 107.0 <= min(y1, y2)
               and max(y1, y2) <= 124.0)
    if old:
        b.Delete(it)
        gone += 1
print('deleted', gone)

for ref in ('D9', 'JP1', 'R25', 'R26', 'C32'):
    u, v, rot, side = pc.LINKP[ref]
    put(b, ref, pc.CX + u, pc.Y0 + v, rot, side)


def pad(ref, num):
    p = b.FindFootprintByReference(ref).FindPadByNumber(num).GetPosition()
    return (mm(p.x), mm(p.y))


def track(net, pts, w, layer=pcbnew.F_Cu):
    for a, c in zip(pts, pts[1:]):
        s = pcbnew.PCB_TRACK(b)
        s.SetStart(pcbnew.VECTOR2I(MM(a[0]), MM(a[1])))
        s.SetEnd(pcbnew.VECTOR2I(MM(c[0]), MM(c[1])))
        s.SetWidth(MM(w))
        s.SetLayer(layer)
        s.SetNet(b.FindNet(net))
        b.Add(s)
        grp.AddItem(s)


def via(net, p, d=0.5, drill=0.2):
    v = pcbnew.PCB_VIA(b)
    v.SetPosition(pcbnew.VECTOR2I(MM(p[0]), MM(p[1])))
    v.SetWidth(MM(d))
    v.SetDrill(MM(drill))
    v.SetNet(b.FindNet(net))
    b.Add(v)
    grp.AddItem(v)


h, th = pad('JP1', '1'), pad('JP1', '2')       # CAN_P, TERM_H
l, tl = pad('JP1', '3'), pad('JP1', '4')       # CAN_N, TERM_L
dh, dl, dg = pad('D9', '1'), pad('D9', '2'), pad('D9', '3')
jh, jl = pad('J3', '1'), pad('J3', '2')
XH, XL = 102.2, 108.4                          # CAN_P left of R25, CAN_N right of R26
track('/Link/CAN_P', [(103.6, 109.1), (XH, 110.5), (XH, h[1]), h], 0.2)
track('/Link/CAN_P', [h, (dh[0], h[1] + dh[0] - h[0]), dh], 0.2)
track('/Link/CAN_P', [dh, (dh[0], dh[1] + 0.66), (jh[0], dh[1] + 0.66 + dh[0] - jh[0]), jh], 0.2)
track('/Link/CAN_N', [(XL, 111.7), (XL, l[1] - 0.9), (XL - 0.9, l[1]), l], 0.2)
track('/Link/CAN_N', [l, (dl[0], l[1] + dl[0] - l[0]), dl], 0.2)
track('/Link/CAN_N', [dl, (dl[0], dl[1] + 0.66), (jl[0], dl[1] + 0.66 + dl[0] - jl[0]), jl], 0.2)
track('/Link/TERM_H', [pad('R25', '1'), th], 0.2)
track('/Link/TERM_L', [pad('R26', '2'), tl], 0.2)
track('/Link/TERM_MID', [pad('R25', '2'), pad('R26', '1')], 0.2)
cm = pad('C32', '1')
track('/Link/TERM_MID', [cm, (cm[0], pad('R25', '2')[1])], 0.2)
track('AUX_12V', [(106.1, 107.2), (109.8, 110.9), (109.8, 122.0), (108.2, 123.6)], 0.5, pcbnew.B_Cu)
vg = (dg[0], dg[1] - 1.26)
via('LINK_GND', vg)
track('LINK_GND', [vg, dg], 0.3)


def pin1_mark(layer, tip, d, txt):
    """filled triangle pointing at the pin 1 row (d = -1: tip left), '1' above it; board silk, group 'hand'
    (the connector's own silk is gone: boardfit.nosilk)"""
    x, y = tip
    s = pcbnew.PCB_SHAPE(b, pcbnew.SHAPE_T_POLY)
    s.SetPolyPoints([pcbnew.VECTOR2I(MM(px), MM(py)) for px, py in
                     ((x, y), (x - d * 1.2, y - 0.6), (x - d * 1.2, y + 0.6))])
    s.SetFilled(True)
    s.SetWidth(MM(0.1))
    s.SetLayer(layer)
    b.Add(s)
    grp.AddItem(s)
    t = pcbnew.PCB_TEXT(b)
    t.SetText('1')
    t.SetLayer(layer)
    t.SetTextSize(pcbnew.VECTOR2I(MM(1.0), MM(1.0)))
    t.SetTextThickness(MM(0.15))
    t.SetPosition(pcbnew.VECTOR2I(MM(txt[0]), MM(txt[1])))
    if layer == pcbnew.B_SilkS:
        t.SetMirrored(True)
    b.Add(t)
    grp.AddItem(t)


# pin 1 of the bare link plug J2 (the controller's IDC ribbon plugs onto it): F at the edge, pointing at the
# protruding pin row; B inboard of pin 1 (the plug's body covers the edge strip on B)
p1 = pad('J2', '1')
pin1_mark(pcbnew.F_SilkS, (44.3, p1[1]), -1, (44.9, p1[1] - 1.6))
pin1_mark(pcbnew.B_SilkS, (p1[0] + 1.5, p1[1]), -1, (p1[0] + 2.1, p1[1] - 1.6))
b.Save(dst)
