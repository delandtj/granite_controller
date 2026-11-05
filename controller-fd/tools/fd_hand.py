#!/usr/bin/env python3
"""Plane welds for the FD board's new / moved parts, in group "hand".

Usage: fd_hand.py <board.kicad_pcb>      (in place; after fd_board.py + relink)

Every SMD pad on GND or +3V3 of the parts fd_board.py added or moved gets a
plane via (0.6 / 0.3) at its centre; `pcbkit viaout` then moves each one
just outside its pad with a stub (the same way KRT's plane welds are
cleaned on rev D). U1's local I2C pins get hand escapes. U6's centre GND pads 3 / 8 are strapped under the body
as rev D's route_hand.py does for the TPD4E05U06 arrays.
"""
import sys
import pcbnew

MM = pcbnew.FromMM
PARTS = "U23 U24 Y3 D13 C57 C58 C59 C60 C61 C62 C63 R75 R76 U6 D6 F1 RN1".split()
PLANE = {"GND", "+3V3"}
# pad pairs strapped on F.Cu: the second pad takes no via of its own
STRAPS = [(("C57", "1"), ("R75", "1"))]
SOLID = [("U23", "7"), ("U24", "2"), ("Y3", "2"), ("Y3", "4"), ("D13", "3"),
         ("U6", "3"), ("U6", "8"), ("J15", "3"), ("J15", "4")]


def main(argv):
    b = pcbnew.LoadBoard(argv[0])
    nets = b.GetNetsByName()
    grp = [g for g in b.Groups() if g.GetName() == "hand"][0]
    n = 0
    skip = {b2 for _, b2 in STRAPS}

    def pad(ref, num):
        return [p for p in b.FindFootprintByReference(ref).Pads() if p.GetNumber() == num][0]
    def line(net, layer, pts, w=0.2):
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(b)
            t.SetStart(pcbnew.VECTOR2I(MM(x0), MM(y0)))
            t.SetEnd(pcbnew.VECTOR2I(MM(x1), MM(y1)))
            t.SetWidth(MM(w))
            t.SetLayer(layer)
            t.SetNet(nets[net])
            b.Add(t)
            grp.AddItem(t)

    def dot(net, x, y, d, h):
        v = pcbnew.PCB_VIA(b)
        v.SetPosition(pcbnew.VECTOR2I(MM(x), MM(y)))
        v.SetWidth(MM(d))
        v.SetDrill(MM(h))
        v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        v.SetNet(nets[net])
        b.Add(v)
        grp.AddItem(v)

    for a, c in STRAPS:
        pa, pc = pad(*a), pad(*c)
        t = pcbnew.PCB_TRACK(b)
        t.SetStart(pa.GetPosition())
        t.SetEnd(pc.GetPosition())
        t.SetWidth(MM(0.3))
        t.SetLayer(pcbnew.F_Cu)
        t.SetNet(pa.GetNet())
        b.Add(t)
        grp.AddItem(t)
    # C62 (VIO 100n) straight onto U24 pin 5 below it, its own plane vias
    # beside it (the gap to J15 pin 1 is too tight for viaout)
    c1, c2, vio = pad("C62", "1"), pad("C62", "2"), pad("U24", "5")
    x1, y1 = c1.GetPosition().x, c1.GetPosition().y
    x2, y2 = c2.GetPosition().x, c2.GetPosition().y
    for net, a, vx in (("+3V3", c1.GetPosition(), x1 - MM(1.0)), ("GND", c2.GetPosition(), x2 + MM(0.95))):
        t = pcbnew.PCB_TRACK(b)
        t.SetStart(a)
        t.SetEnd(pcbnew.VECTOR2I(vx, a.y))
        t.SetWidth(MM(0.3))
        t.SetLayer(pcbnew.F_Cu)
        t.SetNet(nets[net])
        b.Add(t)
        grp.AddItem(t)
        v = pcbnew.PCB_VIA(b)
        v.SetPosition(pcbnew.VECTOR2I(vx, a.y))
        v.SetWidth(MM(0.6))
        v.SetDrill(MM(0.3))
        v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        v.SetNet(nets[net])
        b.Add(v)
        grp.AddItem(v)
    t = pcbnew.PCB_TRACK(b)
    t.SetStart(c1.GetPosition())
    t.SetEnd(vio.GetPosition())
    t.SetWidth(MM(0.3))
    t.SetLayer(pcbnew.F_Cu)
    t.SetNet(nets["+3V3"])
    b.Add(t)
    grp.AddItem(t)
    skip |= {("C62", "1"), ("C62", "2"), ("U24", "5")}
    for r in PARTS:
        fp = b.FindFootprintByReference(r)
        for p in fp.Pads():
            net = str(p.GetNetname())
            if net not in PLANE or p.HasHole():
                continue
            if (r, p.GetNumber()) in skip or (r == "U6" and p.GetNumber() == "8"):
                continue          # strapped to pad 3 below
            v = pcbnew.PCB_VIA(b)
            v.SetPosition(p.GetPosition())
            v.SetWidth(MM(0.6))
            v.SetDrill(MM(0.3))
            v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
            v.SetNet(nets[net])
            b.Add(v)
            grp.AddItem(v)
            n += 1
    u6 = {p.GetNumber(): p.GetPosition() for p in b.FindFootprintByReference("U6").Pads()}
    t = pcbnew.PCB_TRACK(b)
    t.SetStart(u6["3"])
    t.SetEnd(u6["8"])
    t.SetWidth(MM(0.2))
    t.SetLayer(pcbnew.F_Cu)
    t.SetNet(nets["GND"])
    b.Add(t)
    grp.AddItem(t)
    # U1 local I2C escapes (pins 6 / 7): KRT boxes pin 7 in among the other
    # right-column escapes, so both leave by hand: F stub to a via, B.Cu
    # diagonals under the module (parallel, 0.9 mm apart), then north to
    # the module's top edge; KRT finishes to U5 / R13 / R14 / RN1
    for net, py, xt in (("I2C_LOC_SCL", 69.49, 111.4), ("I2C_LOC_SDA", 70.76, 110.0)):
        pts = [(116.75, py), (118.0, py)]
        line(net, pcbnew.F_Cu, pts)
        dot(net, 118.0, py, 0.5, 0.2)
        line(net, pcbnew.B_Cu, [(118.0, py), (xt, py - (118.0 - xt)), (xt, 59.0)])

    # solid pour connection where KiCad's thermal spokes come out starved
    # (ICs / crystal / socket GND pads between the new routes)
    for r, num in SOLID:
        for p in b.FindFootprintByReference(r).Pads():
            if p.GetNumber() == num:
                p.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
    b.Save(argv[0])
    print("%d plane vias in pads (run pcbkit viaout next), U6 GND strap" % n)


if __name__ == "__main__":
    main(sys.argv[1:])
