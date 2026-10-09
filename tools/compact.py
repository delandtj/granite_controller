#!/usr/bin/env python3
"""Shorten the board: re-place everything right of the MCU block, keep the rest.

Usage: compact.py <in.kicad_pcb> <out.kicad_pcb>

Power, Ethernet and MCU blocks (x < X_CUT) keep their placement and copper.
Everything right of X_CUT is re-placed (layout below) and its copper is
deleted; afterwards run krt_reroute.sh --keep on the open nets (plus GND and
+3V3 for the plane welds), then gndpour.py, silk.py, connlabels.py.

Layout right of the MCU block (x in board mm, y = 28.6 top edge):
  top edge   J6 (in the gap after J5) | J7 | J9 | J11-J14
  interior   protection + dry-input parts | U14 over U15 | 4 relay columns
  bottom     J8 (DNP) | J10 | J15-J18
Relay columns at PITCH: K pair at the left, one resistor strip at the right;
the top node's parts under its top connector, the bottom node's parts over
its bottom connector. The band between them carries U16 (nodes 1-4, inputs
up) and U17 (nodes 5-8, inputs down) with the 47k NODE_ON pull-ups on the output
side of each.
The right edge, H2/H3, the GND pours, planes and keep-outs move left by DX.
Same scratch-dir rule as mkboard/outline.
"""
import sys
import pcbnew

src, out = sys.argv[1:3]
X_CUT = 134.8        # copper with an end right of this is deleted
OLD_RIGHT = 275.0
PITCH = 13.6         # relay column pitch (JST PH 5p courtyard is 13.0)
X0 = 163.7           # pin 1 of J11/J15
NEW_RIGHT = round(X0 + 3 * PITCH + 10.49 + 9.0, 1)   # 9 mm keeps J14 pads off H2
DX = NEW_RIGHT - OLD_RIGHT
GAP = 0.5            # courtyard gap for packed small parts
STEP = 0.25

b = pcbnew.LoadBoard(src)
_fps = b.Footprints()
FP = {_fps[i].GetReference(): _fps[i] for i in range(len(_fps))}


def mm(v):
    return pcbnew.ToMM(v)


def V(x, y):
    return pcbnew.VECTOR2I_MM(x, y)


def cbox(fp):
    cy = fp.GetCourtyard(pcbnew.F_CrtYd)
    bb = cy.BBox() if cy.OutlineCount() else fp.GetBoundingBox(False)
    return mm(bb.GetLeft()), mm(bb.GetTop()), mm(bb.GetRight()), mm(bb.GetBottom())


# header-side nets of RN1/RN2 (rigid, but their copper all runs to J8): clear all
hdr = set()
for r in ('RN1', 'RN2'):
    P = FP[r].Pads()
    hdr |= {P[i].GetNetname() for i in range(len(P)) if P[i].GetNumber() in '1234'}
hdr = {n for n in hdr if 'HDR_' in n}

moved = {r for r, f in FP.items()
         if mm(f.GetPosition().x) > X_CUT and not r.startswith('H')} | {'J6'}

# --- copper right of the cut (and under J6's new spot) goes
t = b.Tracks()
items = [t[i] for i in range(len(t))]
gone = 0
for it in items:
    pts = [it.GetStart(), it.GetEnd()]
    if it.GetNetname() in hdr or any(mm(p.x) > X_CUT or (125.5 < mm(p.x) and mm(p.y) < 35.8) for p in pts):
        b.Delete(it)
        gone += 1
print('deleted', gone, 'tracks/vias')

# --- obstacles: everything that stays
placed = [cbox(f) for r, f in FP.items() if r not in moved and not r.startswith('H')]


def hit(bx):
    return any(not (bx[2] + GAP <= o[0] or o[2] + GAP <= bx[0] or
                    bx[3] + GAP <= o[1] or o[3] + GAP <= bx[1]) for o in placed)


def put(ref, x, y, rot=0):
    """Place by footprint origin."""
    fp = FP[ref]
    fp.SetOrientationDegrees(rot)
    fp.SetPosition(V(x, y))
    placed.append(cbox(fp))


def put_c(ref, cx, cy, rot=0):
    """Place by courtyard centre."""
    fp = FP[ref]
    fp.SetOrientationDegrees(rot)
    fp.SetPosition(V(0, 0))
    x0, y0, x1, y1 = cbox(fp)
    fp.SetPosition(V(cx - (x0 + x1) / 2, cy - (y0 + y1) / 2))
    placed.append(cbox(fp))


def pack(refs, region, rot=0):
    rx0, ry0, rx1, ry1 = region
    left = []
    for ref in refs:
        fp = FP[ref]
        fp.SetOrientationDegrees(rot)
        fp.SetPosition(V(0, 0))
        x0, y0, x1, y1 = cbox(fp)
        w, h = x1 - x0, y1 - y0
        done = False
        y = ry0
        while not done and y + h <= ry1 + 1e-6:
            x = rx0
            while x + w <= rx1 + 1e-6:
                bx = (x, y, x + w, y + h)
                if not hit(bx):
                    fp.SetPosition(V(x - x0, y - y0))
                    placed.append(bx)
                    done = True
                    break
                x += STEP
            y += STEP
        if not done:
            left.append(ref)
    return left


# --- connectors on the edges
YT, YB = 31.6, 74.8
put('J6', 128.6, YT)
put('J7', 138.2, YT)
put('J9', 147.8, YT)
put('J8', 138.8, 63.5)
put('J10', 150.1, YB)

# --- expanders, stacked between J9 and J10
put_c('U14', 154.6, 44.25, 0)
put_c('U15', 154.6, 63.15, 0)

# --- relay columns: top node c+1, bottom node c+5
YK = {0: (39.3, 44.8, 37.2), 1: (61.9, 67.4, 60.5)}   # K a, K b, strip top (room for pin legends)
for c in range(4):
    x0 = X0 + c * PITCH
    put(f'J{11 + c}', x0, YT)
    put(f'J{15 + c}', x0, YB)
    kx, sx = x0 + 1.84, x0 + 8.6
    for half in (0, 1):
        ya, yb, ys = YK[half]
        ka, kb = 2 * c + 1 + 8 * half, 2 * c + 2 + 8 * half
        put_c(f'K{ka}', kx, ya)
        put_c(f'K{kb}', kx, yb)
        r1a, r1b = 101 + 2 * c + 8 * half, 102 + 2 * c + 8 * half
        r2a, r2b = 201 + 2 * c + 8 * half, 202 + 2 * c + 8 * half
        rl = 47 + c + 4 * half                    # 220R PLED series
        for ref, y in ((f'R{r1a}', 0.0), (f'R{r1b}', 1.2), (f'R{rl}', 2.4),
                       (f'R{r2a}', 3.8), (f'R{r2b}', 5.6)):
            put_c(ref, sx, ys + y)


# --- optos in the middle band, inputs towards their connectors
def put_opto(ref, cx, cy, inputs_up):
    for rot in (90, 270):
        put_c(ref, cx, cy, rot)
        placed.pop()
        f = FP[ref]
        if (mm(f.FindPadByNumber('1').GetPosition().y) < cy) == inputs_up:
            break
    placed.append(cbox(f))


YM = 53.3
put_opto('U16', X0 + 0.5 * PITCH + 4.0, YM, True)
put_opto('U17', X0 + 2.5 * PITCH + 4.0, YM, False)
left = pack([f'R{39 + i}' for i in range(4)], (X0 + 0.5 * PITCH - 2.0, 57.9, X0 + 1.5 * PITCH + 6.0, 59.7))  # under U16's outputs
left += pack([f'R{43 + i}' for i in range(4)], (X0 + 2.5 * PITCH + 10.0, 48.0, X0 + 3.5 * PITCH + 6.0, 59.0))

# --- small parts: ESD next to its connector first, then the rest
left += pack(['U7'], (143.5, 62.0, 148.2, 67.0))            # J8
left += pack(['U18'], (143.5, 67.5, 148.2, 72.2), rot=180)  # J10, DRYC pads right
left += pack(['F2', 'F1', 'D7', 'D6', 'U6', 'C38', 'C46', 'R32', 'R33'],
             (135.4, 38.2, 148.2, 61.4))
# dry inputs: one channel per row (1k series, filter cap, pull-up)
left += pack(['R55', 'C47', 'R59', 'R56', 'C48', 'R60', 'R57', 'C49', 'R61',
              'R58', 'C50', 'R62'], (140.5, 47.5, 148.2, 61.4))
if left:
    print('NOT PLACED:', left)

# --- TPD4E05 centre GND pads 3+8: strap on F.Cu to a plane via (KRT cannot
#     reach them between the 0.5 mm pitch pads)
for ref in ('U7', 'U18'):
    f = FP[ref]
    p3, p8 = f.FindPadByNumber('3').GetPosition(), f.FindPadByNumber('8').GetPosition()
    net = f.FindPadByNumber('3').GetNet()
    k = 1.2 / abs(pcbnew.ToMM(p8.x - p3.x))   # extend pad 3 -> pad 8 by 1.2 mm
    vx = pcbnew.VECTOR2I(int(p8.x + (p8.x - p3.x) * k), int(p8.y + (p8.y - p3.y) * k))
    for a, z in ((p3, p8), (p8, vx)):
        t = pcbnew.PCB_TRACK(b); t.SetStart(a); t.SetEnd(z); t.SetWidth(pcbnew.FromMM(0.2))
        t.SetLayer(pcbnew.F_Cu); t.SetNet(net); b.Add(t)
    v = pcbnew.PCB_VIA(b); v.SetPosition(vx); v.SetWidth(pcbnew.FromMM(0.6)); v.SetDrill(pcbnew.FromMM(0.3))
    v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); v.SetNet(net); b.Add(v)

# --- right edge, holes, zones, keep-outs
def shift_pt(p):
    return V(mm(p.x) + DX, mm(p.y)) if mm(p.x) > 200 else p


d = b.Drawings()
for s in [d[i].Cast() for i in range(len(d))]:
    if s.GetLayer() != pcbnew.Edge_Cuts:
        continue
    if s.GetShape() == pcbnew.SHAPE_T_ARC:
        s.SetArcGeometry(shift_pt(s.GetStart()), shift_pt(s.GetArcMid()), shift_pt(s.GetEnd()))
    else:
        s.SetStart(shift_pt(s.GetStart()))
        s.SetEnd(shift_pt(s.GetEnd()))
for r in ('H2', 'H3'):
    p = FP[r].GetPosition()
    FP[r].SetPosition(V(mm(p.x) + DX, mm(p.y)))
z = b.Zones()
for zone in [z[i] for i in range(len(z))]:
    o = zone.Outline()
    for i in range(o.TotalVertices()):
        p = o.CVertex(i)
        if mm(p.x) > 200:
            o.SetVertex(i, V(mm(p.x) + DX, mm(p.y)))
    zone.UnFill()

# --- escapes KRT does not find between 0.5 mm pitch pads: a stub to a via.
#     RN1 pad 3 as on rev C; TPD4E05 pads 2 and 4 are boxed in by 1/3/5, so
#     each gets a via 1.15 mm out on the signal side (away from the GND strap)
ESC = [('RN1', '3', 134.7, 68.25)]
for ref in ('U6', 'U7', 'U18'):
    f = FP[ref]
    p3, p8 = f.FindPadByNumber('3').GetPosition(), f.FindPadByNumber('8').GetPosition()
    k = 1.15 / abs(mm(p3.x - p8.x))
    for num in ('2', '4'):
        q = f.FindPadByNumber(num)
        if q.GetNetname().startswith('unconnected'):
            continue
        c = q.GetPosition()
        ESC.append((ref, num, mm(c.x + (p3.x - p8.x) * k), mm(c.y + (p3.y - p8.y) * k)))
for ref, num, vx, vy in ESC:
    pad = FP[ref].FindPadByNumber(num)
    t = pcbnew.PCB_TRACK(b); t.SetStart(pad.GetPosition()); t.SetEnd(V(vx, vy))
    t.SetWidth(pcbnew.FromMM(0.2)); t.SetLayer(pcbnew.F_Cu); t.SetNet(pad.GetNet()); b.Add(t)
    v = pcbnew.PCB_VIA(b); v.SetPosition(V(vx, vy)); v.SetWidth(pcbnew.FromMM(0.5)); v.SetDrill(pcbnew.FromMM(0.2))
    v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); v.SetNet(pad.GetNet()); b.Add(v)

b.Save(out)
print(f'right edge {OLD_RIGHT} -> {NEW_RIGHT} (DX {DX:+.1f}), board length {NEW_RIGHT - 25.0:.1f} mm')
