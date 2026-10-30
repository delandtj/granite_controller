#!/usr/bin/env python3
"""Generate the PM-19V-250W-B schematic (KiCad 10, one sheet) and its symbol
library pmb.kicad_sym.

usage: tools/gensch.py

Single-phase LM5116 port module, docs/pm-19v-250w-b.md (component spec:
values, equations, BOM). Uses the plane's schematic engine
(power/pp-48/tools/sch, Block API in draw.py). Every net is joined by name
labels at the pins (b.tie), GND by power symbols, so the sheet reads as
functional groups with headings rather than drawn wires.

The .kicad_sch and pmb.kicad_sym are output, not source: edit this file and
regenerate.
"""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
PROJ_DIR = os.path.dirname(HERE)
ENGINE = os.path.join(PROJ_DIR, "..", "pp-48", "tools")
sys.path.insert(0, os.path.normpath(ENGINE))
from sch import kicad as K                                       # noqa: E402
from sch.draw import Block, R0402, R0603, R0805, R1206, R2512, C0402, C0805, C1210   # noqa: E402

PROJECT = "pm-19v-250w-b"
OUT = os.path.join(PROJ_DIR, PROJECT + ".kicad_sch")
SYMLIB = os.path.join(PROJ_DIR, "pmb.kicad_sym")
DATE = "2026-10-30"

# ---------------------------------------------------------------- symbol library
F = "(effects (font (size 1.27 1.27)))"
FH = "(effects (font (size 1.27 1.27)) (hide yes))"


def symbol(name, ref, fp, ds, desc, body, pins):
    """body = (x0, y0, x1, y1) lib mm, y up; pins = [(num, name, type, x, y, rot)]"""
    x0, y0, x1, y1 = body
    o = ['\t(symbol "%s" (exclude_from_sim no) (in_bom yes) (on_board yes)' % name,
         '\t\t(property "Reference" "%s" (at %.2f %.2f 0) %s)' % (ref, x0, y0 + 1.27, F),
         '\t\t(property "Value" "%s" (at %.2f %.2f 0) %s)' % (name, x1, y0 + 1.27, F),
         '\t\t(property "Footprint" "%s" (at 0 0 0) %s)' % (fp, FH),
         '\t\t(property "Datasheet" "%s" (at 0 0 0) %s)' % (ds, FH),
         '\t\t(property "Description" "%s" (at 0 0 0) %s)' % (desc, FH),
         '\t\t(symbol "%s_0_1" (rectangle (start %.2f %.2f) (end %.2f %.2f) (stroke (width 0.254) (type default)) (fill (type background))))'
         % (name, x0, y0, x1, y1),
         '\t\t(symbol "%s_1_1"' % name]
    for num, nm, typ, x, y, rot in pins:
        hide = " (hide yes)" if nm.startswith("~") else ""
        o.append('\t\t\t(pin %s line (at %.2f %.2f %d) (length 2.54)%s (name "%s" %s) (number "%s" %s))'
                 % (typ, x, y, rot, hide, nm.lstrip("~"), F, num, F))
    o.append('\t\t)\n\t)')
    return "\n".join(o)


def lm5116():
    L, Rr = -12.7, 12.7
    left = [("1", "VIN", "power_in"), ("4", "EN", "input"), ("2", "UVLO", "input"), ("3", "RT/SYNC", "passive"),
            ("5", "RAMP", "passive"), ("7", "SS", "passive"), ("8", "FB", "input"), ("9", "COMP", "output"),
            ("10", "VOUT", "input")]
    right = [("18", "HB", "passive"), ("19", "HO", "output"), ("20", "SW", "passive"), ("15", "LO", "output"),
             ("16", "VCC", "power_out"), ("17", "VCCX", "input"), ("12", "CS", "input"), ("13", "CSG", "input"),
             ("11", "DEMB", "input")]
    pins = [(n, nm, t, L, 11.43 - 2.54 * i, 0) for i, (n, nm, t) in enumerate(left)]
    pins += [(n, nm, t, Rr, 11.43 - 2.54 * i, 180) for i, (n, nm, t) in enumerate(right)]
    pins += [("6", "AGND", "power_in", -2.54, -13.97, 90), ("14", "PGND", "power_in", 0.0, -13.97, 90),
             ("21", "EP", "power_in", 2.54, -13.97, 90)]
    return symbol("LM5116", "U", "Package_SO:HTSSOP-20-1EP_4.4x6.5mm_P0.65mm_EP3.4x6.5mm_ThermalVias",
                  "https://www.ti.com/lit/ds/symlink/lm5116.pdf",
                  "100 V synchronous buck controller, emulated peak current mode, HTSSOP-20 PowerPAD (PWP)",
                  (-10.16, 13.97, 10.16, -11.43), pins)


def tps3701():
    pins = [("3", "INA", "input", -10.16, 2.54, 0), ("4", "INB", "input", -10.16, -2.54, 0),
            ("1", "OUTA", "open_collector", 10.16, 2.54, 180), ("6", "OUTB", "open_collector", 10.16, -2.54, 180),
            ("5", "VDD", "power_in", 0.0, 7.62, 270), ("2", "GND", "power_in", 0.0, -7.62, 90)]
    return symbol("TPS3701", "U", "Package_TO_SOT_SMD:SOT-23-6", "https://www.ti.com/lit/ds/symlink/tps3701.pdf",
                  "window comparator, 400 mV reference, two open-drain outputs (OUTA low: INA under; OUTB low: INB over), 1.8-36 V",
                  (-7.62, 5.08, 7.62, -5.08), pins)


def write_symlib():
    txt = ('(kicad_symbol_lib\n\t(version 20241209)\n\t(generator "pmb-gen")\n\t(generator_version "10.0")\n'
           + lm5116() + "\n" + tps3701() + "\n)\n")
    with open(SYMLIB, "w") as f:
        f.write(txt)
    K._libcache["pmb"] = txt


write_symlib()

# ---------------------------------------------------------------- parts
FET = "Package_TO_SOT_SMD:TDSON-8-1"
SOD323, SOD123 = "Diode_SMD:D_SOD-323", "Diode_SMD:D_SOD-123"
LXAL = "Inductor_SMD:L_Coilcraft_XAL1510-103"
CP63 = "Capacitor_SMD:CP_Elec_6.3x7.7"
SOT236, SOT89 = "Package_TO_SOT_SMD:SOT-23-6", "Package_TO_SOT_SMD:SOT-89-3"
NETTIE = "NetTie:NetTie-2_SMD_Pad0.5mm"

b = Block("Root", key=PROJECT + "/root", project=PROJECT, paper="A3", date=DATE)


def part(ref, lib, value, fp, nets, at, rot=0, lcsc="", desc="", dnp=False, mirror=None):
    s = b.sym(ref, lib, value, fp, nets, at=at, rot=rot, mirror=mirror, fields={"LCSC": lcsc} if lcsc else {},
              desc=desc, dnp=dnp)
    done = set()
    for num, net in nets.items():
        if net in ("GND", "NC"):
            continue
        xy = s.pin_xy(num)
        if xy in done:
            continue                      # stacked pins (FET sources): one label
        done.add(xy)
        b.tie(net, (ref, num))
    gnd = [num for num, net in nets.items() if net == "GND"]
    if len(gnd) > 1:                      # one GND symbol, the other GND pins wired to it
        b.wire("GND", [(ref, num) for num in gnd])
    return s


def two(ref, lib, value, fp, n1, n2, x, y, lcsc, desc="", dnp=False, flip=False):
    """horizontal two-pin part: pin 1 left (rot 90 puts R/C pin 1 left), pin 2 right"""
    if lib in ("Device:D", "Device:D_Zener"):       # pin 1 (K) left at rot 0
        return part(ref, lib, value, fp, {"1": n1, "2": n2}, (x, y), 180 if flip else 0, lcsc, desc, dnp)
    return part(ref, lib, value, fp, {"1": n1, "2": n2}, (x, y), 270 if flip else 90, lcsc, desc, dnp)


def heading(txt, x, y):
    b.text(txt, (x, y), size=2.0)


def note(txt, x, y):
    b.text(txt, (x, y), size=1.5)


R, C, CP, L, D, DZ = "Device:R", "Device:C", "Device:C_Polarized", "Device:L", "Device:D", "Device:D_Zener"
P = 7.62          # row pitch

# ---- mount and aux (docs/pm-interface.md)
x, y = 40.64, 40.64
heading("MOUNT AND AUX (docs/pm-interface.md)", x - 20.32, y - 10.16)
part("J1", "Connector_Generic:Conn_01x04", "Screw pads S1-S4", "pm:PM_Mount_M2",
     {"1": "VIN", "2": "GND", "3": "GND", "4": "VOUT"}, (x, y), lcsc="",
     desc="four M2 screws: S1 VIN, S2 GND (input), S3 GND (output), S4 VOUT; no part, not in BOM")
part("J2", "Connector_Generic:Conn_01x06", "Aux header 2x3 1.27", "pm:PM_Aux_Header_2x03_P1.27",
     {"1": "OV_KILL", "2": "PGOOD", "3": "ID", "4": "RUN", "5": "GND", "6": "GND"}, (x, y + 25.4),
     lcsc="C41376035", desc="module B side; pin n = An (1 OV_KILL, 2 PGOOD, 3 ID, 4 RUN, 5/6 GND)")
two("R1", R, "22k (ID)", R0603, "ID", "GND", x - 15.24, y + 45.72, "C31850", "ID resistor: 22k = PM-19V-250W class")
note("RUN = the UVLO node: open = on, host open drain low = off", x - 20.32, y + 55.88)

# ---- input
x, y = 40.64, 129.54
heading("INPUT (100 V parts)", x - 20.32, y - 10.16)
for i in range(6):
    two("C%d" % (i + 1), C, "4.7u 100V", C1210, "VIN", "GND", x - 7.62, y + P * i, "C381466",
        "input MLCC X7R 1210, next to Q1 drain / R4 GND")
two("R2", R, "10R", R0603, "VIN", "VIN_IC", x - 7.62, y + P * 7, "C22859", "U1 VIN pin filter")
two("C7", C, "1u 100V", C0805, "VIN_IC", "GND", x - 7.62, y + P * 8, "C5370002", "U1 VIN pin bypass")

# ---- controller
ux, uy = 170.18, 78.74
heading("CONTROLLER LM5116 (fsw 252 kHz, emulated peak current mode)", ux - 50.8, uy - 27.94)
part("U1", "pmb:LM5116", "LM5116MHX", "Package_SO:HTSSOP-20-1EP_4.4x6.5mm_P0.65mm_EP3.4x6.5mm_ThermalVias",
     {"1": "VIN_IC", "4": "VIN_IC", "2": "RUN", "3": "RT", "5": "RAMP", "7": "SS", "8": "FB", "9": "COMP",
      "10": "VOUT", "18": "HB", "19": "HO", "20": "SW", "15": "LO", "16": "VCC", "17": "VCCX", "12": "CS",
      "13": "CSG", "11": "DEMB", "6": "GND", "14": "GND", "21": "GND"}, (ux, uy), lcsc="C13755",
     desc="LM5116MHX/NOPB, EN tied to VIN (UVLO / RUN is the on-off input)")
# left column: analog parts (pin 1 of each on the U1 net)
lx = ux - 50.8
two("R7", R, "12.4k", R0402, "RT", "GND", lx, uy - 7.62, "C2929935", "RT: fsw = 1 / (12.4k x 284p + 450n) = 252 kHz (eq 1)")
two("C19", C, "820p C0G", C0402, "RAMP", "GND", lx, uy - 7.62 + P, "C161159", "CRAMP = IOS L / (VOUT A RS) (eq 37)")
two("R8", R, "232k", R0402, "VCC", "RAMP", lx, uy - 7.62 + 2 * P, "C3013159", "RRAMP to VCC: IOS = 25u + 38u = 63 uA (eq 39)")
two("C20", C, "47n", C0402, "SS", "GND", lx, uy - 7.62 + 3 * P, "C272875", "soft start 5.7 ms to 1.215 V (eq 23)")
two("R9", R, "36.5k 0.1%", R0402, "VOUT", "FB", lx, uy - 7.62 + 4 * P, "C852741", "RFB2: VOUT = 1.215 (1 + 36.5 / 2.49) = 19.03 V")
two("R10", R, "2.49k 0.1%", R0402, "FB", "GND", lx, uy - 7.62 + 5 * P, "C852700", "RFB1")
two("R11", R, "61.9k", R0402, "COMP", "CMP", lx, uy - 7.62 + 6 * P, "C2933106", "RCOMP (type II, fc ~19 kHz)")
two("C21", C, "1.5n", C0402, "CMP", "FB", lx, uy - 7.62 + 7 * P, "C107022", "CCOMP: zero 1.7 kHz")
two("C22", C, "22p", C0402, "COMP", "FB", lx, uy - 7.62 + 8 * P, "C1555", "CHF: pole 117 kHz")
# right column: supply and drive parts
rx = ux + 45.72
two("C17", C, "1u", C0402, "VCC", "GND", rx, uy - 7.62, "C52923", "VCC bypass at pins 16 / 14")
two("D1", D, "BAS21", SOD323, "HB", "VCC", rx, uy - 7.62 + P, "C82466", "bootstrap diode, 250 V")
two("C16", C, "1u", C0402, "HB", "SW", rx, uy - 7.62 + 2 * P, "C52923", "bootstrap cap HB-SW")
two("R3", R, "0R", R0402, "HO", "HG", rx, uy - 7.62 + 3 * P, "C17168", "HO series (bring-up: up to 2.2R to slow Q1 turn-on)")
two("R12", R, "10k", R0402, "DEMB", "CSG", rx, uy - 7.62 + 4 * P, "C25744", "RDEMB: full synchronous after soft start")
two("C18", C, "1u", C0402, "VCCX", "GND", rx, uy - 7.62 + 5 * P, "C52923", "VCCX bypass")
note("CS / CSG: Kelvin from R4's two pads through NT2 / NT1", ux - 25.4, uy + 25.4)

# ---- power stage
px, py = 170.18, 165.1
heading("POWER STAGE (single phase, 10 uH, RS 4 mOhm in the low-side source)", px - 50.8, py - 20.32)
part("Q1", "Transistor_FET:BSC098N10NS5", "BSC098N10NS5", FET, {"5": "VIN", "4": "HG", "1": "SW", "2": "SW", "3": "SW"},
     (px - 38.1, py), lcsc="C534398", desc="high side, 100 V 9.8 mOhm, Qsw 7.8 nC")
part("Q2", "Transistor_FET:BSC070N10NS5", "BSC070N10NS5", FET, {"5": "SW", "4": "LO", "1": "LSS", "2": "LSS", "3": "LSS"},
     (px - 7.62, py + 15.24), lcsc="C534364", desc="low side, 100 V 7.0 mOhm")
part("R4", R, "4m 2512", R2512, {"1": "LSS", "2": "GND"}, (px + 15.24, py + 15.24), lcsc="C459681",
     desc="RS: ILIM = 0.110 / RS - VOUT T / L = 20 A typ (eq 37); 3 W 1 % 2512")
part("NT1", "Device:NetTie_2", "NetTie", NETTIE, {"1": "GND", "2": "CSG"}, (px + 30.48, py + 15.24),
     desc="CSG Kelvin tap at R4's GND pad")
part("NT2", "Device:NetTie_2", "NetTie", NETTIE, {"1": "LSS", "2": "CS"}, (px + 30.48, py + 25.4),
     desc="CS Kelvin tap at R4's top (LSS) pad")
part("L1", L, "10u APS1510M100F", LXAL, {"1": "SW", "2": "VOUT"}, (px + 30.48, py - 7.62), rot=90, lcsc="C49261402",
     desc="10 uH, 8.6 mOhm, Isat 26.5 A (40 % drop), 16.5 x 15.5 x 10 mm, F side; second source FEXL1510A-100M C5378597")

# ---- output
ox, oy = 297.18, 40.64
heading("OUTPUT 19.0 V", ox - 15.24, oy - 10.16)
for i in range(4):
    two("C%d" % (9 + i), C, "22u 35V", C1210, "VOUT", "GND", ox, oy + P * i, "C778721", "output MLCC X5R 1210")
for i in range(3):
    two("C%d" % (13 + i), CP, "100u 35V polymer", CP63, "VOUT", "GND", ox, oy + P * (4 + i) + 2.54, "C46550467",
        "MA35V100M6X8 solid polymer, 30 mOhm, 6.3 x 7.7 mm, F side")

# ---- VCCX from VOUT
vx, vy = 297.18, 116.84
heading("VCCX = 9.3 V FROM VOUT (VCCX max 16 V; VOUT 19 V too high)", vx - 25.4, vy - 12.7)
part("Q3", "Transistor_BJT:BCX56", "BCX56-16", SOT89, {"1": "VB", "2": "VOUT", "3": "VCCX"}, (vx, vy),
     lcsc="C123402", desc="emitter follower, ~0.2 W")
two("R6", R, "10k", R0402, "VOUT", "VB", vx + 22.86, vy - 5.08, "C25744", "zener feed")
two("D2", DZ, "BZT52C10", SOD123, "VB", "GND", vx + 22.86, vy + 5.08, "C2103", "10 V reference for VCCX")

# ---- UVLO / RUN / OVLO
ux2, uy2 = 297.18, 152.4
heading("UVLO 38.5 V / RUN / HICCUP / OVLO 66.6 V", ux2 - 25.4, uy2 - 12.7)
two("R13", R, "200k 1206", R1206, "VIN", "RUN", ux2 - 15.24, uy2, "C17954", "RUV2 > 500 x VIN(max)")
two("R14", R, "6.34k", R0402, "RUN", "GND", ux2 - 15.24, uy2 + P, "C5713274", "RUV1: on 38.5 V, off ~35.3 V")
two("C23", C, "1u", C0402, "RUN", "GND", ux2 - 15.24, uy2 + 2 * P, "C52923", "CFT: hiccup off-time ~11 ms at 48 V")
two("D3", D, "BAS21", SOD323, "VIN", "RUN", ux2 - 15.24, uy2 + 3 * P, "C82466", "discharges CFT on an input dip")
two("R15", R, "1M 1206", R1206, "VIN", "OVS", ux2 - 15.24, uy2 + 4 * P + 2.54, "C17927", "OVLO divider top")
two("R16", R, "6.04k", R0402, "OVS", "GND", ux2 - 15.24, uy2 + 5 * P + 2.54, "C2998182", "OVLO: 0.400 x 1006.04 / 6.04 = 66.6 V")
two("C24", C, "1n", C0402, "OVS", "GND", ux2 - 15.24, uy2 + 6 * P + 2.54, "C1523", "OVLO filter 6 us")
part("U4", "pmb:TPS3701", "TPS3701DDCR", SOT236, {"3": "VAUX", "4": "OVS", "1": "NC", "6": "RUN", "5": "VAUX", "2": "GND"},
     (ux2 + 25.4, uy2 + 15.24), lcsc="C132788", desc="OVLO: OUTB pulls RUN low above 66.6 V; INA unused (held high)")
two("C25", C, "100n", C0402, "VAUX", "GND", ux2 + 17.78, uy2 + 35.56, "C307331", "U4 bypass")

# ---- output supervision (as rev A)
sx, sy = 40.64, 228.6
heading("OUTPUT SUPERVISION (as PM-19V-250W rev A): OV_KILL > 22.68 V, PGOOD > 17.26 V", sx - 20.32, sy - 15.24)
two("R17", R, "47k 0805", R0805, "VIN", "VAUX", sx - 7.62, sy, "C17713", "VAUX feed from VIN")
two("D4", DZ, "4.7V", SOD323, "VAUX", "GND", sx - 7.62, sy + P, "C107713", "VAUX 4.7 V zener")
two("C26", C, "1u", C0402, "VAUX", "GND", sx - 7.62, sy + 2 * P, "C52923", "VAUX")
part("U2", "Comparator:MCP65R46", "MCP65R46-2.4", SOT236, {"1": "OV_KILL", "2": "GND", "3": "OV_REF", "4": "OVD",
     "5": "OV_REF", "6": "VAUX"}, (sx + 50.8, sy + 2.54), lcsc="C623510", desc="OV_KILL: low when VOUT > 22.68 V")
two("R18", R, "84.5k", R0402, "VOUT", "OVD", sx + 27.94, sy + 20.32, "C185390", "OV divider: 2.4 x 94.5 / 10 = 22.68 V")
two("R19", R, "10k", R0402, "OVD", "GND", sx + 27.94, sy + 20.32 + P, "C25744", "")
two("C27", C, "100n", C0402, "VAUX", "GND", sx + 27.94, sy + 20.32 + 2 * P, "C307331", "U2 bypass")
part("U3", "Comparator:MCP65R46", "MCP65R46-2.4", SOT236, {"1": "PGOOD", "2": "GND", "3": "PGD", "4": "PG_REF",
     "5": "PG_REF", "6": "VAUX"}, (sx + 109.22, sy + 2.54), lcsc="C623510", desc="PGOOD: high when VOUT > 17.26 V")
two("R20", R, "61.9k", R0402, "VOUT", "PGD", sx + 86.36, sy + 20.32, "C2933106", "PG divider: 2.4 x 71.9 / 10 = 17.26 V")
two("R21", R, "10k", R0402, "PGD", "GND", sx + 86.36, sy + 20.32 + P, "C25744", "")
two("C28", C, "100n", C0402, "VAUX", "GND", sx + 86.36, sy + 20.32 + 2 * P, "C307331", "U3 bypass")

# ---- power flags
fx, fy = 297.18, 222.25
heading("POWER FLAGS", fx - 15.24, fy - 7.62)
for i, net in enumerate(["GND", "VIN_IC", "VAUX"]):
    xx = fx + 25.4 * i
    b.wire(net, [(xx, fy), (xx + 5.08, fy)])
    if net == "GND":
        b.flag(net, (xx + 5.08, fy))          # the GND symbol sits at the first point
    else:
        b.flag(net, (xx, fy))
        b.tie(net, (xx + 5.08, fy))


def write():
    b.finish()
    sh = K.Sheet("Root", PROJECT, key=PROJECT + "/root", paper="A3", root=True,
                 title="PM-19V-250W-B port module", date=DATE, rev="B0.1", company="Granite",
                 comments=["LM5116 single-phase buck 19.0 V 13.2 A, 38-66 V in, 252 kHz",
                           "interface as PM-19V-250W (docs/pm-interface.md); spec docs/pm-19v-250w-b.md",
                           "generated by tools/gensch.py (engine: power/pp-48/tools/sch)"])
    for net, a, c in b.segs: sh.wire(a, c, net)
    for p in b.junctions: sh.junction(*p)
    for xy in b.ncs: sh.nc(*xy)
    for net, xy, ang in b.labels: sh.label(net, xy[0], xy[1], ang)
    for txt, xx, yy, size in b.texts: sh.text(txt, xx, yy, size)
    for ref in b.order:
        sh.symbol(b.syms[ref])
    for s in b.psyms:
        sh.symbol(s)
    txt = sh.text_out()
    # J1 is screw pads only, NT1 / NT2 net ties: not in the BOM or the placement file
    for ref in ("J1", "NT1", "NT2"):
        i = txt.find('(property "Reference" "%s"' % ref)
        j = txt.rfind("\t(symbol\n", 0, i)
        head = txt[j:i].replace("(in_bom yes)", "(in_bom no)").replace("(in_pos_files yes)", "(in_pos_files no)")
        txt = txt[:j] + head + txt[i:]
    with open(OUT, "w") as f:
        f.write(txt)
    # project file: root sheet uuid
    pro = os.path.join(PROJ_DIR, PROJECT + ".kicad_pro")
    d = json.load(open(pro))
    d["schematic"]["top_level_sheets"] = [{"filename": PROJECT + ".kicad_sch", "name": PROJECT, "uuid": sh.uuid}]
    d["sheets"] = [[sh.uuid, PROJECT]]
    with open(pro, "w") as f:
        json.dump(d, f, indent=2)
        f.write("\n")
    print("wrote", OUT, len(b.order), "symbols")


if __name__ == "__main__":
    write()
