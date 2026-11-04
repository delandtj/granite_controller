"""prune2.py <board> <drc.json>: delete track/via_dangling items except the SEG_SCL via at (139.7, 145.7) (In2 hop)"""
import json, sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1]); d = json.load(open(sys.argv[2])); M = pcbnew.ToMM
pos = []
for v in d['violations']:
    if v['type'] in ('track_dangling', 'via_dangling'):
        p = v['items'][0]['pos']
        if abs(p['x'] - 139.7) < 0.01 and abs(p['y'] - 145.7) < 0.01: continue
        pos.append((v['items'][0]['uuid'],))
ids = {p[0] for p in pos}
t = b.Tracks(); n = 0
for it in [t[i] for i in range(len(t))]:
    if it.m_Uuid.AsString() in ids: b.Delete(it); n += 1
b.Save(sys.argv[1]); print('pruned', n)
