"""Supplies sheet: the plane's 3V3 (LM5164 buck) from VIN, ahead of the tile hot-swap.

The LM5164 drawing is power/pp-48-2s bucks.py's (datasheet layout, type 3
ripple injection), fed from VIN (the bus at the input connector, ahead of
the tile hot-swap) so the core stays up and reports when the hot-swap
trips (ADR 0003). Load: ~20 mA per column (INA238, TCA9535, ADC,
photoMOS / opto LEDs, pull-ups) plus the MCU, ~0.2 A at N = 8.
The isolated 12 V aux for the controller (ADR 0003 decision 14, own
Fly-Buck / flyback, ~5 W) goes on this sheet too: not drawn yet, its
coupled inductor / transformer is not chosen (LCSC stock).
"""
from sch.draw import Block, R0402, R0603, R0805, C0402, C0603, C1210, LED0603

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
    return b.finish()
