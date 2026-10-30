#!/usr/bin/env python3
"""Put back hand routes that krt-route ripped.

Usage: restore_hand.py <before.kicad_pcb> <routed.kicad_pcb>   (routed in place)

krt-route keeps group "hand" in principle but rips and re-routes some of
its nets anyway (memory: krt-keep-groups-rip; seen here on ETH_RST,
ETH_XI). For every net whose "hand" item count dropped, all of the net's
copper in <routed> is deleted and the net's hand items from <before> are
copied back into the group; for GND / +3V3 KRT's copper stays and only
the missing hand items are added. Prints the restored nets; run kicad-cli DRC
afterwards and krt-reroute whatever now crosses them.
"""
import collections
import sys
import pcbnew


PLANE = {"GND", "+3V3"}


def key(it):
    if it.Type() == pcbnew.PCB_VIA_T:
        return ("v", str(it.GetNetname()), it.GetPosition().x, it.GetPosition().y)
    s, e = (it.GetStart().x, it.GetStart().y), (it.GetEnd().x, it.GetEnd().y)
    return ("t", str(it.GetNetname()), it.GetLayer()) + tuple(sorted([s, e]))


def hand_items(b):
    for g in b.Groups():
        if g.GetName() == "hand":
            return [it for it in g.GetItems() if it.Type() in (pcbnew.PCB_TRACE_T, pcbnew.PCB_VIA_T, pcbnew.PCB_ARC_T)]
    return []


def main(argv):
    pre = pcbnew.LoadBoard(argv[0])
    b = pcbnew.LoadBoard(argv[1])
    before = collections.defaultdict(list)
    for it in hand_items(pre):
        before[str(it.GetNetname())].append(it)
    after = collections.Counter(str(it.GetNetname()) for it in hand_items(b))
    lost = sorted(n for n in before if after[n] < len(before[n]))
    if not lost:
        print("hand routes intact")
        return
    grp = [g for g in b.Groups() if g.GetName() == "hand"][0]
    nets = {n: b.FindNet(n) for n in lost}      # before any Remove (SWIG wrappers break after)
    tracks = b.Tracks()
    have = set()
    vias = []
    for t in [tracks[i] for i in range(len(tracks))]:
        n = str(t.GetNetname())
        if n in lost:
            if n in PLANE:          # GND / +3V3: keep KRT's plane taps, only add what is missing
                have.add(key(t))
                if t.Type() == pcbnew.PCB_VIA_T:
                    vias.append((n, t.GetPosition()))
            else:
                b.Remove(t)
    for n in lost:
        for it in before[n]:
            if key(it) in have:
                continue
            if it.Type() == pcbnew.PCB_VIA_T and any(
                    vn == n and (vp - it.GetPosition()).EuclideanNorm() < pcbnew.FromMM(1.0) for vn, vp in vias):
                continue            # KRT moved this via a little: keep KRT's
            if it.Type() == pcbnew.PCB_VIA_T:
                c = pcbnew.PCB_VIA(b)
                c.SetPosition(it.GetPosition())
                c.SetWidth(it.GetWidth(pcbnew.F_Cu))
                c.SetDrill(it.GetDrill())
                c.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
            else:
                c = pcbnew.PCB_ARC(b) if it.Type() == pcbnew.PCB_ARC_T else pcbnew.PCB_TRACK(b)
                if it.Type() == pcbnew.PCB_ARC_T:
                    c.SetMid(it.GetMid())
                c.SetStart(it.GetStart())
                c.SetEnd(it.GetEnd())
                c.SetWidth(it.GetWidth())
                c.SetLayer(it.GetLayer())
            c.SetNet(nets[n])
            b.Add(c)
            grp.AddItem(c)
    b.Save(argv[1])
    print("restored:", " ".join(lost))


if __name__ == "__main__":
    main(sys.argv[1:])
