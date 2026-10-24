"""Input sheet: bus terminal, main fuse, TVS, bulk caps, VBUS sense divider.

Parts and pin -> net tables copied from gensch.py "# input". VBUS_IN is
local (J1.1 -> F1.1, PWR_FLAG on the wire); VBUS and GND are power
symbols; VBUS_SNS leaves the sheet on the right.
"""
from sch.draw import Block, R0402, R0805, C0402, C1210

G = 1.27

def block(N=2):
    b = Block("Input")
    b.power("VBUS", "power:VBUS")
    y = 48 * G                      # VBUS_IN / VBUS rail
    yc = y + 6 * G                  # centre of the parts hanging from the rail
    xs = 96 * G                   # x of the sense divider

    # ---------------------------------------------------------- terminal and fuse
    b.sym("J1", "Connector:Screw_Terminal_01x02", "BUS 40-60V",
          "TerminalBlock_Phoenix:TerminalBlock_Phoenix_MKDS-3-2-5.08_1x02_P5.08mm_Horizontal",
          {"1": "VBUS_IN", "2": "GND"}, at=(24 * G, y), mirror="y",
          desc="bus input, 24 A terminal (2-slot plane); lugs on the 8-slot plane")
    b.sym("F1", "Device:Fuse", "10A nano2", "Fuse:Fuse_Littelfuse-NANO2-451_453", {"1": "VBUS_IN", "2": "VBUS"},
          at=(46 * G, y), rot=90, fields={"LCSC": "C44479"}, desc="0451010.MRL main fuse, 2-slot plane")
    xf = 34 * G                     # PWR_FLAG on VBUS_IN
    b.wire("VBUS_IN", [("J1", "1"), (xf, y), ("F1", "1")])
    b.flag("VBUS_IN", (xf, y))
    b.name("VBUS_IN", at=(xf + G, y))   # b.name is shadowed by the sheet name attribute
    b.wire("GND", [(31 * G, yc + 3 * G), ("J1", "2")])

    # ---------------------------------------------------------- TVS and bulk caps
    xd, xc1, xc2 = 58 * G, 70 * G, 82 * G
    b.sym("D1", "Device:D_TVS", "SMBJ60A", "Diode_SMD:D_SMB", {"1": "GND", "2": "VBUS"}, at=(xd, yc), rot=90,
          fields={"LCSC": "C353345"}, desc="bus TVS, 60 V standoff, clamps below the 100 V parts")
    b.sym("C1", "Device:C", "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, at=(xc1, yc),
          fields={"LCSC": "C338133"}, desc="X7R")
    b.sym("C2", "Device:C", "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, at=(xc2, yc),
          fields={"LCSC": "C338133"}, desc="X7R")

    # ---------------------------------------------------------- sense divider
    b.sym("R1", "Device:R", "470k", R0805, {"1": "VBUS", "2": "VBUS_SNS"}, at=(xs, yc),
          desc="bus sense divider top, 150 V rated size")
    b.sym("R2", "Device:R", "13k", R0402, {"1": "VBUS_SNS", "2": "GND"}, at=(xs, yc + 10 * G),
          desc="60 V -> 1.61 V at the ADC")
    b.sym("C3", "Device:C", "10n", C0402, {"1": "VBUS_SNS", "2": "GND"}, at=(xs + 10 * G, yc + 10 * G))

    # VBUS: one rail from the fuse, one power symbol on it
    xp = 52 * G
    path = [(xp, y), ("F1", "2"), (xp, y)]
    for x, ref in ((xd, "D1"), (xc1, "C1"), (xc2, "C2")):
        pin = "2" if ref == "D1" else "1"
        path += [(x, y), (ref, pin), (x, y)]
    path += [(xs, y), ("R1", "1")]
    b.wire("VBUS", path)

    ysn = yc + 5 * G                # sense node, between R1 and R2
    xport = xs + 20 * G
    b.wire("VBUS_SNS", [("R1", "2"), ("R2", "1")])
    b.wire("VBUS_SNS", [(xs, ysn), (xport, ysn)])
    b.wire("VBUS_SNS", [(xs + 10 * G, ysn), ("C3", "1")])
    b.port("VBUS_SNS", "output", side="right", pos=(xport, ysn))

    b.text("BUS INPUT 40-60 V", (22 * G, y - 10 * G))
    return b.finish()
