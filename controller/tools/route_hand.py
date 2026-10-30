#!/usr/bin/env python3
"""Hand routes for rev D, added to group "hand" (kept by krt-route).

Usage: route_hand.py <board.kicad_pcb>      (in place)

W5500 SPI (rev C's bus, shortened): U3's bottom pins fan down at 0.4 mm
pitch, the five lines run east on F.Cu at 0.8 mm pitch (y 70.0 - 73.2),
drop to B.Cu at a via column x 92.6 - 96.6 (the U3 edge order is the
reverse of U1's west column), rise on B.Cu and enter U1's pins on F.Cu.
ETH_RST runs straight on F.Cu at y 68.22 as on rev C.

J10 dry-contact rows: the straight DRYC / DRY_IN shorts and the pull-up /
capacitor plane vias.

ESD arrays: GND strap between the centre pads 3 and 8 of U6 / U18 /
U19 / U20.
"""
import sys
import pcbnew

MM = pcbnew.FromMM
W = 0.2
U1X = 99.25            # U1 west pad column (rev C 106.25, block moved -7)

# net: (U3 pin x, fan y, bus y, via x, U1 pin y)
SPI = {
    "ETH_CS":   (47.75, 71.45, 73.2, 92.6, 61.87),
    "ETH_SCLK": (48.25, 71.05, 72.4, 93.6, 63.14),
    "ETH_MOSI": (49.25, 70.25, 70.8, 94.6, 64.41),
    "ETH_MISO": (48.75, 70.65, 71.6, 95.6, 65.68),
    "ETH_INT":  (49.75, 69.85, 70.0, 96.6, 66.95),
}
U3Y = 68.763
FAN_X = 52.75          # end of the 45 degree step from fan y to bus y


def main(argv):
    """route_hand.py <board> [--only NET ...]: with --only, delete the
    named nets' copper and redraw just their hand routes (re-apply after
    krt-route, which can rip hand routes)."""
    only = set(argv[2:]) if len(argv) > 1 and argv[1] == "--only" else None
    b = pcbnew.LoadBoard(argv[0])
    nets = b.GetNetsByName()
    grp = [g for g in b.Groups() if g.GetName() == "hand"][0]
    if only:
        for t in list(b.GetTracks()):
            if str(t.GetNetname()) in only:
                b.Remove(t)

    def trk(net, layer, pts, w=W):
        if only and net not in only:
            return
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
            t = pcbnew.PCB_TRACK(b)
            t.SetStart(pcbnew.VECTOR2I(MM(x0), MM(y0)))
            t.SetEnd(pcbnew.VECTOR2I(MM(x1), MM(y1)))
            t.SetWidth(MM(w))
            t.SetLayer(layer)
            t.SetNet(nets[net])
            b.Add(t)
            grp.AddItem(t)

    def via(net, x, y, d=0.5, h=0.2):
        if only and net not in only:
            return
        v = pcbnew.PCB_VIA(b)
        v.SetPosition(pcbnew.VECTOR2I(MM(x), MM(y)))
        v.SetWidth(MM(d))
        v.SetDrill(MM(h))
        v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
        v.SetNet(nets[net])
        b.Add(v)
        grp.AddItem(v)

    F, B = pcbnew.F_Cu, pcbnew.B_Cu
    for net, (px, fy, by, vx, uy) in SPI.items():
        step = by - fy
        trk(net, F, [(px, U3Y), (px, fy), (FAN_X - step, fy), (FAN_X, by), (vx, by)])
        via(net, vx, by)
        trk(net, B, [(vx, by), (vx, uy)])
        via(net, vx, uy)
        trk(net, F, [(vx, uy), (U1X, uy)])
    trk("ETH_RST", F, [(51.162, 67.35), (52.0, 67.35), (52.87, 68.22), (U1X, 68.22)])
    if only:            # re-apply mode covers the SPI bus only
        b.Save(argv[0])
        print("re-applied", " ".join(sorted(only)))
        return

    # TPD4E05U06 (USON-10): the centre GND pads 3 and 8 face each other;
    # strap them under the body (KRT cannot reach pad 8)
    # and drop a plane via 0.8 mm beyond pad 8 (U6: beyond pad 3, pad 8
    # faces J9)
    for ref, far in (("U6", "3"), ("U18", "8"), ("U19", "8"), ("U20", "8")):
        f = b.FindFootprintByReference(ref)
        p = {q.GetNumber(): (pcbnew.ToMM(q.GetPosition().x), pcbnew.ToMM(q.GetPosition().y)) for q in f.Pads()}
        a, c = p["3"], p["8"]
        trk("GND", F, [a, c])
        o, n = (c, a) if far == "8" else (a, c)
        dx, dy = o[0] - n[0], o[1] - n[1]
        L = (dx * dx + dy * dy) ** 0.5
        vx, vy = o[0] + dx / L * 0.8, o[1] + dy / L * 0.8
        trk("GND", F, [o, (vx, vy)], 0.3)
        via("GND", vx, vy, 0.6, 0.3)

    # J10 dry-contact rows: J10 pin -> 10k pull-up -> 1k on one straight
    # DRYC line, 1k -> C on DRY_IN, pull-up +3V3 and C GND to plane vias
    def pxy(ref, num):
        p = [q for q in b.FindFootprintByReference(ref).Pads() if q.GetNumber() == num][0]
        return pcbnew.ToMM(p.GetPosition().x), pcbnew.ToMM(p.GetPosition().y)
    for pin, r1k, cap, r10k in (("1", "R55", "C47", "R59"), ("2", "R56", "C48", "R60"),
                                ("3", "R57", "C49", "R61"), ("4", "R58", "C50", "R62")):
        net = str(b.FindFootprintByReference(r1k).Pads()[1].GetNetname())
        jx, jy = pxy("J10", pin)
        trk(net, F, [(jx, jy), pxy(r1k, "2")])
        trk(str(b.FindFootprintByReference(r1k).Pads()[0].GetNetname()), F, [pxy(r1k, "1"), pxy(cap, "1")])
        px, py = pxy(r10k, "1")
        trk("+3V3", F, [(px, py), (px + 0.9, py)], 0.4)
        via("+3V3", px + 0.9, py, 0.6, 0.3)
        gx, gy = pxy(cap, "2")
        trk("GND", F, [(gx, gy), (gx - 0.85, gy)], 0.4)
        via("GND", gx - 0.85, gy, 0.6, 0.3)

    b.Save(argv[0])
    print("hand routes added")


if __name__ == "__main__":
    main(sys.argv[1:])
