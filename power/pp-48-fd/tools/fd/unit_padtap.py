"""padtap.py <in> <out> REF.PAD ...: a GND via (0.5/0.2) next to each listed pad on its layer, with a 0.3 mm
track, at the nearest spot that clears every other-net item on every copper layer (0.2 mm; link nets 0.5)."""
import math, sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1]); MM = pcbnew.FromMM; T = pcbnew.ToMM
gnd = b.FindNet('GND')
grp = ([g for g in b.Groups() if g.GetName() == 'fd-taps'] or [None])[0]
if grp is None:
    grp = pcbnew.PCB_GROUP(b); grp.SetName('fd-taps'); b.Add(grp)
t = b.Tracks(); tracks = [t[i] for i in range(len(t))]
pads = [p for fp in b.GetFootprints() for p in fp.Pads()]
LAY = (pcbnew.F_Cu, pcbnew.In1_Cu, pcbnew.In2_Cu, pcbnew.B_Cu)
def clr(net): return 0.5 if net.startswith('/Chain/') else 0.2
def ok_via(x, y):
    c = pcbnew.VECTOR2I(MM(x), MM(y))
    for it in tracks + pads:
        if it.GetNetCode() == gnd.GetNetCode(): continue
        for l in LAY:
            if not it.IsOnLayer(l): continue
            sh = it.GetEffectiveShape(l)
            if sh.Collide(pcbnew.SEG(c, c), MM(0.25 + clr(it.GetNetname()))): return False
    for p in pads:
        if p.GetDrillSize().x > 0 and math.hypot(T(p.GetPosition().x) - x, T(p.GetPosition().y) - y) < T(p.GetDrillSize().x) / 2 + 0.1 + 0.42: return False
    return True
def ok_trk(a, c, lay):
    s = pcbnew.SEG(pcbnew.VECTOR2I(MM(a[0]), MM(a[1])), pcbnew.VECTOR2I(MM(c[0]), MM(c[1])))
    for it in tracks + pads:
        if it.GetNetCode() == gnd.GetNetCode() or not it.IsOnLayer(lay): continue
        if it.GetEffectiveShape(lay).Collide(s, MM(0.15 + clr(it.GetNetname()))): return False
    return True
for spec in sys.argv[3:]:
    ref, num = spec.split('.')
    pad = [p for p in b.FindFootprintByReference(ref).Pads() if p.GetNumber() == num][0]
    lay = pcbnew.B_Cu if pad.IsOnLayer(pcbnew.B_Cu) and not pad.IsOnLayer(pcbnew.F_Cu) else pcbnew.F_Cu
    px, py = T(pad.GetPosition().x), T(pad.GetPosition().y)
    best = None
    for r10 in range(9, 25):
        r = r10 / 10
        for k in range(24):
            x, y = px + r * math.cos(k * math.pi / 12), py + r * math.sin(k * math.pi / 12)
            if not pad.HitTest(pcbnew.VECTOR2I(MM(x), MM(y)), MM(0.3)) and ok_via(x, y) and ok_trk((px, py), (x, y), lay):
                best = (x, y); break
        if best: break
    print(spec, 'via at', best)
    if not best: continue
    v = pcbnew.PCB_VIA(b); v.SetPosition(pcbnew.VECTOR2I(MM(best[0]), MM(best[1]))); v.SetWidth(MM(0.5)); v.SetDrill(MM(0.2)); v.SetNet(gnd); v.SetIsFree(True); b.Add(v)
    tr = pcbnew.PCB_TRACK(b); tr.SetStart(pad.GetPosition()); tr.SetEnd(v.GetPosition()); tr.SetWidth(MM(0.3)); tr.SetLayer(lay); tr.SetNet(gnd); b.Add(tr)
    grp.AddItem(v); grp.AddItem(tr)
    tracks += [v, tr]
b.Save(sys.argv[2])
