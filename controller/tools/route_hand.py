#!/usr/bin/env python3
"""Hand routes for rev D, added to group "hand" (kept by krt-route).

Usage: route_hand.py <board.kicad_pcb>      (in place)

W5500 SPI (rev C's bus, shortened): U3's bottom pins fan down at 0.4 mm
pitch, the five lines run east on F.Cu at 0.8 mm pitch (y 70.0 - 73.2),
drop to B.Cu at a via column x 92.6 - 96.6 (the U3 edge order is the
reverse of U1's west column), rise on B.Cu and enter U1's pins on F.Cu.
ETH_RST runs straight on F.Cu at y 68.22 as on rev C.

Plane link I2C: PLANE_SCL / PLANE_SDA leave U1 pins 26 / 27 on F.Cu
westwards below the SPI via column and drop to B.Cu at (91.2, 73.85) /
(90.2, 73.85), clear of J13 pin 5; KRT takes them on from there.
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
    b = pcbnew.LoadBoard(argv[0])
    nets = b.GetNetsByName()
    grp = [g for g in b.Groups() if g.GetName() == "hand"][0]

    def trk(net, layer, pts, w=W):
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

    trk("PLANE_SCL", F, [(U1X, 74.57), (91.92, 74.57), (91.2, 73.85)])
    via("PLANE_SCL", 91.2, 73.85)
    trk("PLANE_SDA", F, [(U1X, 75.84), (90.2, 75.84), (90.2, 73.85)])
    via("PLANE_SDA", 90.2, 73.85)

    b.Save(argv[0])
    print("hand routes added")


if __name__ == "__main__":
    main(sys.argv[1:])
