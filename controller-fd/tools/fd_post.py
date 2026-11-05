#!/usr/bin/env python3
"""Post-routing hand fixes for the FD board, in group "hand".

Usage: fd_post.py <board.kicad_pcb>      (in place; run `pcbkit viaout` after)

TP4 (+3V3 test pad), R30 and the J7 1-wire pull-up reached the +3V3 plane
through rev D's R16 -> TP4 strap, which fd_board.py cut where C57 / R75 now
sit. TP4 gets its own plane via (dropped in the pad; viaout moves it out
with a stub).

J10 pin 5 (GND, the dry-contact header's end pin) keeps one F.Cu thermal spoke
between the rerouted DRY_IN tracks (starved_thermal): solid connection.
"""
import sys
import pcbnew

MM = pcbnew.FromMM


def main(argv):
    b = pcbnew.LoadBoard(argv[0])
    grp = [g for g in b.Groups() if g.GetName() == "hand"][0]
    pad = b.FindFootprintByReference("TP4").Pads()[0]
    v = pcbnew.PCB_VIA(b)
    v.SetPosition(pad.GetPosition())
    v.SetWidth(MM(0.6))
    v.SetDrill(MM(0.3))
    v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
    v.SetNet(pad.GetNet())
    b.Add(v)
    grp.AddItem(v)
    for p in b.FindFootprintByReference("J10").Pads():
        if p.GetNumber() == "5":
            p.SetLocalZoneConnection(pcbnew.ZONE_CONNECTION_FULL)
    b.Save(argv[0])
    print("TP4 plane via added, J10 pin 5 solid")


if __name__ == "__main__":
    main(sys.argv[1:])
