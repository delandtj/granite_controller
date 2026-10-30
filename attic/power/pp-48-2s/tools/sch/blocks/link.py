"""Link sheet: isolated controller link, ISO1640 (I2C) + ISO7721 (EXP_INT).

Parts and pin -> net tables copied from gensch.py "# controller link".
Plane side (+3V3 / GND, I2C1 pull-ups, ports) on the left, controller
side (CTRL_3V3 / CTRL_GND from the cable) on the right; the isolators are
drawn mirrored so that side 2 (plane) faces left. U5 sits above U4 so
that INT, SDA, SCL arrive at J2 top to bottom (J2 / J3 drawn flipped:
pin 5 on top) without crossing. J3 is the daisy-chain twin of J2.
"""
from sch.draw import Block, R0402, C0402

G = 1.27
JST = "Connector_JST:JST_PH_B5B-PH-K_1x05_P2.00mm_Vertical"


def block(N=2):
    b = Block("Link")
    b.power("CTRL_3V3", "power:+3V3", value="CTRL_3V3")
    b.power("CTRL_GND", "power:GND", value="CTRL_GND")
    X = 150 * G
    Y5 = 60 * G
    Y4 = Y5 + 29 * G

    b.sym("U5", "Isolator:ISO7721D", "ISO7721D", "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
          {"1": "CTRL_3V3", "2": "ISO_OUTA", "3": "CTRL_GND", "4": "CTRL_GND", "5": "GND", "6": "NC", "7": "INT_OUT", "8": "+3V3"},
          at=(X, Y5), mirror="y", fields={"LCSC": "C366164"},
          desc="EXP_INT plane -> controller (channel A, side 2 in, side 1 out); channel B unused")
    b.sym("U4", "Isolator:ISO1540", "ISO1640BD", "Package_SO:SOIC-8_3.9x4.9mm_P1.27mm",
          {"1": "CTRL_3V3", "2": "CTRL_SDA", "3": "CTRL_SCL", "4": "CTRL_GND", "5": "GND", "6": "SCL1", "7": "SDA1", "8": "+3V3"},
          at=(X, Y4), mirror="y", fields={"LCSC": "C5122339"},
          desc="isolated I2C; side 1 = controller cable, side 2 = plane. ISO1540 symbol, ISO1640 pin compatible")

    # ---------------------------------------------------------- supply taps with their decoupling
    def vcc(u, pin, cap, net, gnd, side, desc):
        s = b.syms[u]
        px, py = s.pin_xy(pin)
        d = 1 if side == "right" else -1
        rail = py - 12 * G
        xc = X + d * 20 * G
        b.sym(cap, "Device:C", "100n", C0402, {"1": net, "2": gnd}, at=(xc, rail + 5 * G), desc=desc)
        b.wire(net, [(xc + d * 4 * G, rail), (cap, "1"), (X + d * 12 * G, rail), (u, pin)])

    vcc("U5", "8", "C27", "+3V3", "GND", "left", "U5 VCC2")
    vcc("U5", "1", "C26", "CTRL_3V3", "CTRL_GND", "right", "U5 VCC1")
    vcc("U4", "8", "C25", "+3V3", "GND", "left", "U4 VCC2")
    vcc("U4", "1", "C24", "CTRL_3V3", "CTRL_GND", "right", "U4 VCC1")
    for u, pin in (("U5", "5"), ("U4", "5")):
        px, py = b.syms[u].pin_xy(pin)
        b.wire("GND", [(px - 2 * G, py + 2 * G), (u, pin)])
    px, py = b.syms["U4"].pin_xy("4")
    b.wire("CTRL_GND", [(px + 2 * G, py + 2 * G), ("U4", "4")])
    px, py = b.syms["U5"].pin_xy("4")
    b.wire("CTRL_GND", [(px + 2 * G, py + 2 * G), ("U5", "4"), ("U5", "3")])

    # ---------------------------------------------------------- plane side: ports and I2C1 pull-ups
    XL = X - 8 * G
    ysda, yscl = b.syms["U4"].pin_xy("7")[1], b.syms["U4"].pin_xy("6")[1]
    xa, xb = X - 30 * G, X - 44 * G
    b.sym("R26", "Device:R", "4.7k", R0402, {"1": "+3V3", "2": "SDA1"}, at=(xa, ysda - 6 * G), desc="I2C1 pull-up, isolator side 2")
    b.sym("R27", "Device:R", "4.7k", R0402, {"1": "+3V3", "2": "SCL1"}, at=(xb, ysda - 6 * G))
    b.wire("SDA1", [("U4", "7"), (xa - 4 * G, ysda)])
    b.wire("SDA1", [(xa, ysda), ("R26", "2")])
    b.port("SDA1", "bidirectional", side="left", pos=(xa - 4 * G, ysda))
    b.wire("SCL1", [("U4", "6"), (xb - 4 * G, yscl)])
    b.wire("SCL1", [(xb, yscl), ("R27", "2")])
    b.port("SCL1", "bidirectional", side="left", pos=(xb - 4 * G, yscl))
    yint = b.syms["U5"].pin_xy("7")[1]
    b.wire("INT_OUT", [("U5", "7"), (xb - 4 * G, yint)])
    b.port("INT_OUT", "input", side="left", pos=(xb - 4 * G, yint))

    # ---------------------------------------------------------- controller side: INT driver
    yo = b.syms["U5"].pin_xy("2")[1]
    xr30, xq = X + 18 * G, X + 30 * G
    b.sym("R30", "Device:R", "100k", R0402, {"1": "ISO_OUTA", "2": "CTRL_GND"}, at=(xr30, yo + 5 * G), desc="gate pull-down")
    b.sym("Q1", "Transistor_FET:2N7002", "2N7002", "Package_TO_SOT_SMD:SOT-23", {"1": "ISO_OUTA", "2": "CTRL_GND", "3": "CTRL_INT"},
          at=(xq, yo), fields={"LCSC": "C8545"},
          desc="open-drain EXP_INT on the controller side (MCU drives INT_OUT high to assert)")
    b.wire("ISO_OUTA", [("U5", "2"), ("Q1", "1")])
    b.wire("ISO_OUTA", [(xr30, yo), ("R30", "1")])

    # ---------------------------------------------------------- connectors (flipped: pin 5 on top)
    xj = X + 66 * G
    y2 = Y4 + 2 * G                      # J2 pin 4 on the SDA row
    y3 = y2 + 16 * G
    for j, y in (("J2", y2), ("J3", y3)):
        b.sym(j, "Connector_Generic:Conn_01x05", "CTRL " + ("IN" if j == "J2" else "OUT"), JST,
              {"1": "CTRL_3V3", "2": "CTRL_GND", "3": "CTRL_SCL", "4": "CTRL_SDA", "5": "CTRL_INT"}, at=(xj, y), mirror="x",
              fields={"LCSC": "C157993"}, desc="controller link, daisy chain; 3V3 GND SCL SDA INT, from J8 of the controller")
    J2, J3 = b.syms["J2"], b.syms["J3"]
    xi = X + 44 * G                       # INT riser from Q1
    qd = b.syms["Q1"].pin_xy("3")
    b.wire("CTRL_INT", [("Q1", "3"), (qd[0], qd[1] - 2 * G), (xi, qd[1] - 2 * G), (xi, J2.pin_xy("5")[1]), ("J2", "5")])
    # DNP controller-side pull-ups
    xr28, xr29 = X + 29 * G, X + 37 * G
    b.sym("R28", "Device:R", "4.7k", R0402, {"1": "CTRL_3V3", "2": "CTRL_SDA"}, at=(xr28, ysda - 7 * G), dnp=True,
          desc="DNP: the controller's J8 bus carries the pull-ups")
    b.sym("R29", "Device:R", "4.7k", R0402, {"1": "CTRL_3V3", "2": "CTRL_SCL"}, at=(xr29, ysda - 7 * G), dnp=True)
    b.wire("CTRL_SDA", [("U4", "2"), ("J2", "4")])
    b.wire("CTRL_SDA", [(xr28, ysda), ("R28", "2")])
    b.wire("CTRL_SCL", [("U4", "3"), ("J2", "3")])
    b.wire("CTRL_SCL", [(xr29, yscl), ("R29", "2")])
    # daisy chain J2 -> J3: INT innermost (closest to the connectors)
    for net, pin, x in (("CTRL_INT", "5", xj - 18 * G), ("CTRL_SDA", "4", xj - 20 * G), ("CTRL_SCL", "3", xj - 22 * G)):
        b.wire(net, [(x, J2.pin_xy(pin)[1]), (x, J3.pin_xy(pin)[1]), ("J3", pin)])

    # ---------------------------------------------------------- isolation barrier
    for k in range(-16, 49, 3):
        y = Y5 + k * G
        if not (Y5 - 9 * G < y < Y5 + 9 * G or Y4 - 9 * G < y < Y4 + 11 * G):
            b.text("|", (X, y), size=1.5)
    b.text("PLANE", (X - 12 * G, Y5 - 21 * G))
    b.text("CONTROLLER (cable)", (X + 4 * G, Y5 - 21 * G))
    return b.finish()
