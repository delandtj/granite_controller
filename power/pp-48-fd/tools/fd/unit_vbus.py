"""CAN FD unit: VBUS taps of slot pads whose In2 vias stood in the link band (route_unit.py link()): one via
each just above the band, a short F track from the router's stub. Usage: vbus.py <in> <out>"""
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1]); ni = b.FindNet('VBUS')
def mm(v): return pcbnew.FromMM(v)
for (x0, y0), (x1, y1) in (((109.3, 119.5), (109.3, 118.7)), ((135.5, 120.5), (135.5, 118.7)), ((141.1, 118.7), (141.1, 118.7))):
    if (x0, y0) != (x1, y1):
        t = pcbnew.PCB_TRACK(b); t.SetStart(pcbnew.VECTOR2I(mm(x0), mm(y0))); t.SetEnd(pcbnew.VECTOR2I(mm(x1), mm(y1)))
        t.SetWidth(mm(0.3)); t.SetLayer(pcbnew.F_Cu); t.SetNet(ni); b.Add(t)
    v = pcbnew.PCB_VIA(b); v.SetPosition(pcbnew.VECTOR2I(mm(x1), mm(y1))); v.SetWidth(mm(0.5)); v.SetDrill(mm(0.2)); v.SetNet(ni); b.Add(v)
b.Save(sys.argv[2])
