"""KiCad 10 schematic emitters (file version 20260306), stdlib only.

Library side (moved from gensch.py): block_at, raw_symbol, prop_val,
set_prop, flatten, custom_symbol, pins_of, sym / SYMS ("lib:name" ->
(block, pins)); stock libs from SYMDIR, "pp:" from the project pp.kicad_sym.

Geometry: lib coordinates are y-up, sheet coordinates y-down. An instance
(at x y rot) (mirror m) maps a lib offset by: y flip, then rotation
counter-clockwise as seen on the sheet, then mirror ("y" negates sheet x,
"x" negates sheet y). xform() is that map; pin_abs(symbol, pin) the
absolute pin point, Symbol.pin_out(pin) the outward unit vector there.

Sheet(name, project, ...) collects items and writes a .kicad_sch; every
uuid is uuid5 of the sheet key plus a stable item key, so output is a pure
function of the input.
"""
import os, re, uuid

SYMDIR = "/usr/share/kicad/symbols"
PROJ = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
VERSION = "20260306"
GRID = 1.27

# ---------------------------------------------------------------- lib parsing
def block_at(text, start):
    """return text of the s-expr starting at index start (a '(')"""
    depth = 0; i = start; instr = False
    while i < len(text):
        c = text[i]
        if instr:
            if c == '\\': i += 1
            elif c == '"': instr = False
        else:
            if c == '"': instr = True
            elif c == '(': depth += 1
            elif c == ')':
                depth -= 1
                if depth == 0: return text[start:i+1]
        i += 1
    raise ValueError("unbalanced")

_libcache = {}
def libtext(lib):
    if lib not in _libcache:
        _libcache[lib] = open(os.path.join(SYMDIR, lib + ".kicad_sym")).read()
    return _libcache[lib]

def raw_symbol(text, name, indent="\t"):
    key = '\n%s(symbol "%s"' % (indent, name)
    i = text.find(key)
    if i < 0: raise KeyError(name)
    return block_at(text, i + 1 + len(indent))

def prop_val(block, pname):
    m = re.search(r'\(property "%s" "((?:[^"\\]|\\.)*)"' % re.escape(pname), block)
    return m.group(1) if m else None

def set_prop(block, pname, val):
    return re.sub(r'(\(property "%s" ")(?:[^"\\]|\\.)*(")' % re.escape(pname),
                  lambda m: m.group(1) + val + m.group(2), block, count=1)

def flatten(lib, name):
    """lib symbol block, extends resolved, renamed to LIB:NAME"""
    text = libtext(lib)
    child = raw_symbol(text, name)
    m = re.search(r'\(extends "([^"]+)"\)', child)
    if m:
        parent = m.group(1)
        base = raw_symbol(text, parent)
        base = base.replace('(symbol "%s"' % parent, '(symbol "%s:%s"' % (lib, name), 1)
        base = base.replace('(symbol "%s_' % parent, '(symbol "%s_' % name)
        for p in ("Reference", "Value", "Footprint", "Datasheet", "Description", "ki_keywords", "ki_fp_filters"):
            v = prop_val(child, p)
            if v is not None:
                if prop_val(base, p) is None:
                    base = base.rstrip(")") + '\t\t(property "%s" "%s" (at 0 0 0) (effects (font (size 1.27 1.27)) (hide yes)))\n\t)' % (p, v)
                else:
                    base = set_prop(base, p, v)
        return base
    return child.replace('(symbol "%s"' % name, '(symbol "%s:%s"' % (lib, name), 1)

def custom_symbol(name):
    """from the project lib pp.kicad_sym"""
    text = open(os.path.join(PROJ, "pp.kicad_sym")).read()
    b = raw_symbol(text, name)
    return b.replace('(symbol "%s"' % name, '(symbol "pp:%s"' % name, 1)

def pins_of(block):
    out = []
    for m in re.finditer(r'\(pin\s+(\w+)\s+\w+\s*\n?\s*\(at\s+([-\d.]+)\s+([-\d.]+)\s+([-\d.]+)\)', block):
        pb = block_at(block, m.start())
        nm = re.search(r'\(name "((?:[^"\\]|\\.)*)"', pb).group(1)
        num = re.search(r'\(number "((?:[^"\\]|\\.)*)"', pb).group(1)
        ln = re.search(r'\(length ([-\d.]+)\)', pb)
        out.append(dict(type=m.group(1), x=float(m.group(2)), y=float(m.group(3)),
                        rot=float(m.group(4)), name=nm, num=num, len=float(ln.group(1)) if ln else 0.0))
    return out

def body_pts(block):
    """lib-space points spanning the drawn body: graphics, pin ends and pin roots"""
    pts = [(float(a), float(b)) for a, b in re.findall(r'\((?:xy|start|end|mid) ([-\d.]+) ([-\d.]+)\)', block)]
    for cx, cy, r in re.findall(r'\(center ([-\d.]+) ([-\d.]+)\)\s*\(radius ([-\d.]+)\)', block):
        cx, cy, r = float(cx), float(cy), float(r)
        pts += [(cx - r, cy - r), (cx + r, cy + r)]
    for p in pins_of(block):
        dx, dy = DIRS[int(p["rot"]) % 360]
        pts += [(p["x"], p["y"]), (p["x"] + dx * p["len"], p["y"] + dy * p["len"])]
    return pts

SYMS = {}   # "lib:name" -> (block, pins)
def sym(libname):
    if libname not in SYMS:
        lib, name = libname.split(":")
        b = custom_symbol(name) if lib == "pp" else flatten(lib, name)
        SYMS[libname] = (b, pins_of(b))
    return SYMS[libname]

def by_name(libname, names):
    """nets keyed by pin NAME (all pins with that name get the net)"""
    out = {}
    for p in sym(libname)[1]:
        if p["name"] in names: out[p["num"]] = names[p["name"]]
    missing = set(names) - {p["name"] for p in sym(libname)[1]}
    if missing: raise ValueError("%s: no pins named %s" % (libname, missing))
    return out

# ---------------------------------------------------------------- geometry
DIRS = {0: (1, 0), 90: (0, 1), 180: (-1, 0), 270: (0, -1)}   # lib space, y up

def R(v): return round(v + 0.0, 4) + 0.0

def xform(dx, dy, rot=0, mirror=None):
    """lib offset (y up) -> sheet offset (y down) for an instance at rot / mirror"""
    x, y = dx, -dy
    for _ in range((int(rot) // 90) % 4):
        x, y = y, -x                  # 90 deg counter-clockwise on the sheet
    if mirror == "y": x = -x          # mirror AFTER rotation (netlist-verified, 12 orientations)
    elif mirror == "x": y = -y
    return x, y

def on_grid(v, g=GRID): return abs(round(v / g) * g - v) < 1e-6

class Symbol:
    """one placed symbol; fields = hidden extra properties (LCSC, MPN, ...);
    instances = [(sheet path, reference)], None = this sheet's own path"""
    def __init__(self, ref, lib, value, fp="", at=(0, 0), rot=0, mirror=None, fields=None,
                 desc="", dnp=False, power=False, instances=None):
        if rot not in (0, 90, 180, 270): raise ValueError("%s: rot %r" % (ref, rot))
        if mirror not in (None, "x", "y"): raise ValueError("%s: mirror %r" % (ref, mirror))
        self.ref, self.lib, self.value, self.fp = ref, lib, value, fp
        self.at, self.rot, self.mirror = (R(at[0]), R(at[1])), rot, mirror
        self.fields, self.desc, self.dnp, self.power = dict(fields or {}), desc, dnp, power
        self.instances = instances
        self.block, self.pins = sym(lib)
        self.pinmap = {}
        for p in self.pins: self.pinmap.setdefault(p["num"], p)
    def pin(self, num):
        if num not in self.pinmap: raise KeyError("%s (%s) has no pin %s" % (self.ref, self.lib, num))
        return self.pinmap[num]
    def pin_xy(self, num): return pin_abs(self, num)
    def pin_out(self, num):
        """unit vector on the sheet pointing away from the body at this pin"""
        dx, dy = DIRS[int(self.pin(num)["rot"]) % 360]
        x, y = xform(dx, dy, self.rot, self.mirror)
        return (-round(x), -round(y))
    def bbox(self):
        pts = [xform(a, b, self.rot, self.mirror) for a, b in body_pts(self.block)] or [(0, 0)]
        xs = [self.at[0] + p[0] for p in pts]; ys = [self.at[1] + p[1] for p in pts]
        return R(min(xs)), R(min(ys)), R(max(xs)), R(max(ys))

def pin_abs(s, num):
    p = s.pin(num) if not isinstance(num, dict) else num
    dx, dy = xform(p["x"], p["y"], s.rot, s.mirror)
    return R(s.at[0] + dx), R(s.at[1] + dy)

# ---------------------------------------------------------------- emit helpers
def esc(s): return s.replace('\\', '\\\\').replace('"', '\\"')

def n(v):
    s = ("%.4f" % R(v)).rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s

def effects(justify=None, size=1.27, bold=False, hide=False, ind="\t\t"):
    s = '%s(effects\n%s\t(font\n%s\t\t(size %s %s)\n' % (ind, ind, ind, n(size), n(size))
    if bold: s += '%s\t\t(bold yes)\n' % ind
    s += '%s\t)\n' % ind
    if justify: s += '%s\t(justify %s)\n' % (ind, justify)
    if hide: s += '%s\t(hide yes)\n' % ind
    return s + '%s)\n' % ind

def prop(name, val, x, y, hide=False, justify="left", ang=0):
    return ('\t\t(property "%s" "%s"\n\t\t\t(at %s %s %d)\n%s\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n'
            % (name, esc(val), n(x), n(y), ang, "\t\t\t(hide yes)\n" if hide else "")
            + effects(justify, ind="\t\t\t") + '\t\t)\n')

LABEL_JUST = {0: "left bottom", 180: "right bottom", 90: "left bottom", 270: "right bottom"}
HLABEL_JUST = {0: "left", 180: "right", 90: "left", 270: "right"}
SHAPES = ("input", "output", "bidirectional", "tri_state", "passive")
SIDE_PIN = {"left": (180, "left"), "right": (0, "right"), "top": (90, "right"), "bottom": (270, "left")}

class SheetSymbol:
    """a sheet symbol on a parent sheet; pins = [(name, shape, side, offset)],
    offset measured from the top / left corner along that edge (mm)"""
    def __init__(self, name, file, at, size, pins=(), page="2"):
        self.name, self.file, self.page = name, file, str(page)
        self.at, self.size = (R(at[0]), R(at[1])), (R(size[0]), R(size[1]))
        self.pins = list(pins)
        self.uuid = self.path = None    # set by Sheet.sheet()
    def pin_xy(self, name):
        for nm, shape, side, off in self.pins:
            if nm == name:
                x, y = self.at; w, h = self.size
                return {"left": (x, y + off), "right": (x + w, y + off),
                        "top": (x + off, y), "bottom": (x + off, y + h)}[side]
        raise KeyError("sheet %s has no pin %s" % (self.name, name))

class Sheet:
    """one .kicad_sch file. root=True adds sheet_instances and makes the
    default symbol instance path "/<uuid>"."""
    def __init__(self, name, project, key=None, paper="A4", root=False, title=None,
                 date="2026-10-24", rev="", company="", comments=()):
        self.name, self.project, self.key, self.paper, self.root = name, project, key or name, paper, root
        self.uuid = str(uuid.uuid5(uuid.NAMESPACE_URL, "sch/%s" % self.key))
        self.title, self.date, self.rev, self.company, self.comments = title, date, rev, company, list(comments)
        self.body, self.libs, self._seen = [], set(), set()
    def U(self, key):
        if key in self._seen: raise ValueError("%s: duplicate item key %s" % (self.name, key))
        self._seen.add(key)
        return str(uuid.uuid5(uuid.NAMESPACE_URL, "%s/%s" % (self.key, key)))

    def symbol(self, s):
        self.libs.add(s.lib)
        x, y = s.at
        pw = s.power
        o = '\t(symbol\n\t\t(lib_id "%s")\n\t\t(at %s %s %d)\n' % (s.lib, n(x), n(y), s.rot)
        if s.mirror: o += '\t\t(mirror %s)\n' % s.mirror
        o += ('\t\t(unit 1)\n\t\t(body_style 1)\n\t\t(exclude_from_sim no)\n\t\t(in_bom %s)\n\t\t(on_board %s)\n'
              '\t\t(in_pos_files yes)\n\t\t(dnp %s)\n\t\t(uuid "%s")\n') % (
              "no" if pw else "yes", "no" if pw else "yes", "yes" if s.dnp else "no", self.U("sym/" + s.ref))
        inst = s.instances if s.instances is not None else [("/" + self.uuid, s.ref)]
        ref = inst[0][1]          # KiCad writes the first instance's reference here
        x0, y0, x1, y1 = s.bbox()
        cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
        # field angles compose with the symbol rotation: write 90 on a 90 / 270
        # symbol to read horizontally; centred text is immune to the mirror
        fa = 90 if s.rot in (90, 270) else 0
        tw = lambda t: len(t) * 1.0 / 2           # half text width, 1.27 font
        if pw:
            ox, oy = s.pin_out(s.pins[0]["num"]) if s.pins else (0, 1)
            ox, oy = -ox, -oy                     # value beyond the symbol, away from the host pin
            vx, vy = {(0, 1): (cx, y1 + 1.905), (0, -1): (cx, y0 - 1.27),
                      (1, 0): (x1 + 0.635 + tw(s.value), cy), (-1, 0): (x0 - 0.635 - tw(s.value), cy)}[(ox, oy)]
            o += prop("Reference", ref, x, y, hide=True, justify=None, ang=fa)
            o += prop("Value", s.value, vx, vy, justify=None, ang=fa)
        else:
            dirs = {s.pin_out(p["num"]) for p in s.pins}
            if len(s.pins) == 1:           # one-pin part (standoff, pogo pin): reference beside it, away from the pin
                side = "single"
            elif len(s.pins) <= 2:
                side = "right" if (y1 - y0) >= (x1 - x0) else "split"
            elif (0, -1) not in dirs: side = "above"
            elif (0, 1) not in dirs: side = "below"
            elif (1, 0) not in dirs: side = "right"
            elif (-1, 0) not in dirs: side = "left"
            else: side = "corner"          # pins on all four sides: top-right corner, outside
            if side == "single":
                px = s.pin_out(s.pins[0]["num"])[0]
                rp = (x0 - 1.27 - tw(ref), cy) if px > 0 else (x1 + 1.27 + tw(ref), cy)
                o += prop("Reference", ref, rp[0], rp[1], justify=None, ang=fa)
                o += prop("Value", s.value, x, y, hide=True)
            elif side == "split":          # two-pin part lying flat: reference above, value below
                rp, vp = (cx, y0 - 1.27), (cx, y1 + 1.905)
            elif side == "above":
                rp, vp = (cx, y0 - 2.794), (cx, y0 - 1.27)
            elif side == "below":
                rp, vp = (cx, y1 + 1.905), (cx, y1 + 3.429)
            elif side == "right":
                rp, vp = (x1 + 1.27 + tw(ref), cy - 1.27), (x1 + 1.27 + tw(s.value), cy + 1.27)
            elif side == "left":
                rp, vp = (x0 - 1.27 - tw(ref), cy - 1.27), (x0 - 1.27 - tw(s.value), cy + 1.27)
            else:
                rp, vp = (x1 + 1.27 + tw(ref), y0 - 2.794), (x1 + 1.27 + tw(s.value), y0 - 1.27)
            if side != "single":
                o += prop("Reference", ref, rp[0], rp[1], justify=None, ang=fa)
                o += prop("Value", s.value, vp[0], vp[1], justify=None, ang=fa)
        o += prop("Footprint", s.fp, x, y, hide=True)
        o += prop("Datasheet", "", x, y, hide=True)
        o += prop("Description", s.desc, x, y, hide=True)
        if not pw:
            o += prop("LCSC", s.fields.get("LCSC", ""), x, y, hide=True)
            for k, v in s.fields.items():
                if k != "LCSC": o += prop(k, v, x, y, hide=True)
        for p in s.pins:
            o += '\t\t(pin "%s"\n\t\t\t(uuid "%s")\n\t\t)\n' % (esc(p["num"]), self.U("pin/%s/%s" % (s.ref, p["num"])))
        o += '\t\t(instances\n\t\t\t(project "%s"\n' % esc(self.project)
        for path, ref in inst:
            o += '\t\t\t\t(path "%s"\n\t\t\t\t\t(reference "%s")\n\t\t\t\t\t(unit 1)\n\t\t\t\t)\n' % (path, esc(ref))
        o += '\t\t\t)\n\t\t)\n\t)\n'
        self.body.append(o)

    def wire(self, a, b, net=""):
        self.body.append('\t(wire\n\t\t(pts\n\t\t\t(xy %s %s) (xy %s %s)\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)\n\t\t\t(type default)\n\t\t)\n\t\t(uuid "%s")\n\t)\n'
                         % (n(a[0]), n(a[1]), n(b[0]), n(b[1]), self.U("wire/%s/%s/%s/%s/%s" % (net, n(a[0]), n(a[1]), n(b[0]), n(b[1])))))

    def junction(self, x, y):
        self.body.append('\t(junction\n\t\t(at %s %s)\n\t\t(diameter 0)\n\t\t(color 0 0 0 0)\n\t\t(uuid "%s")\n\t)\n'
                         % (n(x), n(y), self.U("junction/%s/%s" % (n(x), n(y)))))

    def nc(self, x, y):
        self.body.append('\t(no_connect\n\t\t(at %s %s)\n\t\t(uuid "%s")\n\t)\n' % (n(x), n(y), self.U("nc/%s/%s" % (n(x), n(y)))))

    def label(self, text, x, y, ang=0):
        self.body.append('\t(label "%s"\n\t\t(at %s %s %d)\n%s\t\t(uuid "%s")\n\t)\n'
                         % (esc(text), n(x), n(y), ang, effects(LABEL_JUST[ang]), self.U("label/%s/%s/%s" % (text, n(x), n(y)))))

    def glabel(self, text, x, y, ang=0, shape="passive"):
        self.body.append(('\t(global_label "%s"\n\t\t(shape %s)\n\t\t(at %s %s %d)\n\t\t(fields_autoplaced yes)\n%s\t\t(uuid "%s")\n'
                          '\t\t(property "Intersheetrefs" "${INTERSHEET_REFS}"\n\t\t\t(at %s %s 0)\n\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n\t\t\t(hide yes)\n%s\t\t)\n\t)\n')
                         % (esc(text), shape, n(x), n(y), ang, effects(HLABEL_JUST[ang]), self.U("glabel/%s/%s/%s" % (text, n(x), n(y))),
                            n(x), n(y), effects(ind="\t\t\t")))

    def hlabel(self, text, shape, x, y, ang=180):
        if shape not in SHAPES: raise ValueError("hierarchical label %s: shape %s" % (text, shape))
        self.body.append('\t(hierarchical_label "%s"\n\t\t(shape %s)\n\t\t(at %s %s %d)\n%s\t\t(uuid "%s")\n\t)\n'
                         % (esc(text), shape, n(x), n(y), ang, effects(HLABEL_JUST[ang]), self.U("hlabel/%s/%s/%s" % (text, n(x), n(y)))))

    def text(self, txt, x, y, size=2.0, bold=True):
        self.body.append('\t(text "%s"\n\t\t(exclude_from_sim no)\n\t\t(at %s %s 0)\n%s\t\t(uuid "%s")\n\t)\n'
                         % (esc(txt), n(x), n(y), effects("left bottom", size, bold), self.U("text/%s/%s/%s" % (txt, n(x), n(y)))))

    def sheet(self, ss):
        """place a SheetSymbol; sets ss.uuid and ss.path (the child's instance path)"""
        ss.uuid = self.U("sheet/" + ss.name)
        ss.path = "/%s/%s" % (self.uuid, ss.uuid)
        (x, y), (w, h) = ss.at, ss.size
        o = ('\t(sheet\n\t\t(at %s %s)\n\t\t(size %s %s)\n\t\t(exclude_from_sim no)\n\t\t(in_bom yes)\n\t\t(on_board yes)\n\t\t(dnp no)\n'
             '\t\t(fields_autoplaced yes)\n\t\t(stroke\n\t\t\t(width 0.1524)\n\t\t\t(type solid)\n\t\t)\n\t\t(fill\n\t\t\t(color 0 0 0 0)\n\t\t)\n'
             '\t\t(uuid "%s")\n') % (n(x), n(y), n(w), n(h), ss.uuid)
        o += prop("Sheetname", ss.name, x, y - 0.7116, justify="left bottom")
        o += prop("Sheetfile", ss.file, x, y + h + 0.5884, justify="left top")
        for nm, shape, side, off in ss.pins:
            if shape not in SHAPES: raise ValueError("sheet pin %s: shape %s" % (nm, shape))
            px, py = ss.pin_xy(nm)
            ang, just = SIDE_PIN[side]
            o += ('\t\t(pin "%s" %s\n\t\t\t(at %s %s %d)\n\t\t\t(uuid "%s")\n' % (esc(nm), shape, n(px), n(py), ang, self.U("sheetpin/%s/%s" % (ss.name, nm)))
                  + effects(just, ind="\t\t\t") + '\t\t)\n')
        o += ('\t\t(instances\n\t\t\t(project "%s"\n\t\t\t\t(path "/%s"\n\t\t\t\t\t(page "%s")\n\t\t\t\t)\n\t\t\t)\n\t\t)\n\t)\n'
              % (esc(self.project), self.uuid, ss.page))
        self.body.append(o)

    def text_out(self):
        out = ['(kicad_sch', '\t(version %s)' % VERSION, '\t(generator "pp-gen")', '\t(generator_version "10.0")',
               '\t(uuid "%s")' % self.uuid, '\t(paper "%s")' % self.paper]
        if self.title is not None:
            out += ['\t(title_block', '\t\t(title "%s")' % esc(self.title), '\t\t(date "%s")' % self.date]
            if self.rev: out.append('\t\t(rev "%s")' % esc(self.rev))
            if self.company: out.append('\t\t(company "%s")' % esc(self.company))
            for i, c in enumerate(self.comments): out.append('\t\t(comment %d "%s")' % (i + 1, esc(c)))
            out.append('\t)')
        out.append('\t(lib_symbols')
        for k in sorted(self.libs):
            out.append("\t\t" + sym(k)[0].replace("\n", "\n\t"))
        out.append('\t)')
        out.append("".join(self.body).rstrip("\n"))
        if self.root:
            out.append('\t(sheet_instances\n\t\t(path "/"\n\t\t\t(page "1")\n\t\t)\n\t)')
        out.append('\t(embedded_fonts no)\n)')
        return "\n".join(x for x in out if x) + "\n"

    def write(self, path):
        d = os.path.dirname(os.path.abspath(path))
        os.makedirs(d, exist_ok=True)
        with open(path, "w") as f: f.write(self.text_out())
        return path
