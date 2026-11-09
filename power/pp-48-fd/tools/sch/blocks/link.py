"""Link sheet: the plane link over isolated CAN FD (docs/adr/0006-plane-link-can-fd.md).

The core sits in one row of boards on the mounting bar; the link is one
isolated net set along the row (LINK_GND, CAN_P, CAN_N, AUX_12V), shared with
the controller, never tied to the plane's GND.

- U4 ISO1044BDR (C2878225): isolated CAN FD transceiver. Side 1 on the
  plane (+3V3 / GND, TXD = CAN_TX, RXD = CAN_RX to the MCU sheet: the
  MCP2518FD in the G070 build, FDCAN1 PB8 / PB9 in the G0B1 build), side 2
  on the link domain. TXD has an internal pull-up (recessive while nothing
  drives it). Decoupling per the datasheet (section 10): 100 nF at VCC1,
  100 nF + 4.7 uF at VCC2.
- U7 HT7550-1 (SOT-89, 100 mA, 24 V max in): LINK_5V for the ISO1044 side 2
  (ADR decision 5: ICC2 70 mA max dominant, ~30 mA average at 1 Mbit/s),
  fed from this core's own isolated 12 V (ISO_12V, the LM5018 Fly-Buck
  secondary on the Supplies sheet). Dissipation (12 - 5) V x 70 mA = 0.5 W
  peak, ~0.2 W average: the SOT-89 tab wants copper.
- D10 B5819W: OR diode ISO_12V -> AUX_12V (decision 4); every core feeds
  the row's AUX_12V, the controller takes its 12 V from it.
- J2 LINK IN (left edge, 2x3 right-angle plug A2541WR-2x3P) and J3 LINK OUT
  (right edge, 2x3 right-angle socket A2541HWR-2x3P), pin n = pin n:
  1 CAN_P, 2 CAN_N, 3 / 4 LINK_GND, 5 / 6 AUX_12V (decision 3).
- D8 / D9 NUP2105L: 2-line CAN TVS (24 V working) on CAN_P / CAN_N at each
  connector.
- Right-end split termination (decision 8): R25 / R26 2 x 60.4R, C32 4.7 nF
  from the mid point to LINK_GND, behind the 2x2 2.54 mm pin header JP1
  (C492419): 1 CAN_P / 2 TERM_H and 3 CAN_N / 4 TERM_L, one jumper cap per
  leg (1-2, 3-4). No caps by default; both caps fitted on the last board of
  the row (two standard 2.54 mm caps, e.g. C5305, not assembled, not in the
  board BOM). Two legs so that an open termination leaves no capacitive
  load on one line only.

Coordinates are in grid units of 1.27 mm (P(x, y)).
"""
from sch.draw import Block, C0402, C0603

G = 1.27
R0603 = "Resistor_SMD:R_0603_1608Metric"
SYM = "Connector_Generic:Conn_02x03_Odd_Even"
PLUG = "Connector_PinHeader_2.54mm:PinHeader_2x03_P2.54mm_Horizontal"
SOCKET = "Connector_PinSocket_2.54mm:PinSocket_2x03_P2.54mm_Horizontal"
PLUG_FIELDS = {"LCSC": "C239334", "MPN": "CJT A2541WR-2x3P"}
SOCKET_FIELDS = {"LCSC": "C5333425", "MPN": "CJT A2541HWR-2x3P"}
TERM_SYM = "Connector_Generic:Conn_02x02_Odd_Even"
TERM_FP = "Connector_PinHeader_2.54mm:PinHeader_2x02_P2.54mm_Vertical"
TERM_HDR = {"LCSC": "C492419", "MPN": "XFCN PZ254V-12-4P"}
TERM_CAP = "C5305"                                # 2.54 mm jumper cap (accessory: 2 per row, last board)
PINS = {"1": "CAN_P", "2": "CAN_N", "3": "LINK_GND", "4": "LINK_GND", "5": "AUX_12V", "6": "AUX_12V"}
ESD = ("Power_Protection:NUP2105L", "NUP2105L", "Package_TO_SOT_SMD:SOT-23",
       {"LCSC": "C14486", "MPN": "onsemi NUP2105LT1G"})
TERM_R = {"LCSC": "C144645", "MPN": "Yageo AC0603FR-0760R4L"}
TERM_C = {"LCSC": "C1538", "MPN": "FH 0402B472K500NT"}


def P(x, y, d=None):
    """grid point; d = the direction a wire leaves it (a power symbol there faces away)"""
    return (x * G, y * G) if d is None else (x * G, y * G, d)


def connectors(b, j_in, j_out, x, y_in, y_out, nets=PINS):
    """link plug (IN) and socket (OUT), pin n = pin n; CAN_P / CAN_N tied by name"""
    b.sym(j_in, SYM, "LINK IN", PLUG, dict(nets), at=P(x, y_in), fields=PLUG_FIELDS, ref_at=(1.27, -7.62),
          desc="plane link from the left neighbour (left edge, 2x3 right-angle plug); pin n = pin n: "
               "1 CAN_P 2 CAN_N 3/4 LINK_GND 5/6 AUX_12V")
    b.sym(j_out, SYM, "LINK OUT", SOCKET, dict(nets), at=P(x, y_out), fields=SOCKET_FIELDS, ref_at=(1.27, -7.62),
          desc="plane link to the right neighbour (right edge, 2x3 right-angle socket), same pinout")
    for j in (j_in, j_out):
        b.tie("CAN_P", (j, "1"), 180)
        b.tie("CAN_N", (j, "2"), 0)


def termination(b, refs, x, y, lgnd="LINK_GND"):
    """split termination behind a 2x2 jumper-cap header; refs = (JP header, R CAN_P leg, R CAN_N leg, C), header at
    (x, y): 1 CAN_P / 2 TERM_H on row y, 3 CAN_N / 4 TERM_L on row y + 2; a cap on 1-2 and one on 3-4 close it"""
    j, ra, rb, c = refs
    b.sym(j, TERM_SYM, "TERM", TERM_FP, {"1": "CAN_P", "2": "TERM_H", "3": "CAN_N", "4": "TERM_L"}, at=P(x, y),
          fields=TERM_HDR, ref_at=(1.27, -5.08),
          desc="right-end termination, 2x2 2.54 mm header: jumper caps on 1-2 (CAN_P) and 3-4 (CAN_N), both on the "
               "last board of the row only, none elsewhere (caps %s, not assembled)" % TERM_CAP)
    xr = x + 6                                       # header pins 2 / 4
    b.sym(ra, "Device:R", "60.4", R0603, {"1": "TERM_H", "2": "TERM_MID"}, at=P(xr + 13, y), rot=90, fields=TERM_R,
          ref_at=(-2.54, -2.54), val_at=(2.54, -2.54), desc="split termination 2 x 60.4R 0603 (dominant ~2 V: ~17 mW each)")
    b.sym(rb, "Device:R", "60.4", R0603, {"1": "TERM_MID", "2": "TERM_L"}, at=P(xr + 13, y + 4), rot=270, fields=TERM_R,
          ref_at=(-2.54, 2.54), val_at=(2.54, 2.54))           # the two legs' texts apart
    b.sym(c, "Device:C", "4.7n", C0402, {"1": "TERM_MID", "2": lgnd}, at=P(xr + 30, y + 2), rot=90, fields=TERM_C,
          desc="split termination common-mode cap to LINK_GND")
    b.wire("TERM_H", [(j, "2"), (ra, "1")])
    b.wire("TERM_L", [(j, "4"), P(xr + 1, y + 2), P(xr + 1, y + 4), (rb, "2")])
    b.wire("TERM_MID", [(ra, "2"), P(xr + 18, y), P(xr + 18, y + 4), (rb, "1")])
    b.wire("TERM_MID", [P(xr + 18, y + 2), (c, "1")])
    b.tie("CAN_P", (j, "1"), 180)
    b.tie("CAN_N", (j, "3"), 180)


def block(N=2):
    b = Block("Link", date="2026-11-04")
    for net, sym in (("LINK_GND", "power:GND"), ("LINK_5V", "power:+5V"), ("ISO_12V", "power:+12V"),
                     ("AUX_12V", "power:+12V")):
        b.power(net, sym, value=net)
    X, Y = 70, 50                                    # U4 origin (grid units)

    b.sym("U4", "Interface_CAN_LIN:ISO1044BD", "ISO1044BDR", "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
          {"1": "+3V3", "2": "CAN_TX", "3": "CAN_RX", "4": "GND", "5": "CAN_N", "6": "CAN_P", "7": "LINK_GND", "8": "LINK_5V"},
          at=P(X, Y), fields={"LCSC": "C2878225", "MPN": "TI ISO1044BDR"}, ref_at=(0, -8.89),
          desc="isolated CAN FD transceiver (5 Mbit/s); side 1 = plane (TXD / RXD to the MCP2518FD or the G0B1 FDCAN1), "
               "side 2 = the link domain (LINK_GND, LINK_5V)")

    # ---------------------------------------------------------- supply taps with their decoupling
    def vcc(u, pin, caps, net, side):
        px, py = b.syms[u].pin_xy(pin)
        d = 1 if side == "right" else -1
        rail = py - 12 * G
        path = []
        for k, (cap, val, fp, gnd, desc) in enumerate(caps):
            xc = X * G + d * (20 + 10 * k) * G
            b.sym(cap, "Device:C", val, fp, {"1": net, "2": gnd}, at=(xc, rail + 5 * G), desc=desc)
            path.append((cap, "1"))
        xend = X * G + d * (24 + 10 * (len(caps) - 1)) * G
        b.wire(net, [(xend, rail)] + path[::-1] + [(X * G + d * 12 * G, rail), (u, pin)])

    vcc("U4", "1", [("C24", "100n", C0402, "GND", "U4 VCC1")], "+3V3", "left")
    vcc("U4", "8", [("C25", "100n", C0402, "LINK_GND", "U4 VCC2, at the pin"),
                    ("C26", "4.7u", C0603, "LINK_GND", "U4 VCC2 bulk (ISO1044 datasheet section 10: typ. 4.7 uF)")],
        "LINK_5V", "right")
    b.port("CAN_TX", "input", side="left", pos=("U4", "2"))
    b.port("CAN_RX", "output", side="left", pos=("U4", "3"))
    b.tie("CAN_P", ("U4", "6"), 0)
    b.tie("CAN_N", ("U4", "5"), 0)

    # ---------------------------------------------------------- LINK_5V regulator and the AUX_12V OR diode
    yl = Y + 34
    b.sym("U7", "Regulator_Linear:HT75xx-1-SOT89", "HT7550-1", "Package_TO_SOT_SMD:SOT-89-3",
          {"1": "LINK_GND", "2": "ISO_12V", "3": "LINK_5V"}, at=P(X, yl), fields={"LCSC": "C16106", "MPN": "Holtek HT7550-1"}, ref_at=(0, -8.89),
          desc="LINK_5V for U4 side 2 (100 mA, 24 V max in), from this core's isolated 12 V (ADR 0006 decision 5)")
    b.sym("C27", "Device:C", "1u 25V", C0603, {"1": "ISO_12V", "2": "LINK_GND"}, at=P(X - 14, yl + 3), desc="U7 input")
    b.wire("ISO_12V", [P(X - 20, yl - 2), ("C27", "1"), ("U7", "2")])
    b.sym("D10", "Device:D_Schottky", "B5819W", "Diode_SMD:D_SOD-123", {"1": "AUX_12V", "2": "ISO_12V"},
          at=P(X, yl + 18), mirror="y", fields={"LCSC": "C8598", "MPN": "SL B5819W"},
          desc="OR diode: this core's isolated 12 V onto the row's AUX_12V (ADR 0006 decision 4); 40 V 1 A")
    b.text("LINK_5V  U7 from ISO_12V;  AUX_12V OR  D10", P(X - 22, yl - 9))

    # ---------------------------------------------------------- connectors, ESD, termination
    XJ = 120
    connectors(b, "J2", "J3", XJ, Y, Y + 16)
    for d, x in (("D8", XJ + 22), ("D9", XJ + 40)):
        b.sym(d, ESD[0], ESD[1], ESD[2], {"1": "CAN_P", "2": "CAN_N", "3": "LINK_GND"}, at=P(x, Y + 6), fields=ESD[3],
              desc="CAN ESD / TVS (24 V working) on CAN_P / CAN_N at %s" % ("J2 LINK IN" if d == "D8" else "J3 LINK OUT"))
        b.tie("CAN_P", (d, "1"), 90)
        b.tie("CAN_N", (d, "2"), 90)
    termination(b, ("JP1", "R25", "R26", "C32"), XJ - 10, Y + 34)

    for ref, mpn in (("C24", "Samsung CL05B104KO5NNNC"), ("C25", "Samsung CL05B104KO5NNNC"),
                     ("C26", "Samsung CL10A475KO8NNNC"), ("C27", "Samsung CL10A105KB8NNNC")):
        b.syms[ref].fields["MPN"] = mpn
    for k in range(-16, 20, 3):                       # isolation barrier through U4
        y = Y + k
        if not (Y - 8 < y < Y + 8):
            b.text("|", P(X, y), size=1.5)
    b.text("PLANE (+3V3 / GND)", P(X - 30, Y - 20))
    b.text("LINK DOMAIN (LINK_GND)", P(X + 4, Y - 20))
    b.text("LINK  J2 IN plug / J3 OUT socket: 1 CAN_P 2 CAN_N 3/4 LINK_GND 5/6 AUX_12V", P(XJ - 10, Y - 24))
    b.text("TERMINATION  JP1: jumper caps on 1-2 and 3-4 on the last board of the row only", P(XJ - 10, Y + 28))
    return b.finish()
