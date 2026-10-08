"""Board helpers for scripted hand routing (run with the KiCad python):
load/add_track/add_via/delete_net_copper, DRC report parsing, and
render(): a region PNG with F.Cu red, B.Cu blue, highlighted nets, DRC
opens as dashed lines and violations as crosses.
"""
import math, re, sys
import pcbnew

MM = pcbnew.FromMM
def mm(v): return pcbnew.ToMM(v)
def P(x, y): return pcbnew.VECTOR2I(MM(x), MM(y))


def load(path): return pcbnew.LoadBoard(path)


def tracks(b):
    t = b.Tracks(); return [t[i] for i in range(len(t))]


def zones(b):
    z = b.Zones(); return [z[i] for i in range(len(z))]


def pads(b, net=None):
    out = []
    for f in b.GetFootprints():
        for p in f.Pads():
            if net is None or p.GetNetname() == net:
                out.append(p)
    return out


def layer_id(b, name): return b.GetLayerID(name)


def add_track(b, net, layer, pts, w=0.2):
    n = b.FindNet(net); assert n, net
    L = layer_id(b, layer) if isinstance(layer, str) else layer
    for (x0, y0), (x1, y1) in zip(pts, pts[1:]):
        t = pcbnew.PCB_TRACK(b); t.SetStart(P(x0, y0)); t.SetEnd(P(x1, y1))
        t.SetWidth(MM(w)); t.SetLayer(L); t.SetNet(n); b.Add(t)


def add_via(b, net, x, y, size=0.5, drill=0.2):
    n = b.FindNet(net); assert n, net
    v = pcbnew.PCB_VIA(b); v.SetPosition(P(x, y)); v.SetWidth(MM(size)); v.SetDrill(MM(drill))
    v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu); v.SetNet(n); b.Add(v)


def delete_net_copper(b, nets, region=None):
    """Delete tracks/vias of nets; region=(x0,y0,x1,y1) limits to items touching it."""
    nets = set(nets); k = 0
    for t in tracks(b):
        if t.GetNetname() not in nets: continue
        if region:
            x0, y0, x1, y1 = region
            pts = [t.GetStart(), t.GetEnd()]
            if not any(x0 <= mm(p.x) <= x1 and y0 <= mm(p.y) <= y1 for p in pts): continue
        b.Delete(t); k += 1
    return k


def pad_xy(b, ref, num):
    p = b.FindFootprintByReference(ref).FindPadByNumber(str(num)).GetPosition()
    return (round(mm(p.x), 4), round(mm(p.y), 4))


def drc_unconnected(rpt):
    """Parse a kicad-cli DRC report: list of (kind, [(x,y,desc)...])."""
    out = []; cur = None
    for line in open(rpt):
        if line.startswith('['):
            cur = (line.split(']')[0][1:], []); out.append(cur)
        elif cur and line.strip().startswith('@('):
            m = re.match(r'\s*@\(([-\d.]+) mm, ([-\d.]+) mm\): (.*)', line)
            if m: cur[1].append((float(m[1]), float(m[2]), m[3].strip()))
    return out


def render(b, out, x0, y0, x1, y1, hl=(), rpt=None, labels=True, scale=None):
    import matplotlib; matplotlib.use('Agg')
    import matplotlib.pyplot as plt
    from matplotlib.patches import Polygon, Circle
    W, H = x1 - x0, y1 - y0
    s = scale or min(0.6, 18.0 / W) * 1.0
    fig = plt.figure(figsize=(W * s, H * s), dpi=110)
    ax = fig.add_axes([0, 0, 1, 1]); ax.set_xlim(x0, x1); ax.set_ylim(y1, y0)
    ax.set_aspect('equal'); ax.set_facecolor('#111')
    def inside(x, y, m=3): return x0 - m <= x <= x1 + m and y0 - m <= y <= y1 + m
    def polys(shape, **kw):
        for i in range(shape.OutlineCount()):
            o = shape.Outline(i)
            pts = [(mm(o.CPoint(j).x), mm(o.CPoint(j).y)) for j in range(o.PointCount())]
            if pts and any(inside(*p, 6) for p in pts):
                ax.add_patch(Polygon(pts, closed=True, **kw))
    hl = set(hl)
    for z in zones(b):
        for L, c in ((pcbnew.B_Cu, '#1d3550'), (pcbnew.F_Cu, '#4a2020')):
            if z.IsOnLayer(L) and z.HasFilledPolysForLayer(L):
                polys(z.GetFilledPolysList(L), fc=c, ec='none', alpha=0.5, zorder=1)
    for f in b.GetFootprints():
        c = f.GetCourtyard(pcbnew.F_Cu)
        if c.OutlineCount():
            polys(c, fc='none', ec='#666', lw=0.4, zorder=2)
        for p in f.Pads():
            sh = p.GetEffectivePolygon(pcbnew.F_Cu if p.IsOnLayer(pcbnew.F_Cu) else pcbnew.B_Cu, pcbnew.ERROR_INSIDE)
            th = p.GetAttribute() == pcbnew.PAD_ATTRIB_PTH
            col = '#c9a227' if p.GetNetname() in hl else ('#888' if th else ('#c33' if p.IsOnLayer(pcbnew.F_Cu) else '#36c'))
            polys(sh, fc=col, ec='none', alpha=0.9, zorder=4)
            x, y = mm(p.GetPosition().x), mm(p.GetPosition().y)
            if labels and x0 <= x <= x1 and y0 <= y <= y1:
                n = p.GetNetname().split('/')[-1]
                if n.startswith('unconnected'): n = 'nc'
                ax.text(x, y, f"{p.GetNumber()}\n{n}", fontsize=3.2, color='w', ha='center', va='center', zorder=9)
        x, y = mm(f.GetPosition().x), mm(f.GetPosition().y)
        if x0 <= x <= x1 and y0 <= y <= y1:
            ax.text(x, y, f.GetReference(), fontsize=6, color='#0f0', ha='center', va='bottom', zorder=10, alpha=0.8)
    for t in tracks(b):
        n = t.GetNetname(); h = n in hl
        if t.GetClass() == 'PCB_VIA':
            x, y = mm(t.GetPosition().x), mm(t.GetPosition().y)
            if inside(x, y):
                ax.add_patch(Circle((x, y), mm(t.Cast().GetWidth(pcbnew.F_Cu)) / 2, fc='#ddd' if not h else '#ff0', ec='k', lw=0.2, zorder=7))
            continue
        xs = [mm(t.GetStart().x), mm(t.GetEnd().x)]; ys = [mm(t.GetStart().y), mm(t.GetEnd().y)]
        if not (inside(xs[0], ys[0], 30) or inside(xs[1], ys[1], 30)): continue
        f = t.GetLayer() == pcbnew.F_Cu
        col = ('#ff5' if f else '#5ff') if h else ('#e44' if f else '#48f')
        ax.plot(xs, ys, color=col, lw=max(0.3, mm(t.GetWidth()) * s * 72), solid_capstyle='round', alpha=0.75 if f else 0.6, zorder=6 if f else 5)
    if rpt:
        for kind, items in drc_unconnected(rpt):
            if kind == 'unconnected_items' and len(items) == 2:
                ax.plot([items[0][0], items[1][0]], [items[0][1], items[1][1]], color='#ff0', lw=0.8, ls='--', zorder=11)
            elif kind not in ('unconnected_items',):
                for x, y, _ in items[:1]:
                    if inside(x, y, 0): ax.plot(x, y, 'x', color='#f0f', ms=6, zorder=12)
    ax.set_xticks([x for x in range(int(x0), int(x1) + 1)]); ax.set_yticks([y for y in range(int(y0), int(y1) + 1)])
    ax.tick_params(labelsize=4, colors='#aaa', direction='in', pad=-8)
    ax.grid(True, color='#333', lw=0.3, zorder=0)
    fig.savefig(out); plt.close(fig)
