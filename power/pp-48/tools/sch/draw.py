"""Drawing engine: a Block is one sheet of placed symbols; finish() draws it.

    from sch.draw import Block, R0402, C0402
    b = Block("Slot", pwr_base=0)
    b.sym("R1", "Device:R", "10k", R0402, {"1": "VIN", "2": "MID"}, at=(50.8, 50.8))
    b.sym("C1", "Device:C", "100n", C0402, {"1": "MID", "2": "GND"}, at=(60.96, 58.42))
    b.wire("MID", [("R1", "2"), ("C1", "1")], via=[(60.96, 54.61)])
    b.port("VIN", "input", side="left", pos=(25.4, 46.99))
    b.finish()                                   # draws + lints, raises LintError
    b.write("sheets/slot.kicad_sch", instances=[(ss1.path, 100), (ss2.path, 200)])

Coordinates are sheet mm, y down, symbol origins on the 1.27 grid.

b.sym(ref, lib, value, fp, nets, at, rot=0, mirror=None|"x"|"y", fields={}, desc="", dnp=False)
    nets: pin number -> net name, every pin of the symbol must be listed;
    "NC" draws a no-connect flag. fields: hidden properties (LCSC, MPN, ...);
    LCSC is filled from BASIC by (value, fp) when not given. ref is the
    sheet-local reference; write() renumbers it per instance.
b.wire(net, path, via=None)
    path: a list of (ref, pin) and (x, y) points, drawn in order, one leg
    per consecutive pair. via points are inserted after the first element.
    A leg is straight if aligned, else L (corner) or Z (two corners). A leg
    leaves / enters a pin along the pin's axis from outside the body. Nets
    without a wire() are chained pin to pin in declaration order; pins of a
    net not named in any of its wire() calls are joined to the nearest
    named pin. Several wire() calls on one net are allowed (branches);
    junction dots are added where three or more ends meet.
b.port(net, shape="passive", side="left", pos=None)
    hierarchical label for a net that leaves the sheet; shape input /
    output / bidirectional / tri_state / passive; side = the sheet edge the
    label faces (and the root's sheet pin side). pos None = at the net's
    first pin, (ref, pin) = at that pin, (x, y) = at that point with a leg
    from the nearest pin of the net (unless the point is already wired).
b.power(net, symbol, value=None)
    net drawn with power symbols (GND, +3V3, +12V are by default). Each
    pin of the net gets its own symbol, rotated to point away from the
    body; for a wired power net one symbol sits at the first element of
    each wire() path instead (an (x, y) first element = an upright symbol
    at that point). value defaults to the net name (the net is the value).
b.flag(net, at)   PWR_FLAG at (ref, pin) or (x, y). On a pin that already has
    a power symbol it collides (lint); for a power net put it on a wire
    point: b.wire("GND", [("C2", "2"), (x, y)]); b.flag("GND", (x, y)).
b.name(net, at=None, ang=0)
    KiCad names an unlabelled net "Net-(C102-Pad1)", not "/Slot 1/NODE", so
    finish() puts a local label on every local net (one that is not power,
    NC or a port): on its longest horizontal wire, else vertical, else at a
    pin. b.name(net, at=(x, y), ang) places it (the point must be on the
    net's wire or a pin of it); b.name(net, False) leaves the net unnamed.
b.tie(net, at, ang=0)
    local label that joins this part of a net to the rest of it by name
    (no wire across the sheet). at = (ref, pin): the label sits at the pin
    end, facing away from the body, and the pin is left out of the
    automatic pin-to-pin wiring. at = (x, y): the label sits at the point,
    with a leg from the nearest pin of the net unless the point is already
    on one of its wires. A net with ties gets no automatic name label;
    every wired group of it needs a tie of its own.
b.text(txt, at, size=2.0)
b.finish()  derive everything, then lint: a wire leg may not cross a body
    box (box spanned by the pins, pin roots and graphics), pass over a pin
    of another net, or touch a wire end of another net; labels, power
    symbols and bodies may not overlap. Errors raise LintError naming the
    net and the symbols. Fix by moving parts or adding via points.
b.ports  [dict(name, shape, side, pos)]: what the root needs for sheet pins
b.paper  smallest of A4..A0 holding the drawing (or Block(paper=...))
b.write(path, instances=None, project=...)
    instances: [(sheet path, renum)], renum an int added to the reference
    number (R1 + 100 -> R101) or a callable ref -> ref. None = a root sheet.
"""
import re
from . import kicad as K
from .kicad import R

# ---------------------------------------------------------------- footprints
R0402 = "Resistor_SMD:R_0402_1005Metric"; R0603 = "Resistor_SMD:R_0603_1608Metric"
R0805 = "Resistor_SMD:R_0805_2012Metric"; R1206 = "Resistor_SMD:R_1206_3216Metric"
R2512 = "Resistor_SMD:R_2512_6332Metric"
C0402 = "Capacitor_SMD:C_0402_1005Metric"; C0603 = "Capacitor_SMD:C_0603_1608Metric"
C0805 = "Capacitor_SMD:C_0805_2012Metric"; C1210 = "Capacitor_SMD:C_1210_3225Metric"
LED0603 = "LED_SMD:LED_0603_1608Metric"
SJ = "Jumper:SolderJumper-2_P1.3mm_Open_RoundedPad1.0x1.5mm"

BASIC = {   # JLC basic parts, by (value, footprint); verify in the JLC preview before ordering
    ("10k", R0402): "C25744", ("100k", R0402): "C25741", ("47k", R0402): "C25792", ("220", R0402): "C25091",
    ("270", R0402): "C22775", ("2.2k", R0402): "C25879", ("4.7k", R0402): "C25900", ("1k", R0402): "C11702",
    ("10k", R0603): "C25804", ("100k", R0603): "C25803", ("100n", C0402): "C1525",
    # the rest of the core's passives (JLC parts API 2026-10-30; basic where JLC has one, else the
    # best-stocked extended part of the right rating)
    ("1n", C0402): "C1523", ("10n 50V", C0402): "C15195", ("33p", C0402): "C1562",
    ("1.5n", C0402): "C107022", ("2.2n 50V", C0402): "C1531",
    ("1u 25V", C0603): "C15849", ("4.7u", C0603): "C19666", ("100n 100V", C0603): "C696916",
    ("4.7u 25V", C0805): "C1779", ("10u 25V", C0805): "C15850", ("22u 10V", C1210): "C318780",
    ("100k", R0805): "C149504", ("1M", R0805): "C17514", ("200k", R0805): "C17539", ("220k", R0805): "C17556",
    ("280k", R0805): "C5121536", ("4.7k", R0603): "C23162",
    ("2k", R0402): "C4109", ("4.22k", R0402): "C25898", ("18.7k", R0402): "C5153953", ("41.2k", R0402): "C2998053",
    ("49.9k", R0402): "C137969", ("52.3k", R0402): "C2998086", ("86.6k", R0402): "C48533697", ("453k", R0402): "C27009",
}

DEFAULT_POWER = {"GND": "power:GND", "+3V3": "power:+3V3", "+12V": "power:+12V"}
SIDES = {"left": (180, (1, 0)), "right": (0, (-1, 0)), "top": (90, (0, 1)), "bottom": (270, (0, -1))}
PAPERS = [("A4", 297, 210), ("A3", 420, 297), ("A2", 594, 420), ("A1", 841, 594), ("A0", 1189, 841)]
EPS = 1e-6
STEP = 2.54

class LintError(Exception):
    pass

def snap(v, g=STEP): return R(round(v / g) * g)
def is_pin(e): return isinstance(e[0], str)
def fmt(p): return "(%s, %s)" % (K.n(p[0]), K.n(p[1]))

def renumber(ref, off):
    if callable(off): return off(ref)
    m = re.match(r"^(#?[A-Za-z_]+)(\d+)$", ref)
    if not m: raise ValueError("cannot renumber %s" % ref)
    return m.group(1) + str(int(m.group(2)) + off)

def leg(a, da, b, db):
    """points from a to b; da / db = outward unit vector at a pin or None"""
    (ax, ay), (bx, by) = a, b
    if abs(ax - bx) < EPS or abs(ay - by) < EPS: return [a, b]
    ha, va = da is not None and da[1] == 0, da is not None and da[0] == 0
    hb, vb = db is not None and db[1] == 0, db is not None and db[0] == 0
    if ha and hb:
        if da[0] == db[0]: xo = R((max if da[0] > 0 else min)(ax, bx) + da[0] * STEP)
        else: xo = snap((ax + bx) / 2)
        return [a, (xo, ay), (xo, by), b]
    if va and vb:
        if da[1] == db[1]: yo = R((max if da[1] > 0 else min)(ay, by) + da[1] * STEP)
        else: yo = snap((ay + by) / 2)
        return [a, (ax, yo), (bx, yo), b]
    if ha or vb or (da is None and db is None): return [a, (bx, ay), b]
    return [a, (ax, by), b]          # va, or hb with da None

def on_seg(p, a, b, strict=False):
    (px, py), (ax, ay), (bx, by) = p, a, b
    if abs(ax - bx) < EPS:
        if abs(px - ax) > EPS: return False
        lo, hi = min(ay, by), max(ay, by); v = py
    else:
        if abs(py - ay) > EPS: return False
        lo, hi = min(ax, bx), max(ax, bx); v = px
    return lo + EPS < v < hi - EPS if strict else lo - EPS <= v <= hi + EPS

def seg_in_box(a, b, box):
    x0, y0, x1, y1 = box
    if abs(a[1] - b[1]) < EPS:
        lo, hi = min(a[0], b[0]), max(a[0], b[0])
        return y0 + EPS < a[1] < y1 - EPS and min(hi, x1) - max(lo, x0) > EPS
    lo, hi = min(a[1], b[1]), max(a[1], b[1])
    return x0 + EPS < a[0] < x1 - EPS and min(hi, y1) - max(lo, y0) > EPS

def b_len(s): return abs(s[0][0] - s[1][0]) + abs(s[0][1] - s[1][1])

def overlap(p, q):
    return min(p[2], q[2]) - max(p[0], q[0]) > EPS and min(p[3], q[3]) - max(p[1], q[1]) > EPS

def label_box(text, x, y, ang):
    w, h = len(text) * 1.0 + 2.0, 0.9
    return {0: (x, y - h, x + w, y + h), 180: (x - w, y - h, x, y + h),
            90: (x - h, y - w, x + h, y), 270: (x - h, y, x + h, y + w)}[ang]

def llabel_box(text, x, y, ang):
    """local label: text beside the anchor, above (or left of) the wire"""
    w, a, b = len(text) * 1.0 + 0.5, 0.2, 1.6
    return {0: (x, y - b, x + w, y - a), 180: (x - w, y - b, x, y - a),
            90: (x - b, y - w, x - a, y), 270: (x - b, y, x - a, y + w)}[ang]

class Block:
    def __init__(self, name, key=None, project="pp-48", paper=None, title=None, date="2026-10-24", pwr_base=0):
        self.sheet_name, self.key, self.project = name, key or name, project
        self._paper, self.title, self.date, self.pwr_base = paper, title, date, pwr_base
        self.syms, self.order = {}, []
        self.wires, self.ports, self.flags, self.texts = [], [], [], []
        self.powers = dict(DEFAULT_POWER); self.pvalues = {}
        self.names = {}
        self.ties = []
        self.done = False

    # ------------------------------------------------------------ authoring
    def sym(self, ref, lib, value, fp, nets, at=(0, 0), rot=0, mirror=None, fields=None, desc="", dnp=False):
        if ref in self.syms: raise ValueError("dup ref " + ref)
        if not (K.on_grid(at[0]) and K.on_grid(at[1])): raise ValueError("%s: at %s off the 1.27 grid" % (ref, at))
        fields = dict(fields or {})
        if not fields.get("LCSC") and (value, fp) in BASIC: fields["LCSC"] = BASIC[(value, fp)]
        s = K.Symbol(ref, lib, value, fp, at, rot, mirror, fields, desc, dnp)
        for num in nets:
            s.pin(num)
        for p in s.pins:
            if nets.get(p["num"]) is None:
                raise ValueError("%s pin %s (%s) has no net" % (ref, p["num"], p["name"]))
        s.nets = dict(nets)
        self.syms[ref] = s; self.order.append(ref)
        return s

    def wire(self, net, path, via=None):
        path = [tuple(e) for e in path]
        if via: path = path[:1] + [tuple(v) for v in via] + path[1:]
        self.wires.append((net, path))

    def port(self, net, shape="passive", side="left", pos=None):
        if side not in SIDES: raise ValueError("port %s: side %s" % (net, side))
        if shape not in K.SHAPES: raise ValueError("port %s: shape %s" % (net, shape))
        self.ports.append(dict(name=net, shape=shape, side=side, pos=pos))

    def power(self, net, symbol, value=None):
        self.powers[net] = symbol
        if value: self.pvalues[net] = value

    def name(self, net, at=None, ang=0):
        self.names[net] = at if at is False or at is None else (R(at[0]), R(at[1]), ang)

    def tie(self, net, at, ang=0):
        self.ties.append((net, tuple(at), ang))

    def flag(self, net, at):
        self.flags.append((net, tuple(at)))

    def text(self, txt, at, size=2.0):
        self.texts.append((txt, R(at[0]), R(at[1]), size))

    # ------------------------------------------------------------ finish
    def _ep(self, net, e):
        if is_pin(e):
            ref, num = e
            if ref not in self.syms: raise LintError("net %s: unknown symbol %s" % (net, ref))
            s = self.syms[ref]
            if s.nets.get(num) != net:
                raise LintError("net %s: %s pin %s is on net %s" % (net, ref, num, s.nets.get(num)))
            return s.pin_xy(num), s.pin_out(num)
        return (R(e[0]), R(e[1])), None

    def _path(self, net, elems, ends=None):
        pts = [self._ep(net, e) if not (isinstance(e, tuple) and len(e) == 3) else (e[:2], e[2]) for e in elems]
        for (a, da), (b, db) in zip(pts, pts[1:]):
            ps = leg(a, da, b, db)
            for p, q in zip(ps, ps[1:]):
                if abs(p[0] - q[0]) > EPS or abs(p[1] - q[1]) > EPS:
                    p, q = sorted([(R(p[0]), R(p[1])), (R(q[0]), R(q[1]))])
                    self.segs.append((net, p, q))

    def _psym(self, net, xy, out, tag):
        lib = self.powers[net]
        self.npwr += 1
        ref = ("#FLG%d" if lib == "power:PWR_FLAG" else "#PWR%d") % (self.pwr_base + self.npwr)
        val = "PWR_FLAG" if lib == "power:PWR_FLAG" else self.pvalues.get(net, net)
        s = K.Symbol(ref, lib, val, "", xy, 0, None, power=True)
        if out is not None:
            for rot in (0, 90, 180, 270):
                s.rot = rot
                o = s.pin_out(s.pins[0]["num"])
                if (-o[0], -o[1]) == tuple(out): break     # body points away from the host
            else: raise LintError("net %s: cannot orient %s" % (net, lib))
        s.tag, s.net = tag, net
        self.psyms.append(s)

    def finish(self):
        self.segs, self.psyms, self.ncs, self.hlabels, self.npwr = [], [], [], [], 0
        netpins = {}
        for ref in self.order:
            s = self.syms[ref]
            for num, net in s.nets.items():
                netpins.setdefault(net, []).append((ref, num))
        portnets = {p["name"] for p in self.ports}
        tiepins = {(net, at) for net, at, ang in self.ties if is_pin(at)}
        tienets = {net for net, at, ang in self.ties}
        for p in self.ports:
            if p["name"] not in netpins: raise LintError("port %s: no pin on this net" % p["name"])
        wired = {}
        for net, path in self.wires:
            if net == "NC": raise LintError("wire on NC")
            wired.setdefault(net, []).append(path)
            self._path(net, path)
        for net, pins in netpins.items():
            if net == "NC":
                for ref, num in pins: self.ncs.append(self.syms[ref].pin_xy(num))
                continue
            if net in self.powers:
                named = set()
                for path in wired.get(net, []):
                    e = path[0]; xy, out = self._ep(net, e)
                    self._psym(net, xy, out, "%s %s" % (net, "%s.%s" % e if is_pin(e) else fmt(xy)))
                    named |= {e for e in path if is_pin(e)}
                for ref, num in pins:
                    if (ref, num) not in named:
                        s = self.syms[ref]
                        self._psym(net, s.pin_xy(num), s.pin_out(num), "%s at %s.%s" % (net, ref, num))
                continue
            if len(pins) < 2 and net not in portnets and net not in wired and net not in tienets:
                raise LintError("net %s has one pin (%s.%s) and is not a port" % (net, pins[0][0], pins[0][1]))
            named = [e for path in wired.get(net, []) for e in path if is_pin(e)]
            pins = [e for e in pins if (net, e) not in tiepins or e in named]
            if not pins:
                continue
            if not named:
                self._path(net, pins)
                continue
            for e in pins:
                if e not in named:
                    xy, out = self._ep(net, e)
                    best = min(named, key=lambda f: (abs(self._ep(net, f)[0][0] - xy[0]) + abs(self._ep(net, f)[0][1] - xy[1]), f))
                    self._path(net, [best, e])
        for p in self.ports:
            net, pos = p["name"], p["pos"]
            ang, arrive = SIDES[p["side"]]
            if pos is None: pos = netpins[net][0]
            if is_pin(pos): xy = self._ep(net, pos)[0]
            else:
                xy = (R(pos[0]), R(pos[1]))
                touched = any(on_seg(xy, a, b) for n_, a, b in self.segs if n_ == net) or \
                          any(self._ep(net, e)[0] == xy for e in netpins[net])
                if not touched:
                    best = min(netpins[net], key=lambda e: (abs(self._ep(net, e)[0][0] - xy[0]) + abs(self._ep(net, e)[0][1] - xy[1]), e))
                    self._path(net, [best, (xy[0], xy[1], arrive)])
            self.hlabels.append((net, p["shape"], xy, ang))
        self.tlabels = []
        for net, at, ang in self.ties:
            if is_pin(at):
                xy, out = self._ep(net, at)
                self.tlabels.append((net, xy, {(1, 0): 0, (-1, 0): 180, (0, -1): 90, (0, 1): 270}[tuple(out)]))
                continue
            xy = (R(at[0]), R(at[1]))
            cands = [e for e in netpins[net] if (net, e) not in tiepins]
            touched = any(on_seg(xy, a, b) for n_, a, b in self.segs if n_ == net) or \
                      any(self._ep(net, e)[0] == xy for e in netpins[net])
            if not touched:
                if not cands: raise LintError("tie %s at %s: no pin to leg from" % (net, fmt(xy)))
                best = min(cands, key=lambda e: (abs(self._ep(net, e)[0][0] - xy[0]) + abs(self._ep(net, e)[0][1] - xy[1]), e))
                self._path(net, [best, xy])
            self.tlabels.append((net, xy, ang))
        for net, at in self.flags:
            xy, out = self._ep(net, at)
            saved = self.powers.get(net)
            self.powers[net] = "power:PWR_FLAG"
            self._psym(net, xy, out, "PWR_FLAG %s" % net)
            if saved is None: del self.powers[net]
            else: self.powers[net] = saved
        self._junctions(netpins)
        self.tienets = tienets
        self._names(netpins, portnets)
        self.labels += self.tlabels
        self._lint(netpins)
        self.done = True
        return self

    def _junctions(self, netpins):
        pinpts = {}
        for net, pins in netpins.items():
            for ref, num in pins: pinpts.setdefault(net, set()).add(self.syms[ref].pin_xy(num))
        for s in self.psyms: pinpts.setdefault(s.net, set()).add(s.at)
        segs = list(dict.fromkeys(self.segs))       # drop exact duplicates, keep order
        changed = True
        while changed:
            changed = False
            for i, (net, a, b) in enumerate(segs):
                cuts = {p for m, c, d in segs if m == net for p in (c, d)} | pinpts.get(net, set())
                for p in sorted(cuts):
                    if on_seg(p, a, b, strict=True):
                        segs[i:i + 1] = [(net, a, p), (net, p, b)]
                        changed = True; break
                if changed: break
        self.segs = segs = list(dict.fromkeys(segs))
        ends = {}
        for net, a, b in segs:
            for p in (a, b): ends[(net, p)] = ends.get((net, p), 0) + 1
        self.junctions = []
        for (net, p), k in ends.items():
            if k + (p in pinpts.get(net, ())) >= 3: self.junctions.append(p)

    def _names(self, netpins, portnets):
        self.labels = []
        for net, pins in netpins.items():
            if net == "NC" or net in self.powers or net in portnets or net in self.tienets: continue
            want = self.names.get(net)
            if want is False: continue
            segs = [(a, b) for m, a, b in self.segs if m == net]
            if want is not None:
                xy = want[:2]
                if not (any(on_seg(xy, a, b) for a, b in segs) or any(self.syms[r].pin_xy(k) == xy for r, k in pins)):
                    raise LintError("net %s: name label at %s is not on the net" % (net, fmt(xy)))
                self.labels.append((net, xy, want[2])); continue
            hs = [(a, b) for a, b in segs if abs(a[1] - b[1]) < EPS]
            vs = [(a, b) for a, b in segs if abs(a[0] - b[0]) < EPS]
            if hs:
                a, b = max(hs, key=lambda s: b_len(s))
                self.labels.append((net, (R(a[0] + (1.27 if b_len((a, b)) >= 2.54 else 0)), a[1]), 0))
            elif vs:
                a, b = max(vs, key=lambda s: b_len(s))
                self.labels.append((net, (b[0], R(b[1] - (1.27 if b_len((a, b)) >= 2.54 else 0))), 90))
            else:
                r, k = pins[0]; o = self.syms[r].pin_out(k)
                self.labels.append((net, self.syms[r].pin_xy(k), {(1, 0): 0, (-1, 0): 180, (0, -1): 90, (0, 1): 270}[o]))

    def _lint(self, netpins):
        pins = [(self.syms[r].pin_xy(num), net, r, num) for net, ps in netpins.items() for r, num in ps]
        bodies = [(r, self.syms[r].bbox()) for r in self.order]
        for net, a, b in self.segs:
            where = "net %s: wire %s-%s" % (net, fmt(a), fmt(b))
            for r, box in bodies:
                if seg_in_box(a, b, box) and not any(nn == net and s_.pin_xy(k) in (a, b) and on_seg((s_.pin_xy(k)[0] + 0.01 * s_.pin_out(k)[0], s_.pin_xy(k)[1] + 0.01 * s_.pin_out(k)[1]), a, b) for s_ in [self.syms[r]] for k, nn in s_.nets.items()): raise LintError("%s crosses the body of %s" % (where, r))   # a stub leaving its own pin outward is fine (graphics beyond the pin end, e.g. LED arrows)
            for xy, pnet, r, num in pins:
                if pnet != net and on_seg(xy, a, b):
                    raise LintError("%s passes over %s pin %s (net %s)" % (where, r, num, pnet))
            for m, c, d in self.segs:
                if m != net and (on_seg(c, a, b) or on_seg(d, a, b)):
                    raise LintError("%s touches a wire of net %s at %s-%s" % (where, m, fmt(c), fmt(d)))
        for net, shape, xy, ang in self.hlabels:
            for m, c, d in self.segs:
                if m != net and on_seg(xy, c, d): raise LintError("port %s sits on a wire of net %s" % (net, m))
            for pxy, pnet, r, num in pins:
                if pnet != net and pxy == xy: raise LintError("port %s sits on %s pin %s (net %s)" % (net, r, num, pnet))
        boxes = [("body of %s" % r, box) for r, box in bodies]
        boxes += [("power symbol %s" % s.tag, s.bbox()) for s in self.psyms]
        boxes += [("port %s" % net, label_box(net, xy[0], xy[1], ang)) for net, shape, xy, ang in self.hlabels]
        boxes += [("label %s" % net, llabel_box(net, xy[0], xy[1], ang)) for net, xy, ang in self.labels]
        for i, (na, ba) in enumerate(boxes):
            for nb, bb in boxes[i + 1:]:
                if overlap(ba, bb): raise LintError("%s overlaps %s" % (na, nb))

    # ------------------------------------------------------------ output
    def extent(self):
        xs, ys = [], []
        for box in [self.syms[r].bbox() for r in self.order] + [s.bbox() for s in self.psyms]:
            xs += [box[0], box[2]]; ys += [box[1], box[3]]
        for net, a, b in self.segs: xs += [a[0], b[0]]; ys += [a[1], b[1]]
        for net, shape, xy, ang in self.hlabels:
            box = label_box(net, xy[0], xy[1], ang); xs += [box[0], box[2]]; ys += [box[1], box[3]]
        return (min(xs), min(ys), max(xs), max(ys)) if xs else (0, 0, 0, 0)

    @property
    def paper(self):
        if self._paper: return self._paper
        x0, y0, x1, y1 = self.extent()
        for name, w, h in PAPERS:
            if x1 <= w - 15 and y1 <= h - 45: return name
        return "A0"

    def write(self, path, instances=None, project=None):
        if not self.done: self.finish()
        sh = K.Sheet(self.sheet_name, project or self.project, key=self.key, paper=self.paper, root=instances is None,
                     title=self.title or self.sheet_name, date=self.date)
        def inst(ref):
            return None if instances is None else [(p, renumber(ref, off)) for p, off in instances]
        for net, a, b in self.segs: sh.wire(a, b, net)
        for p in self.junctions: sh.junction(*p)
        for xy in self.ncs: sh.nc(*xy)
        for net, shape, xy, ang in self.hlabels: sh.hlabel(net, shape, xy[0], xy[1], ang)
        for net, xy, ang in self.labels: sh.label(net, xy[0], xy[1], ang)
        for txt, x, y, size in self.texts: sh.text(txt, x, y, size)
        for ref in self.order:
            s = self.syms[ref]; s.instances = inst(ref); sh.symbol(s)
        for s in self.psyms:
            s.instances = inst(s.ref); sh.symbol(s)
        return sh.write(path)
