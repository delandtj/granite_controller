#!/usr/bin/env python3
"""Generate the PP-48-xS power plane schematic (KiCad 10, flat sheet).

usage: gensch.py [N] [out.kicad_sch]   (N slots, default 2)

Every connection is a net label at the pin end; one power-symbol cluster
per power net anchors GND / +3V3 / +12V. Symbols come from the stock KiCad
libraries (flattened into lib_symbols, "extends" resolved) plus the project
library pp.kicad_sym (TLP176AM, PM_Brick_Socket). Edit this file and
regenerate; the .kicad_sch is output, not source. See docs/adr/0001 and
docs/power-board.md "Power plane".
"""
import sys, re, uuid, os

SYMDIR = "/usr/share/kicad/symbols"
HERE = os.path.dirname(os.path.abspath(__file__))
PROJ = os.path.dirname(HERE)
N = int(sys.argv[1]) if len(sys.argv) > 1 else 2
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(PROJ, "pp-48-%ds.kicad_sch" % N)
ROOT_UUID = "5a1b2c3d-0000-4000-8000-00000000pp%02d" % N
ROOT_UUID = str(uuid.uuid5(uuid.NAMESPACE_URL, "pp-48-%ds-root" % N))

def U(name):
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "pp-48-%ds/%s" % (N, name)))

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
        out.append(dict(type=m.group(1), x=float(m.group(2)), y=float(m.group(3)),
                        rot=float(m.group(4)), name=nm, num=num))
    return out

def by_name(libname, names):
    """nets keyed by pin NAME (all pins with that name get the net)"""
    out = {}
    for p in sym(libname)[1]:
        if p["name"] in names: out[p["num"]] = names[p["name"]]
    missing = set(names) - {p["name"] for p in sym(libname)[1]}
    if missing: raise ValueError("%s: no pins named %s" % (libname, missing))
    return out

SYMS = {}   # "lib:name" -> (block, pins)
def sym(libname):
    if libname not in SYMS:
        lib, name = libname.split(":")
        b = custom_symbol(name) if lib == "pp" else flatten(lib, name)
        SYMS[libname] = (b, pins_of(b))
    return SYMS[libname]

# ---------------------------------------------------------------- placement
parts = []      # dicts
labels = []     # (x, y, angle, text)
ncs = []        # (x, y)
wires = []
texts = []
_refs = set()

def extent(pins):
    xs = [p["x"] for p in pins] or [0]; ys = [p["y"] for p in pins] or [0]
    return min(xs) - 2.54, max(xs) + 2.54, min(ys) - 2.54, max(ys) + 2.54

class Block:
    """shelf-packs parts into a rectangle at (x0, y0), width w"""
    def __init__(self, title, x0, y0, w, pad=14.0):
        self.x0, self.y0, self.w, self.pad = x0, y0, w, pad
        self.cx, self.cy, self.rowh = 0.0, 0.0, 0.0
        self.title = title
        texts.append((x0, y0 - 4, title))
    def place(self, ref, libname, value, fp, nets, extra=None, dnp=False, desc="", wide=False):
        if ref in _refs: raise ValueError("dup ref " + ref)
        _refs.add(ref)
        block, pins = sym(libname)
        x0, x1, y0, y1 = extent(pins)
        pw = (x1 - x0) + 2 * self.pad + (10 if wide else 0)
        ph = (y1 - y0) + 2 * self.pad
        if self.cx + pw > self.w and self.cx > 0:
            self.cx = 0.0; self.cy += self.rowh; self.rowh = 0.0
        # symbol origin so that the part's box sits at (cx+pad, cy+pad)
        ox = self.x0 + self.cx + self.pad - x0
        oy = self.y0 + self.cy + self.pad + y1
        ox = round(ox / 1.27) * 1.27; oy = round(oy / 1.27) * 1.27
        self.cx += pw; self.rowh = max(self.rowh, ph)
        parts.append(dict(ref=ref, lib=libname, value=value, fp=fp, x=ox, y=oy,
                          nets=nets, extra=extra or {}, dnp=dnp, desc=desc, pins=pins))
        return ref
    def height(self):
        return self.cy + self.rowh

def pin_abs(part, p):
    return (round(part["x"] + p["x"], 4), round(part["y"] - p["y"], 4))

LABEL_ANGLE = {0: 180, 180: 0, 90: 270, 270: 90}   # pin rot -> label angle

def finish_part(part):
    byname = {p["num"]: p for p in part["pins"]}
    for num, p in byname.items():
        net = part["nets"].get(num)
        if net is None:
            raise ValueError("%s pin %s (%s) has no net" % (part["ref"], num, p["name"]))
        x, y = pin_abs(part, p)
        if net == "NC":
            ncs.append((x, y))
        else:
            labels.append((x, y, LABEL_ANGLE[int(p["rot"]) % 360], net))
    for num in part["nets"]:
        if num not in byname:
            raise ValueError("%s: no pin %s" % (part["ref"], num))

# ---------------------------------------------------------------- emit helpers
def esc(s): return s.replace('\\', '\\\\').replace('"', '\\"')

def prop(name, val, x, y, hide=False, justify="left"):
    return ('\t\t(property "%s" "%s"\n\t\t\t(at %.4f %.4f 0)\n\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n%s'
            '\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)\n\t\t\t\t(justify %s)\n\t\t\t)\n\t\t)\n'
            % (name, esc(val), x, y, "\t\t\t(hide yes)\n" if hide else "", justify))

def emit_part(part):
    block, pins = sym(part["lib"])
    x0, x1, y0, y1 = extent(pins)
    ref, x, y = part["ref"], part["x"], part["y"]
    power = ref.startswith("#")
    s = '\t(symbol\n\t\t(lib_id "%s")\n\t\t(at %.4f %.4f 0)\n\t\t(unit 1)\n\t\t(body_style 1)\n\t\t(exclude_from_sim no)\n\t\t(in_bom %s)\n\t\t(on_board %s)\n\t\t(in_pos_files yes)\n\t\t(dnp %s)\n\t\t(uuid "%s")\n' % (
        part["lib"], x, y, "no" if power else "yes", "no" if power else "yes", "yes" if part["dnp"] else "no", U("sym/" + ref))
    s += prop("Reference", ref, x + x0, y - y1 - 1.27, hide=power)
    s += prop("Value", part["value"], x + x0, y - y0 + 1.27)
    s += prop("Footprint", part["fp"], x, y, hide=True)
    s += prop("Datasheet", "", x, y, hide=True)
    s += prop("Description", part["desc"], x, y, hide=True)
    if not power:
        s += prop("LCSC", part["extra"].get("LCSC", ""), x, y, hide=True)
        for k, v in part["extra"].items():
            if k != "LCSC": s += prop(k, v, x, y, hide=True)
    for p in pins:
        s += '\t\t(pin "%s"\n\t\t\t(uuid "%s")\n\t\t)\n' % (p["num"], U("pin/%s/%s" % (ref, p["num"])))
    s += '\t\t(instances\n\t\t\t(project "pp-48-%ds"\n\t\t\t\t(path "/%s"\n\t\t\t\t\t(reference "%s")\n\t\t\t\t\t(unit 1)\n\t\t\t\t)\n\t\t\t)\n\t\t)\n\t)\n' % (N, ROOT_UUID, ref)
    return s

JUST = {0: "left bottom", 180: "right bottom", 90: "left bottom", 270: "right bottom"}
GLOBAL_NETS = {"GND", "+3V3", "+12V"}
def emit_label(x, y, ang, text):
    if text in GLOBAL_NETS:
        return ('\t(global_label "%s"\n\t\t(shape passive)\n\t\t(at %.4f %.4f %d)\n\t\t(fields_autoplaced yes)\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1.27 1.27)\n\t\t\t)\n\t\t\t(justify %s)\n\t\t)\n\t\t(uuid "%s")\n\t\t(property "Intersheetrefs" "${INTERSHEET_REFS}"\n\t\t\t(at %.4f %.4f 0)\n\t\t\t(show_name no)\n\t\t\t(do_not_autoplace no)\n\t\t\t(hide yes)\n\t\t\t(effects\n\t\t\t\t(font\n\t\t\t\t\t(size 1.27 1.27)\n\t\t\t\t)\n\t\t\t)\n\t\t)\n\t)\n'
                % (esc(text), x, y, ang, JUST[ang].replace("bottom", "top") if ang in (0, 180) else JUST[ang], U("glabel/%s/%.2f/%.2f" % (text, x, y)), x, y))
    return ('\t(label "%s"\n\t\t(at %.4f %.4f %d)\n\t\t(fields_autoplaced yes)\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size 1.27 1.27)\n\t\t\t)\n\t\t\t(justify %s)\n\t\t)\n\t\t(uuid "%s")\n\t)\n'
            % (esc(text), x, y, ang, JUST[ang], U("label/%s/%.2f/%.2f" % (text, x, y))))

def emit_nc(x, y):
    return '\t(no_connect\n\t\t(at %.4f %.4f)\n\t\t(uuid "%s")\n\t)\n' % (x, y, U("nc/%.2f/%.2f" % (x, y)))

def emit_wire(x1, y1, x2, y2):
    return ('\t(wire\n\t\t(pts\n\t\t\t(xy %.4f %.4f) (xy %.4f %.4f)\n\t\t)\n\t\t(stroke\n\t\t\t(width 0)\n\t\t\t(type default)\n\t\t)\n\t\t(uuid "%s")\n\t)\n'
            % (x1, y1, x2, y2, U("wire/%.2f/%.2f/%.2f/%.2f" % (x1, y1, x2, y2))))

def emit_text(x, y, text, size=2.0):
    return ('\t(text "%s"\n\t\t(exclude_from_sim no)\n\t\t(at %.4f %.4f 0)\n\t\t(effects\n\t\t\t(font\n\t\t\t\t(size %.2f %.2f)\n\t\t\t\t(bold yes)\n\t\t\t)\n\t\t\t(justify left bottom)\n\t\t)\n\t\t(uuid "%s")\n\t)\n'
            % (esc(text), x, y, size, size, U("text/%s" % text)))

# ---------------------------------------------------------------- footprints
R0402 = "Resistor_SMD:R_0402_1005Metric"; R0603 = "Resistor_SMD:R_0603_1608Metric"
R0805 = "Resistor_SMD:R_0805_2012Metric"; R1206 = "Resistor_SMD:R_1206_3216Metric"
R2512 = "Resistor_SMD:R_2512_6332Metric"
C0402 = "Capacitor_SMD:C_0402_1005Metric"; C0603 = "Capacitor_SMD:C_0603_1608Metric"
C0805 = "Capacitor_SMD:C_0805_2012Metric"; C1210 = "Capacitor_SMD:C_1210_3225Metric"
LED0603 = "LED_SMD:LED_0603_1608Metric"
SJ = "Jumper:SolderJumper-2_P1.3mm_Open_RoundedPad1.0x1.5mm"

# ---------------------------------------------------------------- power anchors
_pwrn = [0, 0]
def power_cluster(x, y, net, symbol, flag=True):
    """power symbol + label (+ PWR_FLAG) joined by a short wire"""
    _pwrn[0] += 1
    ref = "#PWR%02d" % _pwrn[0]
    block, pins = sym(symbol)
    parts.append(dict(ref=ref, lib=symbol, value=symbol.split(":")[1], fp="", x=x, y=y, nets={"1": None},
                      extra={}, dnp=False, desc="", pins=pins, skipnets=True))
    labels.append((x + 5.08, y, 0, net))
    wires.append((x, y, x + 5.08, y))
    if flag:
        _pwrn[1] += 1
        fref = "#FLG%02d" % _pwrn[1]
        fb, fpins = sym("power:PWR_FLAG")
        parts.append(dict(ref=fref, lib="power:PWR_FLAG", value="PWR_FLAG", fp="", x=x + 5.08, y=y, nets={"1": None},
                          extra={}, dnp=False, desc="", pins=fpins, skipnets=True))

def flag_net(x, y, net):
    """PWR_FLAG + label for a net with no power symbol"""
    _pwrn[1] += 1
    fref = "#FLG%02d" % _pwrn[1]
    fb, fpins = sym("power:PWR_FLAG")
    parts.append(dict(ref=fref, lib="power:PWR_FLAG", value="PWR_FLAG", fp="", x=x, y=y, nets={"1": None},
                      extra={}, dnp=False, desc="", pins=fpins, skipnets=True))
    labels.append((x + 5.08, y, 0, net))
    wires.append((x, y, x + 5.08, y))

# ================================================================ CORE
X0, Y0 = 20.32, 30.48
core = Block("CORE: input, 3V3 and 12 V aux bucks (LM5164), STM32G0B1, isolators, controller link", X0, Y0, 520.0)

# input
core.place("J1", "Connector:Screw_Terminal_01x02", "BUS 40-60V", "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-3-2-5.08_1x02_P5.08mm_Horizontal",
           {"1": "VBUS_IN", "2": "GND"}, desc="bus input, 24 A terminal (2-slot plane); lugs on the 8-slot plane")
core.place("F1", "Device:Fuse", "10A nano2", "Fuse:Fuse_Littelfuse-NANO2-451_453", {"1": "VBUS_IN", "2": "VBUS"},
           extra={"LCSC": "C44479"}, desc="0451010.MRL main fuse, 2-slot plane")
core.place("D1", "Device:D_TVS", "SMBJ60A", "Diode_SMD:D_SMB", {"1": "GND", "2": "VBUS"}, extra={"LCSC": "C353345"},
           desc="bus TVS, 60 V standoff, clamps below the 100 V parts")
core.place("C1", "Device:C", "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, desc="X7R")
core.place("C2", "Device:C", "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, desc="X7R")
core.place("R1", "Device:R", "470k", R0805, {"1": "VBUS", "2": "VBUS_SNS"}, desc="bus sense divider top, 150 V rated size")
core.place("R2", "Device:R", "13k", R0402, {"1": "VBUS_SNS", "2": "GND"}, desc="60 V -> 1.61 V at the ADC")
core.place("C3", "Device:C", "10n", C0402, {"1": "VBUS_SNS", "2": "GND"})

# 3V3 buck U1: LM5164, 200 kHz, type 3 ripple injection
def lm5164(u, vout, nets, rron, l, rfb1, cout, ca, rsuffix, cbase, lcsc_l=""):
    core.place(u, "Regulator_Switching:LM5164DDA", "LM5164DDA", "Package_SO:HSOP-8-1EP_3.9x4.9mm_P1.27mm_EP2.41x3.1mm",
               {"1": "GND", "2": "VBUS", "3": nets["en"], "4": nets["ron"], "5": nets["fb"], "6": "NC", "7": nets["bst"], "8": nets["sw"], "9": "GND"},
               desc="100 V 1 A COT buck, %s" % vout, wide=True)
    r = rsuffix; c = cbase
    core.place("R%d" % r, "Device:R", "1M", R0805, {"1": "VBUS", "2": nets["en"]}, desc="EN/UVLO top")
    core.place("R%d" % (r + 1), "Device:R", "52.3k", R0402, {"1": nets["en"], "2": "GND"}, desc="on at 30 V, off at 28 V")
    core.place("R%d" % (r + 2), "Device:R", rron, R0402, {"1": nets["ron"], "2": "GND"}, desc="FSW = VOUT x 2500 / RRON")
    core.place("R%d" % (r + 3), "Device:R", rfb1, R0402, {"1": nets["vout"], "2": nets["fb"]}, desc="feedback top")
    core.place("R%d" % (r + 4), "Device:R", "49.9k", R0402, {"1": nets["fb"], "2": "GND"}, desc="feedback bottom, VFB 1.2 V")
    core.place("R%d" % (r + 5), "Device:R", "453k", R0402, {"1": nets["sw"], "2": nets["inj"]}, desc="ripple injection RA (type 3)")
    core.place("C%d" % c, "Device:C", "2.2n 50V", C0402, {"1": nets["bst"], "2": nets["sw"]}, desc="bootstrap")
    core.place("C%d" % (c + 1), "Device:C", ca, C0402, {"1": nets["inj"], "2": nets["fb"]}, desc="ripple injection CA")
    core.place("C%d" % (c + 2), "Device:C", "100n", C0402, {"1": nets["fb"], "2": nets["vout"]}, desc="ripple injection CB (DC block)")
    core.place("C%d" % (c + 3), "Device:C", "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, desc="input, at the VIN pin")
    core.place("C%d" % (c + 4), "Device:C", "100n 100V", C0603, {"1": "VBUS", "2": "GND"}, desc="input HF")
    core.place("C%d" % (c + 5), "Device:C", cout, C1210, {"1": nets["vout"], "2": "GND"}, desc="output X7R")
    core.place("C%d" % (c + 6), "Device:C", cout, C1210, {"1": nets["vout"], "2": "GND"}, desc="output X7R")
    core.place("L%d" % (1 if u == "U1" else 2), "Device:L", l, "Inductor_SMD:L_12x12mm_H6mm", {"1": nets["sw"], "2": nets["vout"]},
               desc="shielded, Isat >= 1.5 A (Bourns SRR1260 class)")

lm5164("U1", "3.3 V", dict(en="EN1", ron="RON1", fb="FB1", bst="BST1", sw="SW1", inj="INJ1", vout="+3V3"),
       "41.2k", "33u", "86.6k", "22u 10V", "1.5n", 3, 4)
lm5164("U2", "12 V aux for the controller", dict(en="EN2", ron="RON2", fb="FB2", bst="BST2", sw="SW2", inj="INJ2", vout="+12V"),
       "100k", "68u", "453k", "22u 25V", "2.2n", 9, 11)
core.place("F2", "Device:Fuse", "1.5A", "Fuse:Fuse_1206_3216Metric", {"1": "+12V", "2": "V12_OUT"}, desc="aux output fuse")
core.place("J4", "Connector_Generic:Conn_01x02", "12V AUX OUT", "Connector_JST:JST_PH_B2B-PH-K_1x02_P2.00mm_Vertical",
           {"1": "V12_OUT", "2": "GND"}, desc="always-on 12 V for the controller J3")
core.place("R15", "Device:R", "100k", R0402, {"1": "+12V", "2": "AUX_SNS"}, desc="aux sense divider top")
core.place("R16", "Device:R", "27k", R0402, {"1": "AUX_SNS", "2": "GND"}, desc="12 V -> 2.55 V at the ADC")
core.place("R17", "Device:R", "4.7k", R0603, {"1": "V12_OUT", "2": "LED12_K"}, desc="12V LED")
core.place("D2", "Device:LED", "GREEN 12V", LED0603, {"1": "LED12_K", "2": "GND"}, extra={"LCSC": "C2297"})
core.place("R18", "Device:R", "1k", R0402, {"1": "+3V3", "2": "LED33_K"}, desc="3V3 LED")
core.place("D3", "Device:LED", "GREEN 3V3", LED0603, {"1": "LED33_K", "2": "GND"}, extra={"LCSC": "C2297"})

# MCU
mcu_nets = {
    "1": "ADR0", "2": "ADR1", "3": "ADR2", "4": "+3V3", "5": "+3V3", "6": "+3V3", "7": "GND",
    "8": "OSC_IN", "9": "OSC_OUT", "10": "NRST",
    "11": "S1_ID", "12": "S1_TEMP", "13": "S2_ID", "14": "S2_TEMP", "15": "S3_ID", "16": "S3_TEMP", "17": "S4_ID", "18": "S4_TEMP",
    "19": "VBUS_SNS", "20": "AUX_SNS", "21": "NC",
    "22": "SCL2", "23": "SDA2", "24": "S5_OFF", "25": "S5_LCLR", "26": "S6_OFF", "27": "S6_LCLR",
    "28": "INT_OUT", "29": "S7_OFF", "32": "S7_LCLR", "33": "S8_OFF", "34": "S8_LCLR",
    "30": "S1_OFF", "31": "S1_LCLR", "38": "S2_OFF", "39": "S2_LCLR", "40": "S3_OFF", "41": "S3_LCLR", "42": "S4_OFF", "43": "S4_LCLR",
    "35": "SWDIO", "36": "SWCLK", "37": "NC", "44": "EXP_IRQ", "45": "SCL1", "46": "SDA1", "47": "LED_RUN", "48": "NC",
}
# slots that do not exist on this plane size: leave their MCU pins unconnected
for k, v in list(mcu_nets.items()):
    m = re.match(r"S(\d)_", v or "")
    if m and int(m.group(1)) > N:
        mcu_nets[k] = "NC"
core.place("U3", "MCU_ST_STM32G0:STM32G0B1CBTx", "STM32G0B1CBT6", "Package_QFP:LQFP-48_7x7mm_P0.5mm", mcu_nets,
           desc="plane MCU: I2C1 slave to the controller (PB6/PB7), I2C2 master (PB10/PB11), ADC on PA0-7/PB0-1, SWD on PA13/PA14")
core.place("C18", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, desc="U3 VDD")
core.place("C19", "Device:C", "4.7u", C0603, {"1": "+3V3", "2": "GND"}, desc="U3 VDD bulk")
core.place("C20", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, desc="U3 VREF+/VBAT")
core.place("C21", "Device:C", "100n", C0402, {"1": "NRST", "2": "GND"}, desc="NRST")
core.place("Y1", "Device:Crystal_GND24", "8MHz", "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm", {"1": "OSC_IN", "2": "GND", "3": "OSC_OUT", "4": "GND"},
           desc="HSE, energy time base")
core.place("C22", "Device:C", "12p", C0402, {"1": "OSC_IN", "2": "GND"})
core.place("C23", "Device:C", "12p", C0402, {"1": "OSC_OUT", "2": "GND"})
core.place("J5", "Connector:Conn_ARM_SWD_TagConnect_TC2030-NL", "SWD", "Connector:Tag-Connect_TC2030-IDC-NL_2x03_P1.27mm_Vertical",
           {"1": "+3V3", "2": "SWDIO", "3": "NRST", "4": "SWCLK", "5": "GND", "6": "NC"}, desc="bench programming")
core.place("R19", "Device:R", "1k", R0402, {"1": "LED_RUN", "2": "LEDRUN_K"}, desc="firmware heartbeat LED")
core.place("D4", "Device:LED", "GREEN RUN", LED0603, {"1": "LEDRUN_K", "2": "GND"}, extra={"LCSC": "C2297"})
for i, net in enumerate(["ADR0", "ADR1", "ADR2"]):
    core.place("R%d" % (20 + i), "Device:R", "100k", R0402, {"1": "+3V3", "2": net}, desc="address strap pull-up")
    core.place("JP%d" % (1 + i), "Jumper:SolderJumper_2_Open", "ADR%d" % i, SJ, {"1": net, "2": "GND"}, desc="close = 0; plane address 0x30 + ADR[2:0]")
core.place("R23", "Device:R", "2.2k", R0402, {"1": "+3V3", "2": "SDA2"}, desc="I2C2 pull-up (INA238s, MCP23017s)")
core.place("R24", "Device:R", "2.2k", R0402, {"1": "+3V3", "2": "SCL2"})
core.place("R25", "Device:R", "10k", R0402, {"1": "+3V3", "2": "EXP_IRQ"}, desc="expander INT wired-OR pull-up")

# controller link: ISO1540 (I2C) + ISO7721 (EXP_INT), controller side on the cable's 3V3/GND
core.place("U4", "Isolator:ISO1540", "ISO1640BD", "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
           {"1": "CTRL_3V3", "2": "CTRL_SDA", "3": "CTRL_SCL", "4": "CTRL_GND", "5": "GND", "6": "SCL1", "7": "SDA1", "8": "+3V3"},
           desc="isolated I2C; side 1 = controller cable, side 2 = plane. ISO1540 symbol, ISO1640 pin compatible")
core.place("C24", "Device:C", "100n", C0402, {"1": "CTRL_3V3", "2": "CTRL_GND"}, desc="U4 VCC1")
core.place("C25", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, desc="U4 VCC2")
core.place("R26", "Device:R", "4.7k", R0402, {"1": "+3V3", "2": "SDA1"}, desc="I2C1 pull-up, isolator side 2")
core.place("R27", "Device:R", "4.7k", R0402, {"1": "+3V3", "2": "SCL1"})
core.place("R28", "Device:R", "4.7k", R0402, {"1": "CTRL_3V3", "2": "CTRL_SDA"}, dnp=True, desc="DNP: the controller's J8 bus carries the pull-ups")
core.place("R29", "Device:R", "4.7k", R0402, {"1": "CTRL_3V3", "2": "CTRL_SCL"}, dnp=True)
core.place("U5", "Isolator:ISO7721D", "ISO7721D", "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
           {"1": "CTRL_3V3", "2": "ISO_OUTA", "3": "CTRL_GND", "4": "CTRL_GND", "5": "GND", "6": "NC", "7": "INT_OUT", "8": "+3V3"},
           desc="EXP_INT plane -> controller (channel A, side 2 in, side 1 out); channel B unused")
core.place("C26", "Device:C", "100n", C0402, {"1": "CTRL_3V3", "2": "CTRL_GND"}, desc="U5 VCC1")
core.place("C27", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, desc="U5 VCC2")
core.place("Q1", "Transistor_FET:2N7002", "2N7002", "Package_TO_SOT_SMD:SOT-23", {"1": "ISO_OUTA", "2": "CTRL_GND", "3": "CTRL_INT"},
           extra={"LCSC": "C8545"}, desc="open-drain EXP_INT on the controller side (MCU drives INT_OUT high to assert)")
core.place("R30", "Device:R", "100k", R0402, {"1": "ISO_OUTA", "2": "CTRL_GND"}, desc="gate pull-down")
for j in ("J2", "J3"):
    core.place(j, "Connector_Generic:Conn_01x05", "CTRL " + ("IN" if j == "J2" else "OUT"), "Connector_JST:JST_PH_B5B-PH-K_1x05_P2.00mm_Vertical",
               {"1": "CTRL_3V3", "2": "CTRL_GND", "3": "CTRL_SCL", "4": "CTRL_SDA", "5": "CTRL_INT"},
               desc="controller link, daisy chain; 3V3 GND SCL SDA INT, from J8 of the controller")

# expanders: one MCP23017 per 2 slots, addresses 0x20.. ; GPA = odd slot, GPB = even slot
# per slot: bit0 PWR drive, bit1 RST drive, bit2 PGD (LM5069), bit3 PGOOD (module), bit4 LATCHED, bit5 LED sense, 6-7 spare
def gp(s, bank, k):
    return "S%d_%s" % (s, ["PWRD", "RSTD", "PGD", "PG", "LQ", "LED", "NC", "NC"][k])
for e in range((N + 1) // 2):
    sa, sb = 2 * e + 1, 2 * e + 2
    nets = {"9": "+3V3", "10": "GND", "11": "NC", "14": "NC", "12": "SCL2", "13": "SDA2",
            "15": "+3V3" if e & 1 else "GND", "16": "+3V3" if e & 2 else "GND", "17": "GND",
            "18": "+3V3", "19": "NC", "20": "EXP_IRQ"}
    for k in range(8):
        nets[str(21 + k)] = gp(sa, "A", k) if k < 6 else "NC"            # GPA0..7 = pins 21..28
        nets[str(1 + k)] = (gp(sb, "B", k) if (k < 6 and sb <= N) else "NC")  # GPB0..7 = pins 1..8
    ref = "U%d" % (6 + e)
    core.place(ref, "Interface_Expansion:MCP23017x-x-SO", "MCP23017-E/SO", "Package_SO:SOIC-28W_7.5x17.9mm_P1.27mm", nets,
               desc="slot GPIO for slots %d-%d, address 0x%02X, INTA mirrored open-drain, never reset by the controller" % (sa, sb, 0x20 + e), wide=True)
    core.place("C%d" % (28 + e), "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, desc=ref + " VDD")

# ================================================================ SLOTS
SLOT_W = 520.0
slot_y = Y0 + core.height() + 20.0
for s in range(1, N + 1):
    b = Block("SLOT %d: LM5069-2 + INA238 on 3.5 mOhm, OV latch, brick socket, node connector (PWR/RST/LED)" % s, X0, slot_y, SLOT_W)
    P = "S%d_" % s
    r = s * 100
    def R(n, val, fp, nets, **kw): b.place("R%d" % (r + n), "Device:R", val, fp, nets, **kw)
    def C(n, val, fp, nets, **kw): b.place("C%d" % (r + n), "Device:C", val, fp, nets, **kw)
    # hot-swap
    b.place("U%d" % (r + 1), "Power_Management:LM5069MM-2", "LM5069MM-2", "Package_SO:MSOP-10_3x3mm_P0.5mm",
            {"1": P + "SENSE", "2": "VBUS", "3": P + "UVLO", "4": P + "OVLO", "5": "GND", "6": P + "TIMER", "7": P + "PWR", "8": P + "PGD", "9": P + "OUT", "10": P + "GATE"},
            extra={"LCSC": "C111822"}, desc="hot-swap, 55 mV current limit, power limit, auto-retry 0.5 %", wide=True)
    R(1, "7m 2W", R2512, {"1": "VBUS", "2": P + "SENSE"}, desc="shunt, two 7 mOhm in parallel = 3.5 mOhm: limit 15.7 A, breaker 30 A")
    R(2, "7m 2W", R2512, {"1": "VBUS", "2": P + "SENSE"}, desc="shunt (parallel)")
    b.place("Q%d" % (r + 1), "Transistor_FET:BSC070N10NS5", "BSC070N10NS5", "Package_TO_SOT_SMD:TDSON-8-1",
            by_name("Transistor_FET:BSC070N10NS5", {"G": P + "GATE", "S": P + "OUT", "D": P + "SENSE"}),
            desc="100 V pass MOSFET, placeholder: SOA check open (ADR 0001)")
    R(3, "143k", R0805, {"1": "VBUS", "2": P + "UVLO"}, desc="UVLO: on 38.3 V, off 35.3 V (21 uA hysteresis)")
    R(4, "10k", R0402, {"1": P + "UVLO", "2": "GND"})
    R(5, "240k", R0805, {"1": "VBUS", "2": P + "OVLO"}, desc="OVLO: off 62.5 V, back at 57.5 V")
    R(6, "10k", R0402, {"1": P + "OVLO", "2": "GND"})
    R(7, "13.7k", R0402, {"1": P + "PWR", "2": "GND"}, desc="power limit ~50 W at 60 V (RPWR = 1.3e5 x (RSNS x PLIM - 1.18mV x VDS))")
    C(1, "220n", C0603, {"1": P + "TIMER", "2": "GND"}, desc="insertion 160 ms, fault timeout 10 ms")
    R(8, "10k", R0402, {"1": "+3V3", "2": P + "PGD"}, desc="PGD pull-up")
    b.place("D%d" % (r + 1), "Device:D_TVS", "SMBJ60A", "Diode_SMD:D_SMB", {"1": "GND", "2": P + "OUT"}, extra={"LCSC": "C353345"},
            desc="slot output clamp (cable inductance on a cut)")
    C(2, "2.2u 100V", C1210, {"1": P + "OUT", "2": "GND"}, desc="slot output, in front of the brick")
    # monitor
    b.place("U%d" % (r + 2), "Sensor_Energy:INA238", "INA238AIDGSR", "Package_SO:TSSOP-10_3x3mm_P0.5mm",
            {"1": ["GND", "GND", "GND", "GND", "+3V3", "+3V3", "+3V3", "+3V3"][s - 1], "2": ["GND", "+3V3", "SDA2", "SCL2"][(s - 1) % 4],
             "3": "NC", "4": "SDA2", "5": "SCL2", "6": "+3V3", "7": "GND", "8": P + "OUT", "9": P + "SENSE", "10": "VBUS"},
            desc="16-bit monitor on the shunt, address 0x%02X; VBUS pin reads the slot output" % (0x40 + s - 1))
    C(3, "100n", C0402, {"1": "+3V3", "2": "GND"}, desc="INA238 VS")
    # enable chain: UVLO pulled low by the MCU (OFF) or the OV latch
    b.place("Q%d" % (r + 2), "Transistor_FET:2N7002", "2N7002", "Package_TO_SOT_SMD:SOT-23", {"1": P + "OFF", "2": "GND", "3": P + "UVLO"},
            extra={"LCSC": "C8545"}, desc="MCU OFF: high = slot off; MCU dead = pull-down = ON")
    R(9, "100k", R0402, {"1": P + "OFF", "2": "GND"}, desc="default ON")
    b.place("Q%d" % (r + 3), "Transistor_FET:2N7002", "2N7002", "Package_TO_SOT_SMD:SOT-23", {"1": P + "LQ", "2": "GND", "3": P + "UVLO"},
            extra={"LCSC": "C8545"}, desc="OV latch output cuts the slot")
    R(10, "100k", R0402, {"1": P + "LQ", "2": "GND"})
    b.place("U%d" % (r + 3), "74xGxx:74AUC1G74", "74LVC1G74", "Package_SO:VSSOP-8_2.3x2mm_P0.5mm",
            {"1": "GND", "2": "GND", "3": "NC", "4": "GND", "5": P + "LQ", "6": P + "LCLR", "7": P + "OVK", "8": "+3V3"},
            desc="OV latch: /PRE = module OV_KILL (low = set), /CLR = RC power-on clear + MCU; Q holds UVLO low until cleared")
    C(4, "100n", C0402, {"1": "+3V3", "2": "GND"}, desc="latch VCC")
    R(11, "100k", R0402, {"1": "+3V3", "2": P + "LCLR"}, desc="power-on clear RC (10 ms)")
    C(5, "100n", C0402, {"1": P + "LCLR", "2": "GND"})
    b.place("Q%d" % (r + 4), "Transistor_FET:2N7002", "2N7002", "Package_TO_SOT_SMD:SOT-23", {"1": P + "LCLR_D", "2": "GND", "3": P + "LCLR"},
            extra={"LCSC": "C8545"}, desc="MCU clears the latch (drive high)")
    R(12, "100k", R0402, {"1": P + "LCLR_D", "2": "GND"})
    R(13, "10k", R0402, {"1": "+3V3", "2": P + "OVK"}, desc="OV_KILL pull-up (module comparator is open drain)")
    # brick socket + J2 mate
    b.place("X%d" % (r + 1), "pp:PM_Brick_Socket", "PM-19V-200W", "pp:Brick_Eighth_DOSA_Plane",
            {"1": P + "OUT", "2": "NC", "3": "GND", "4": "GND", "5": "NC", "6": "NC", "7": "NC", "8": P + "NODE"},
            desc="DOSA eighth brick, soldered pins; ON/OFF open = module on (the slot is switched upstream)", wide=True)
    b.place("X%d" % (r + 2), "Connector_Generic:Conn_01x07", "J2 AUX", "Connector_PinHeader_1.27mm:PinHeader_1x07_P1.27mm_Vertical",
            {"1": P + "OVK", "2": P + "PG", "3": P + "ID", "4": P + "TEMP", "5": "GND", "6": "SDA2", "7": "SCL2"},
            desc="module J2 mate: OV_KILL PGOOD ID TEMP GND SDA SCL (6-7 reserved; a module monitor must use 0x48-0x4F)")
    R(14, "10k", R0402, {"1": "+3V3", "2": P + "PG"}, desc="module PGOOD pull-up")
    R(15, "10k", R0402, {"1": "+3V3", "2": P + "ID"}, desc="ID divider top (module resistor to GND)")
    C(6, "100n", C0402, {"1": P + "ID", "2": "GND"})
    R(16, "10k", R0402, {"1": "+3V3", "2": P + "TEMP"}, desc="NTC divider top")
    C(7, "100n", C0402, {"1": P + "TEMP", "2": "GND"})
    C(8, "2.2u 50V", C1210, {"1": P + "NODE", "2": "GND"}, desc="node output, cable side")
    R(17, "47k", R1206, {"1": P + "OUT", "2": P + "ONL_K"}, desc="slot ON LED from 54 V (1.1 mA, 65 mW)")
    b.place("D%d" % (r + 2), "Device:LED", "GREEN ON", LED0603, {"1": P + "ONL_K", "2": "GND"}, extra={"LCSC": "C2297"})
    R(18, "10k", R0603, {"1": P + "NODE", "2": P + "NL_K"}, desc="node 19 V LED")
    b.place("D%d" % (r + 3), "Device:LED", "GREEN NODE", LED0603, {"1": P + "NL_K", "2": "GND"}, extra={"LCSC": "C2297"})
    # node connector + front panel
    b.place("J%d" % (r + 1), "Connector_Generic:Conn_02x04_Odd_Even", "NODE %d" % s, "Connector_Molex:Molex_Mini-Fit_Jr_5566-08A_2x04_P4.20mm_Vertical",
            {"1": P + "NODE", "2": P + "NODE", "3": "GND", "4": "GND", "5": P + "PWR_SW", "6": P + "RST_SW", "7": P + "PLED_P", "8": P + "PLED_N"},
            desc="one cable per node: 1-2 +19 V, 3-4 GND, 5 PWR_SW, 6 RST_SW, 7 PLED+, 8 PLED-", wide=True)
    for k, (sig, drv) in enumerate((("PWR_SW", "PWRD"), ("RST_SW", "RSTD"))):
        b.place("U%d" % (r + 4 + k), "pp:TLP176AM", "TLP176AM", "Package_SO:SOIC-4_4.55x3.7mm_P2.54mm",
                {"1": P + drv + "_A", "3": "GND", "4": "GND", "6": P + sig}, desc="photoMOS front-panel %s contact" % sig)
        R(19 + k, "270", R0402, {"1": P + drv, "2": P + drv + "_A"}, extra={"LCSC": "C22775"}, desc="photoMOS LED, ~6.5 mA from the expander")
    b.place("U%d" % (r + 6), "Isolator:TLP291", "TLP291", "Package_SO:SOIC-4_4.55x2.6mm_P1.27mm",
            {"1": P + "PLED_A", "2": P + "PLED_N", "3": "GND", "4": P + "LED"}, desc="power LED sense opto: LED lit -> output low")
    R(21, "220", R0402, {"1": P + "PLED_P", "2": P + "PLED_A"}, desc="opto LED series (as rev C)")
    R(22, "47k", R0402, {"1": "+3V3", "2": P + "LED"}, desc="opto collector pull-up")
    slot_y += b.height() + 20.0

# ================================================================ power anchors
ay = round((slot_y + 10.0) / 1.27) * 1.27
power_cluster(X0, ay, "GND", "power:GND")
power_cluster(X0 + 38.1, ay, "+3V3", "power:+3V3")
power_cluster(X0 + 76.2, ay, "+12V", "power:+12V")
flag_net(X0 + 114.3, ay, "VBUS")
flag_net(X0 + 152.4, ay, "VBUS_IN")
flag_net(X0 + 190.5, ay, "CTRL_3V3")
flag_net(X0 + 228.6, ay, "CTRL_GND")
flag_net(X0 + 266.7, ay, "V12_OUT")
for s in range(1, N + 1):
    flag_net(X0 + 304.8 + 38.1 * (s - 1), ay, "S%d_OUT" % s)
texts.append((X0, ay - 6, "POWER ANCHORS (power symbols joined to the net labels)"))

# ---------------------------------------------------------------- build
for part in parts:
    if not part.get("skipnets"):
        finish_part(part)

# label sanity: every net on >= 2 points
from collections import Counter
cnt = Counter(l[3] for l in labels)
single = sorted(n for n, c in cnt.items() if c < 2)
if single:
    print("WARNING single-ended nets:", single, file=sys.stderr)

height = ay + 30
paper = "A2" if height < 400 else "A1"
out = ['(kicad_sch', '\t(version 20260306)', '\t(generator "pp-gen")', '\t(generator_version "10.0")',
       '\t(uuid "%s")' % ROOT_UUID, '\t(paper "%s")' % paper,
       '\t(title_block', '\t\t(title "PP-48-%dS power plane")' % N, '\t\t(date "2026-10-23")', '\t\t(rev "0.1")', '\t\t(company "Granite")',
       '\t\t(comment 1 "48 V plane, %d eighth-brick slots: LM5069-2 + INA238 per slot, OV latch, node PWR/RST/LED, STM32G0B1, isolated I2C")' % N,
       '\t\t(comment 2 "generated by tools/gensch.py; docs/adr/0001-48v-power-plane.md")',
       '\t\t(comment 3 "immersion board: no wet electrolytics")', '\t)', '\t(lib_symbols']
for k in sorted(SYMS):
    out.append("\t\t" + SYMS[k][0].replace("\n", "\n\t"))
out.append('\t)')
body = ""
for x1, y1, x2, y2 in wires: body += emit_wire(x1, y1, x2, y2)
for x, y, a, t in labels: body += emit_label(x, y, a, t)
for x, y in ncs: body += emit_nc(x, y)
for x, y, t in texts: body += emit_text(x, y, t)
for part in parts: body += emit_part(part)
out.append(body.rstrip("\n"))
out.append('\t(sheet_instances\n\t\t(path "/"\n\t\t\t(page "1")\n\t\t)\n\t)\n\t(embedded_fonts no)\n)')
open(OUT, "w").write("\n".join(out) + "\n")
print("wrote", OUT, "parts", len(parts), "labels", len(labels), "paper", paper, "height", round(height))
