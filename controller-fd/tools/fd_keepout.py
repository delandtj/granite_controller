#!/usr/bin/env python3
"""Temporary track / via keep-out that steers AUX_12V round U1's escapes.

Usage: fd_keepout.py add|del <board.kicad_pcb>      (in place)

AUX_12V comes in on J15 at the right edge and goes to the OR diode D10 at
the far left. Every escape of U1's right pad column (pins 3-14) climbs the
3 mm channel between U1 and J15, or runs under U1 on B.Cu, toward the top
of the board; an AUX_12V track across that channel or under U1's upper
part boxes them in (KRT then leaves I2C_LOC_SCL open). With this keep-out
(U1's channel and the module from its top edge down to just below the
GND thermal pad, F.Cu and B.Cu) in place, krt-reroute takes AUX_12V below
U1's thermal pad instead; `del` removes it before the other nets route.
"""
import sys
import pcbnew

MM = pcbnew.FromMM
NAME = "tmp AUX_12V steering keep-out"
# x0, y0, x1, y1, layers: U1 and its channel (both layers); left of U1
# of it F.Cu only from the W5500 bus down (the bus, ETH_RST, U1 pins 26 / 27
# and the J9 strip are F.Cu there): AUX_12V crosses on B.Cu
BOXES = [((97.5, 52.0, 121.3, 73.6), (pcbnew.F_Cu, pcbnew.B_Cu)),
         ((52.0, 66.0, 97.5, 78.6), (pcbnew.F_Cu,))]


def main(argv):
    mode, path = argv
    b = pcbnew.LoadBoard(path)
    old = [z for z in b.Zones() if z.GetZoneName() == NAME]
    for z in old:
        b.Remove(z)
    for (x0, y0, x1, y1), layers in BOXES if mode == "add" else ():
        z = pcbnew.ZONE(b)
        z.SetIsRuleArea(True)
        z.SetDoNotAllowTracks(True)
        z.SetDoNotAllowVias(True)
        z.SetDoNotAllowPads(False)
        z.SetDoNotAllowZoneFills(False)
        z.SetDoNotAllowFootprints(False)
        ls = pcbnew.LSET()
        for L in layers:
            ls.AddLayer(L)
        z.SetLayerSet(ls)
        z.SetZoneName(NAME)
        o = z.Outline()
        o.NewOutline()
        for x, y in ((x0, y0), (x1, y0), (x1, y1), (x0, y1)):
            o.Append(MM(x), MM(y))
        b.Add(z)
    b.Save(path)
    print("keep-out", "added" if mode == "add" else "removed")


if __name__ == "__main__":
    main(sys.argv[1:])
