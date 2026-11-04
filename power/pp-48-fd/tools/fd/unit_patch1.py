"""CAN FD unit, router clean-up after mazefix (hand): A_OVLO from R5 pad 2 by a via above it and an In1 hop under
the link band to the In1 hop mazefix made to R6; drop the GND via mazefix put onto UVLO by R214."""
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1]); M = pcbnew.FromMM
def via(net, x, y):
    v = pcbnew.PCB_VIA(b); v.SetPosition(pcbnew.VECTOR2I_MM(x, y)); v.SetWidth(M(0.5)); v.SetDrill(M(0.2))
    v.SetNet(b.FindNet(net)); v.SetIsFree(True); b.Add(v)
def trk(net, pts, w, lay):
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        t = pcbnew.PCB_TRACK(b); t.SetStart(pcbnew.VECTOR2I_MM(x0, y0)); t.SetEnd(pcbnew.VECTOR2I_MM(x1, y1))
        t.SetWidth(M(w)); t.SetLayer(lay); t.SetNet(b.FindNet(net)); b.Add(t)
t = b.Tracks()
for it in [t[i] for i in range(len(t))]:
    if it.Type() == pcbnew.PCB_VIA_T and it.GetNetname() == 'GND' and abs(pcbnew.ToMM(it.GetPosition().x) - 155.0) < 0.05 \
            and abs(pcbnew.ToMM(it.GetPosition().y) - 129.64) < 0.05:
        b.Delete(it)
# the mazefix via at R6 (In1 hop end)
end = None
for it in [t[i] for i in range(len(t))]:
    if it.Type() == pcbnew.PCB_VIA_T and it.GetNetname() == '/Unit/A_OVLO':
        x, y = pcbnew.ToMM(it.GetPosition().x), pcbnew.ToMM(it.GetPosition().y)
        if 111.8 < x < 112.8 and 123.5 < y < 124.5:
            end = (x, y)
print('R6 hop via', end)
trk('/Unit/A_OVLO', [(111.5, 119.5875), (111.5, 118.65)], 0.2, pcbnew.F_Cu)
via('/Unit/A_OVLO', 111.5, 118.65)
trk('/Unit/A_OVLO', [(111.5, 118.65), (111.5, end[1] - (end[0] - 111.5)), end], 0.2, pcbnew.In1_Cu)
b.Save(sys.argv[2])
