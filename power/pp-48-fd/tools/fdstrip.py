#!/usr/bin/env python3
"""Strip the router copper a re-placement invalidates (CAN FD layout, tools/fd_layout.sh).
Usage: fdstrip.py <old.kicad_pcb> <placed.kicad_pcb> <out.kicad_pcb> <regions.json> [--zone NET x0,y0,x1,y1]
regions.json: {"rects": [[x0,y0,x1,y1(,[layers])],...], "skip": [nets], "partial": [nets], "extra": [nets]}
Nets on moved / new footprints, nets with router copper in a rect and the extra nets lose all their
(ungrouped) copper; skip and partial nets lose only their copper inside the rects (partial ones are listed
for the reroute). --zone redraws an ungrouped zone of NET as that box. Prints the nets for krt-reroute."""
import json
import sys

import pcbnew

old = pcbnew.LoadBoard(sys.argv[1])
b = pcbnew.LoadBoard(sys.argv[2])
cfg = json.load(open(sys.argv[4]))
rects = cfg['rects']
skip = set(cfg['skip']) | set(cfg.get('partial', []))


def mm(v):
    return pcbnew.ToMM(v)


def inr(x, y, lay=None):
    return any(r[0] <= x <= r[2] and r[1] <= y <= r[3] and (len(r) < 5 or lay is None or lay in r[4]) for r in rects)


def key(fp):
    return (fp.GetPosition().x, fp.GetPosition().y, fp.GetOrientationDegrees(), fp.IsFlipped(), fp.GetFPID().GetUniStringLibId())


oldk = {fp.GetReference(): key(fp) for fp in old.GetFootprints()}
nets = set(cfg.get('extra', []))
moved = []
for fp in b.GetFootprints():
    if fp.GetAttributes() & pcbnew.FP_BOARD_ONLY:
        continue
    if oldk.get(fp.GetReference()) != key(fp):
        moved.append(fp.GetReference())
        nets |= {p.GetNetname() for p in fp.Pads() if p.GetNetname()}
t = b.Tracks()
items = [t[i] for i in range(len(t)) if t[i].GetParentGroup() is None]


def hit(it):
    if it.Type() == pcbnew.PCB_VIA_T:
        return inr(mm(it.GetPosition().x), mm(it.GetPosition().y))
    a, c = it.GetStart(), it.GetEnd()
    lay = b.GetLayerName(it.GetLayer())
    for k in range(21):
        if inr(mm(a.x + (c.x - a.x) * k / 20), mm(a.y + (c.y - a.y) * k / 20), lay):
            return True
    return False


for it in items:
    if it.GetNetname() not in skip and hit(it):
        nets.add(it.GetNetname())
nets -= skip
nets = {n for n in nets if n and not n.startswith('unconnected-')}
gone = [it for it in items if it.GetNetname() in nets or (it.GetNetname() in skip and hit(it))]
for it in gone:
    b.Delete(it)
args = sys.argv[5:]
while args:
    if args[0] == '--zone':
        name, box = args[1], [float(v) for v in args[2].split(',')]
        for z in b.Zones():
            if z.GetNetname() == name and not z.GetIsRuleArea() and z.GetParentGroup() is None:
                o = z.Outline()
                o.RemoveAllContours()
                o.NewOutline()
                for x, y in ((box[0], box[1]), (box[2], box[1]), (box[2], box[3]), (box[0], box[3])):
                    o.Append(pcbnew.FromMM(x), pcbnew.FromMM(y))
        args = args[3:]
b.Save(sys.argv[3])
print('moved:', ' '.join(sorted(moved)), file=sys.stderr)
print('deleted', len(gone), 'items', file=sys.stderr)
print(' '.join(sorted(nets | set(cfg.get('partial', [])))))
