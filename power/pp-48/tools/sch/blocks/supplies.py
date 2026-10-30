"""Supplies sheet: the plane's 3V3 (LM5164 buck) from VIN, ahead of the tile hot-swap.

The LM5164 drawing is power/pp-48-2s bucks.py's (datasheet layout, type 3
ripple injection), fed from VIN (the bus at the input connector, ahead of
the tile hot-swap) so the core stays up and reports when the hot-swap
trips (ADR 0003). Load: ~20 mA per column (INA238, TCA9535, ADC,
photoMOS / opto LEDs, pull-ups) plus the MCU, ~0.2 A at N = 8.
Isolated 12 V aux for the controller (ADR 0003 decision 14): U2 LM5017
Fly-Buck from VIN (datasheet 8.2.2, K = 9e-11): 1:1 coupled inductor L2
Bourns SRF0703-470M (N1 = 1-3 dot 1 primary, N2 = 2-4 dot 2 secondary,
hi-pot 500 Vrms), primary VOUT1 12.7 V (R14 / R17), ~500 kHz (R12 280k
from VIN), type 3 ripple (R13 / C14 / C15, required for a Fly-Buck),
UVLO on 30.3 V / 2 V hysteresis (R10 / R11), D2 VOUT1 -> VCC. Secondary:
D6 100 V Schottky, CTRL_12V ~12.0 V at 170 mA (~2 W, sized so one OR-ed
feed carries the controller's radio peaks), C30 / C31, D7 13 V TVS and
the LED (~2 mA) as the preload. CTRL_12V / CTRL_GND to J4 are the
controller's world, never tied to GND: functional isolation, >= 1 mm
gap on every layer around the island.
"""
from sch.draw import Block, R0402, R0603, R0805, C0402, C0603, C0805, C1210, LED0603

G = 1.27

def lm5164(b, gx, gy, u, vout, nets, rron, l, rfb1, cout, ca, rsuffix, cbase, lcsc_l=""):
    """one LM5164 buck, symbol origin at (gx, gy) in 1.27 mm grid units;
    returns P() and the x (grid units) where the output rail ends"""
    def P(x, y): return ((gx + x) * G, (gy + y) * G)
    r, c = rsuffix, cbase
    rr = lambda k: "R%d" % (r + k)
    cc = lambda k: "C%d" % (c + k)
    lref = "L%d" % (1 if u == "U1" else 2)
    vo, sw, fb, inj, en, bst = nets["vout"], nets["sw"], nets["fb"], nets["inj"], nets["en"], nets["bst"]

    b.sym(u, "Regulator_Switching:LM5164DDA", "LM5164DDA", "Package_SO:HSOP-8-1EP_3.9x4.9mm_P1.27mm_EP2.41x3.1mm",
          {"1": "GND", "2": "VIN", "3": nets["en"], "4": nets["ron"], "5": nets["fb"], "6": "NC", "7": nets["bst"], "8": nets["sw"], "9": "GND"},
          at=P(0, 0), fields={"LCSC": "C477928"}, desc="100 V 1 A COT buck, %s" % vout)

    # ---------------------------------------------------------- input side (left)
    yr = -22                                       # VBUS rail above the chip
    xen, xcb, xv, xch, xron = -30, -24, -12, -8, -16
    b.sym(rr(0), "Device:R", "1M", R0805, {"1": "VIN", "2": nets["en"]}, at=P(xen, -7), desc="EN/UVLO top (VIN, ahead of the tile hot-swap)")
    b.sym(rr(1), "Device:R", "52.3k", R0402, {"1": nets["en"], "2": "GND"}, at=P(xen, 3), desc="on at 30 V, off at 28 V")
    b.sym(rr(2), "Device:R", rron, R0402, {"1": nets["ron"], "2": "GND"}, at=P(xron, 7), desc="FSW = VOUT x 2500 / RRON")
    b.sym(cc(3), "Device:C", "2.2u 100V", C1210, {"1": "VIN", "2": "GND"}, at=P(xcb, -16),
          fields={"LCSC": "C338133"}, desc="input, at the VIN pin")
    b.sym(cc(4), "Device:C", "100n 100V", C0603, {"1": "VIN", "2": "GND"}, at=P(xch, -16), desc="input HF")
    b.wire("VIN", [P(xen + 3, yr), P(xen, yr), (rr(0), "1"), P(xen, yr), P(xcb, yr), (cc(3), "1"), P(xcb, yr),
                    P(xv, yr), (u, "2"), P(xv, yr), P(xch, yr), (cc(4), "1")])
    b.wire(en, [(rr(0), "2"), (u, "3")])
    b.wire(en, [(rr(0), "2"), (rr(1), "1")])
    b.wire(nets["ron"], [(u, "4"), (rr(2), "1")])

    # ---------------------------------------------------------- switch node (right)
    xbst, xra, xl = 15, 17, 22
    b.sym(cc(0), "Device:C", "2.2n 50V", C0402, {"1": nets["bst"], "2": nets["sw"]}, at=P(xbst, -8), desc="bootstrap")
    b.sym(lref, "Device:L", l, "Inductor_SMD:L_12x12mm_H6mm", {"1": nets["sw"], "2": nets["vout"]}, at=P(xl, -2), rot=90,
          fields={"LCSC": lcsc_l, "MPN": "Bourns SRR1260-330M" if l == "33u" else "Sumida CDRH127/LDNP-680MC"},
          desc="shielded, Isat >= 1.5 A, 12x12 land pattern (SRR1260 / CDRH127 class)")
    b.wire(bst, [(u, "7"), P(12, -6), P(12, -13), P(xbst, -13), (cc(0), "1")])
    b.wire(sw, [(u, "8"), P(xbst, -2), (lref, "1")])
    b.wire(sw, [P(xbst, -2), (cc(0), "2")])

    # ---------------------------------------------------------- ripple injection and feedback
    # type 3 as the datasheet draws it (SNVSAU4D Figure 7-1): RA and CA in series across
    # the inductor (SW -> INJ -> VOUT), CB from the INJ node down to FB
    xinj, xca, xd = 29, 33, 40
    b.sym(rr(5), "Device:R", "453k", R0402, {"1": nets["sw"], "2": nets["inj"]}, at=P(21, 4), rot=90,
          desc="ripple injection RA (type 3)")
    b.sym(cc(1), "Device:C", ca, C0402, {"1": nets["inj"], "2": nets["vout"]}, at=P(xca, 4), rot=90,
          desc="ripple injection CA")
    b.sym(cc(2), "Device:C", "100n", C0402, {"1": nets["inj"], "2": nets["fb"]}, at=P(xinj, 11),
          desc="ripple injection CB (DC block)")
    b.sym(rr(3), "Device:R", rfb1, R0402, {"1": nets["vout"], "2": nets["fb"]}, at=P(xd, 9), desc="feedback top")
    b.sym(rr(4), "Device:R", "49.9k", R0402, {"1": nets["fb"], "2": "GND"}, at=P(xd, 20), desc="feedback bottom, VFB 1.2 V")
    b.wire(sw, [P(xra, -2), (rr(5), "1")])
    b.wire(inj, [(rr(5), "2"), P(xinj, 4), (cc(2), "1")])
    b.wire(inj, [P(xinj, 4), (cc(1), "1")])
    yfb = 15
    b.wire(fb, [(u, "5"), P(12, yfb), P(xd, yfb), (rr(3), "2")])
    b.wire(fb, [P(xinj, yfb), (cc(2), "2")])
    b.wire(fb, [P(xd, yfb), (rr(4), "1")])

    # ---------------------------------------------------------- output
    xo1, xo2 = 52, 63
    b.sym(cc(5), "Device:C", cout, C1210, {"1": nets["vout"], "2": "GND"}, at=P(xo1, 4), desc="output X7R")
    b.sym(cc(6), "Device:C", cout, C1210, {"1": nets["vout"], "2": "GND"}, at=P(xo2, 4), desc="output X7R")
    rail = [P(29, -2), (lref, "2"), P(29, -2), P(37, -2), P(37, 4), (cc(1), "2"), P(37, 4), P(37, -2)]   # CA to VOUT
    for x, e in ((xd, (rr(3), "1")), (xo1, (cc(5), "1")), (xo2, (cc(6), "1"))):
        rail += [P(x, -2), e, P(x, -2)]
    return P, rail

def flybuck(b, gx, gy):
    """the isolated 12 V aux, LM5017 U2 at (gx, gy) in 1.27 mm grid units"""
    def P(x, y): return ((gx + x) * G, (gy + y) * G)
    b.sym("U2", "Regulator_Switching:LM5017MR", "LM5017MRX/NOPB",
          "Package_SO:SOIC-8-1EP_3.9x4.9mm_P1.27mm_EP2.95x4.9mm_Mask2.71x3.4mm_ThermalVias",
          {"1": "GND", "2": "VIN", "3": "UVLO2", "4": "RON2", "5": "FB2", "6": "VCC2", "7": "BST2", "8": "SW2", "9": "GND"},
          at=P(0, 0), fields={"LCSC": "C34355"}, desc="100 V 600 mA COT synchronous buck, Fly-Buck primary 12.7 V")

    # ---------------------------------------------------------- input side (left)
    yr = -22
    b.sym("R10", "Device:R", "100k", R0805, {"1": "VIN", "2": "UVLO2"}, at=P(-30, -7), desc="UVLO top, 20 uA x 100k = 2 V hysteresis")
    b.sym("R11", "Device:R", "4.22k", R0402, {"1": "UVLO2", "2": "GND"}, at=P(-30, 3), desc="on at 30.3 V, off at 28.3 V")
    b.sym("R12", "Device:R", "280k", R0805, {"1": "VIN", "2": "RON2"}, at=P(-16, -12), desc="RON from VIN: fsw = VOUT1 / (9e-11 x RON) ~ 500 kHz")
    b.sym("C11", "Device:C", "2.2u 100V", C1210, {"1": "VIN", "2": "GND"}, at=P(-24, -16),
          fields={"LCSC": "C338133"}, desc="input, at the VIN pin")
    b.sym("C12", "Device:C", "100n 100V", C0603, {"1": "VIN", "2": "GND"}, at=P(-5, -16), desc="input HF")
    b.wire("VIN", [P(-33, yr), P(-30, yr), ("R10", "1"), P(-30, yr), P(-24, yr), ("C11", "1"), P(-24, yr), P(-16, yr),
                   ("R12", "1"), P(-16, yr), P(-12, yr), ("U2", "2"), P(-12, yr), P(-5, yr), ("C12", "1")])
    b.wire("UVLO2", [("R10", "2"), P(-30, -2), P(-20, -2), P(-20, 2), ("U2", "3")])
    b.wire("UVLO2", [P(-30, -2), ("R11", "1")])
    b.wire("RON2", [("R12", "2"), ("U2", "4")])
    b.wire("GND", [P(-1, 14), P(-1, 12), P(-2, 12), ("U2", "1"), P(-2, 12), P(0, 12), ("U2", "9")])

    # ---------------------------------------------------------- switch node, coupled inductor
    b.sym("C13", "Device:C", "10n 50V", C0402, {"1": "BST2", "2": "SW2"}, at=P(14, -8), desc="bootstrap")
    b.sym("L2", "Device:L_Coupled_1324", "47u", "pp48:L_Bourns_SRF0703",
          {"1": "SW2", "3": "VOUT1", "2": "CTRL_GND", "4": "SEC2"}, at=P(36, -8), mirror="x",
          fields={"LCSC": "C3273458", "MPN": "Bourns SRF0703-470M"},
          desc="Fly-Buck coupled inductor 1:1, 47 uH, Isat 1.14 A, hi-pot 500 Vrms; N1 1-3 primary, N2 2-4 secondary (dots 1, 2)")
    b.wire("BST2", [("U2", "7"), P(11, -4), P(11, -14), P(14, -14), ("C13", "1")])
    b.name("BST2", at=P(11, -10), ang=90)
    b.wire("SW2", [("U2", "8"), P(14, -2), P(22, -2), P(22, -6), ("L2", "1")])
    b.wire("SW2", [P(14, -2), ("C13", "2")])
    b.wire("CTRL_GND", [P(28, -10), ("L2", "2")])
    b.name("SW2", at=P(18, -2))

    # ---------------------------------------------------------- type 3 ripple, feedback, VCC
    b.sym("R13", "Device:R", "200k", R0805, {"1": "SW2", "2": "INJ2"}, at=P(26, 4), rot=90, desc="ripple Rr (type 3)")
    b.sym("C14", "Device:C", "1n", C0402, {"1": "INJ2", "2": "VOUT1"}, at=P(38, 4), rot=90, desc="ripple Cr")
    b.sym("C15", "Device:C", "100n", C0402, {"1": "INJ2", "2": "FB2"}, at=P(34, 11), desc="ripple Cac (DC block)")
    b.sym("R14", "Device:R", "18.7k", R0402, {"1": "VOUT1", "2": "FB2"}, at=P(48, 9), desc="feedback top")
    b.sym("R17", "Device:R", "2k", R0402, {"1": "FB2", "2": "GND"}, at=P(48, 20), desc="feedback bottom: VOUT1 = 1.225 V x 10.35 = 12.7 V")
    b.sym("C16", "Device:C", "1u 25V", C0603, {"1": "VCC2", "2": "GND"}, at=P(10, 10), desc="VCC")
    b.sym("D2", "Device:D", "1N4148W", "Diode_SMD:D_SOD-123", {"1": "VCC2", "2": "VOUT1"}, at=P(13, 22), rot=270,
          fields={"LCSC": "C917030"}, desc="VCC from VOUT1 (12.7 V > 8.55 V + VF): bias off the VIN regulator")
    b.wire("SW2", [P(22, -2), ("R13", "1")])
    b.wire("INJ2", [("R13", "2"), P(34, 4), ("C15", "1")])
    b.wire("INJ2", [P(34, 4), ("C14", "1")])
    b.wire("FB2", [("U2", "5"), P(16, 2), P(16, 15), P(48, 15), ("R14", "2")])
    b.wire("FB2", [P(34, 15), ("C15", "2")])
    b.wire("FB2", [P(48, 15), ("R17", "1")])
    b.wire("VCC2", [("U2", "6"), P(13, 6), ("D2", "1")])
    b.wire("VCC2", [P(10, 6), ("C16", "1")])

    # ---------------------------------------------------------- primary output VOUT1
    b.sym("C29", "Device:C", "4.7u 25V", C0805, {"1": "VOUT1", "2": "GND"}, at=P(56, 2), desc="primary output COUT1")
    b.wire("VOUT1", [("L2", "3"), P(42, -6), P(42, 4), ("C14", "2"), P(42, 4), P(42, -6), P(48, -6), ("R14", "1"),
                     P(48, -6), P(56, -6), ("C29", "1"), P(56, -6), P(60, -6), P(60, 27), P(13, 27), ("D2", "2")])

    # ---------------------------------------------------------- secondary: the controller's world (CTRL_GND island)
    b.sym("D6", "Device:D_Schottky", "SS110", "Diode_SMD:D_SMA", {"1": "CTRL_12V", "2": "SEC2"}, at=P(47, -22), rot=180,
          fields={"LCSC": "C2482"}, desc="secondary rectifier 100 V 1 A (VR = VIN, 1:1)")
    b.sym("C30", "Device:C", "10u 25V", C0805, {"1": "CTRL_12V", "2": "CTRL_GND"}, at=P(56, -19), desc="secondary output COUT2")
    b.sym("C31", "Device:C", "10u 25V", C0805, {"1": "CTRL_12V", "2": "CTRL_GND"}, at=P(66, -19), desc="secondary output COUT2")
    b.sym("D7", "Device:D_Zener", "SMF13A", "Diode_SMD:D_SOD-123F", {"1": "CTRL_12V", "2": "CTRL_GND"}, at=P(76, -19), rot=270,
          fields={"LCSC": "C353315"}, desc="13 V TVS: caps VOUT2 when the controller is unplugged (the Fly-Buck secondary rises unloaded)")
    b.sym("R9", "Device:R", "4.7k", R0603, {"1": "CTRL_12V", "2": "LED12_A"}, at=P(86, -19), desc="12V LED, also the ~2 mA preload")
    b.sym("D5", "Device:LED", "GREEN 12V", LED0603, {"1": "CTRL_GND", "2": "LED12_A"}, at=P(86, -10), rot=90,
          fields={"LCSC": "C2297"})
    b.sym("J4", "Connector_Generic:Conn_01x02", "12V AUX OUT", "Connector_JST:JST_PH_B2B-PH-K_1x02_P2.00mm_Vertical",
          {"1": "CTRL_12V", "2": "CTRL_GND"}, at=P(102, -22), fields={"LCSC": "C131337"},
          desc="isolated 12 V to the controller (controller input to be adapted to 12 V)")
    b.wire("SEC2", [("L2", "4"), P(42, -10), P(42, -22), ("D6", "2")])
    b.name("SEC2", at=P(42, -14), ang=90)
    b.wire("CTRL_12V", [P(53, -25), P(53, -22), ("D6", "1"), P(53, -22), P(56, -22), ("C30", "1"), P(56, -22), P(66, -22),
                        ("C31", "1"), P(66, -22), P(76, -22), ("D7", "1"), P(76, -22), P(86, -22), ("R9", "1"), P(86, -22), ("J4", "1")])
    b.wire("CTRL_GND", [P(52, -13), P(52, -15), P(56, -15), ("C30", "2"), P(56, -15), P(66, -15), ("C31", "2"), P(66, -15),
                        P(76, -15), ("D7", "2")])
    b.wire("CTRL_GND", [P(96, -15), P(96, -20), ("J4", "2")])
    b.wire("LED12_A", [("R9", "2"), ("D5", "2")])
    b.wire("CTRL_GND", [P(86, -4), ("D5", "1")])
    b.name("LED12_A", at=P(86, -14))
    b.text("ISOLATED 12 V AUX  U2 LM5017 Fly-Buck from VIN, L2 1:1, secondary on the CTRL_GND island", P(-34, -30))


def block(N=2):
    b = Block("Supplies")
    b.power("VIN", "power:VBUS", value="VIN")
    G1 = (45, 44)

    # 3V3 buck U1: LM5164, 200 kHz, type 3 ripple injection
    P, rail = lm5164(b, *G1, "U1", "3.3 V", dict(en="EN1", ron="RON1", fb="FB1", bst="BST1", sw="SW1", inj="INJ1", vout="+3V3"),
                     "41.2k", "33u", "86.6k", "22u 10V", "1.5n", 3, 4, lcsc_l="C840528")
    b.sym("R18", "Device:R", "1k", R0402, {"1": "+3V3", "2": "LED33_A"}, at=P(75, 3), desc="3V3 LED")
    b.sym("D3", "Device:LED", "GREEN 3V3", LED0603, {"1": "GND", "2": "LED33_A"}, at=P(75, 12), rot=90, fields={"LCSC": "C2297"})
    b.wire("+3V3", rail + [P(75, -2), ("R18", "1")])
    b.wire("LED33_A", [("R18", "2"), ("D3", "2")])
    b.wire("GND", [P(75, 17), ("D3", "1")])          # GND below the LED arrows
    b.name("LED33_A", at=P(75, 8))
    b.text("3V3 BUCK  U1 LM5164, 200 kHz, from VIN", P(-34, -30))

    # isolated 12 V aux U2: LM5017 Fly-Buck (see the docstring)
    b.power("CTRL_12V", "power:+12V", value="CTRL_12V")
    b.power("CTRL_GND", "power:GND", value="CTRL_GND")
    flybuck(b, 45, 112)
    return b.finish()
