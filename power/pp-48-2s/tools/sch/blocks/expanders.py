"""Expanders sheet: one MCP23017 per two slots, addresses 0x20 + e.

Parts and pin -> net tables copied from gensch.py "# expanders" (the loop
over e < (N + 1) // 2): GPA = odd slot, GPB = even slot, bits 0-5 =
PWRD RSTD PGD PG LQ LED, bits 6-7 and the even bank of a missing slot NC.
The chips are stacked; SCL2 / SDA2 / EXP_IRQ run down a bus on the left
(SCL innermost), the slot signals leave on the right.
"""
from sch.draw import Block, C0402

G = 1.27
KINDS = ["PWRD", "RSTD", "PGD", "PG", "LQ", "LED", "NC", "NC"]


def gp(s, bank, k):
    return "S%d_%s" % (s, KINDS[k])


def exp_nets(e, N):
    sa, sb = 2 * e + 1, 2 * e + 2
    nets = {"9": "+3V3", "10": "GND", "11": "NC", "14": "NC", "12": "SCL2", "13": "SDA2",
            "15": "+3V3" if e & 1 else "GND", "16": "+3V3" if e & 2 else "GND", "17": "GND",
            "18": "+3V3", "19": "NC", "20": "EXP_IRQ"}
    for k in range(8):
        nets[str(21 + k)] = gp(sa, "A", k) if k < 6 else "NC"            # GPA0..7 = pins 21..28
        nets[str(1 + k)] = (gp(sb, "B", k) if (k < 6 and sb <= N) else "NC")  # GPB0..7 = pins 1..8
    return nets


def block(N=2):
    b = Block("Expanders")
    X, Y0 = 120 * G, 60 * G
    pitch = 62 * G
    E = (N + 1) // 2
    xb = {"SCL2": X - 18 * G, "SDA2": X - 20 * G, "EXP_IRQ": X - 22 * G}   # bus x, innermost first
    pins = {"SCL2": "12", "SDA2": "13", "EXP_IRQ": "20"}
    xport = X - 28 * G
    xr = X + 24 * G
    for e in range(E):
        sa, sb = 2 * e + 1, 2 * e + 2
        ref, cref = "U%d" % (6 + e), "C%d" % (28 + e)
        y = Y0 + e * pitch
        nets = exp_nets(e, N)
        u = b.sym(ref, "Interface_Expansion:MCP23017x-x-SO", "MCP23017-E/SO", "Package_SO:SOIC-28W_7.5x17.9mm_P1.27mm", nets,
                  at=(X, y), fields={"LCSC": "C47023"},
                  desc="slot GPIO for slots %d-%d, address 0x%02X, INTA mirrored open-drain, never reset by the controller" % (sa, sb, 0x20 + e))
        # VDD with its decoupling, on a short rail above the chip
        rail = y - 31 * G
        b.sym(cref, "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=(X + 6 * G, rail + 5 * G), desc=ref + " VDD")
        b.wire("+3V3", [(X - 4 * G, rail), (X, rail), (ref, "9"), (X, rail), (cref, "1")])
        # bus taps
        for net, num in pins.items():
            py = u.pin_xy(num)[1]
            b.wire(net, [(xb[net], py), (ref, num)])
            if e == 0:
                b.wire(net, [(xport, py), (xb[net], py)])
                b.port(net, "output" if net == "EXP_IRQ" else "bidirectional", side="left", pos=(xport, py))
            else:
                b.wire(net, [(xb[net], u.pin_xy(num)[1] - pitch), (xb[net], py)])
        # slot signals
        for num in [str(n) for n in list(range(21, 29)) + list(range(1, 9))]:
            net = nets[num]
            if net != "NC":
                py = u.pin_xy(num)[1]
                b.wire(net, [(ref, num), (xr, py)])
                b.port(net, "output" if net.endswith(("PWRD", "RSTD")) else "input", side="right", pos=(xr, py))
    return b.finish()
