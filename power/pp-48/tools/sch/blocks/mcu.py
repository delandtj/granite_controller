"""MCU sheet: STM32G070 with decoupling, HSE crystal, NRST, SWD, run LED,
plane address straps, I2C2 pull-ups.

Copied from power/pp-48-2s (same pins for the crystal, SWD, straps, LED and
the two I2C ports) and cut down for ADR 0004: the columns are reached
through the TCA9548A (Mux sheet), so the per-slot OFF / LCLR / ID / TEMP
pins are gone. I2C1 (PB6 / PB7) is the slave link to the controller,
I2C2 (PB10 / PB11) the master for the mux and the plane front end (tile
INA238). GPIO: TILE_OFF (tile hot-swap off), TILE_PGD, TILE_AL (tile
INA238 ALERT), MUX_RST; TILE2_* for the end cap's second group (N = 6 / 8,
NC otherwise). ADC: VIN_SNS (bus ahead of the tile hot-swap).
block(tile=False) is the chained core (docs/adr/0005): no TILE_* pins, the
unit hot-swaps are read and reset over each unit's expander.
"""
from sch.draw import Block, R0402, C0402, C0603, LED0603, SJ

G = 1.27

RIGHT_PORTS = {"SCL2": "bidirectional", "SDA2": "bidirectional",
               "INT_OUT": "output", "SCL1": "bidirectional", "SDA1": "bidirectional"}


def mcu_nets(N, tile=True):
    # pins 29 and 32 are not bonded on the G070 (UCPD pins on the G071/G0B1)
    nets = {
        "1": "ADR0", "2": "ADR1", "3": "ADR2", "4": "+3V3", "5": "+3V3", "6": "+3V3", "7": "GND",
        "8": "OSC_IN", "9": "OSC_OUT", "10": "NRST",
        "11": "NC", "12": "NC", "13": "NC", "14": "NC", "15": "NC", "16": "NC", "17": "NC", "18": "NC",
        "19": "VIN_SNS", "20": "NC", "21": "NC",
        "22": "SCL2", "23": "SDA2", "24": "MUX_RST", "25": "TILE_AL", "26": "NC", "27": "NC",
        "28": "INT_OUT", "29": "NC", "32": "NC", "33": "NC", "34": "NC",
        "30": "TILE_OFF", "31": "TILE_PGD", "38": "TILE2_OFF", "39": "TILE2_PGD", "40": "NC", "41": "NC", "42": "NC", "43": "NC",
        "35": "SWDIO", "36": "SWCLK", "37": "NC", "44": "NC", "45": "SCL1", "46": "SDA1", "47": "LED_RUN", "48": "NC",
    }
    if N <= 4 or not tile:           # one hot-swap group, in the core
        nets["38"] = nets["39"] = "NC"
    if not tile:                     # chained plane (ADR 0005): the hot-swaps are in the units, on their expanders
        nets["25"] = "NC"
        nets["30"] = "PRESS_TRIG"    # PC6 TIM3_CH1: edges retrigger the press deadline (docs/press-deadline-line.md)
        nets["31"] = "EXP_RSTN"      # PC7 open drain: the units' TCA6424A /RESET (R30 pull-ups on the units)
    return nets


SHAPES = {"VIN_SNS": "input", "TILE_PGD": "input", "TILE2_PGD": "input", "TILE_AL": "input",
          "TILE_OFF": "output", "TILE2_OFF": "output", "MUX_RST": "output",
          "PRESS_TRIG": "output", "EXP_RSTN": "output"}


def shape(net):
    if net in RIGHT_PORTS: return RIGHT_PORTS[net]
    return SHAPES[net]


def block(N=2, tile=True):
    b = Block("MCU")
    X, Y = 160 * G, 130 * G            # U3 origin
    nets = mcu_nets(N, tile)
    u = b.sym("U3", "MCU_ST_STM32G0:STM32G070CBTx", "STM32G070CBT6", "Package_QFP:LQFP-48_7x7mm_P0.5mm", nets,
              at=(X, Y), fields={"LCSC": "C529338"},
              desc="plane MCU: I2C1 slave to the controller (PB6/PB7), I2C2 master to the mux and front end (PB10/PB11), SWD on PA13/PA14")
    pin = lambda n: u.pin_xy(n)
    XR, XL = X + 22 * G, X - 22 * G    # right / left pin ends

    # ---------------------------------------------------------- +3V3 rail and decoupling (top)
    yr = Y - 50 * G
    xc20, xc18, xc19 = X - 19 * G, X - 10 * G, X + 6 * G
    b.sym("C20", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=(xc20, yr + 5 * G), desc="U3 VREF+/VBAT")
    b.sym("C18", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=(xc18, yr + 5 * G), desc="U3 VDD")
    b.sym("C19", "Device:C", "4.7u", C0603, {"1": "+3V3", "2": "GND"}, at=(xc19, yr + 5 * G), desc="U3 VDD bulk")
    xvr = XL - 2 * G                   # VREF+ drop
    b.wire("+3V3", [(xvr - 2 * G, yr), (xvr, yr), ("U3", "5"), (xvr, yr), ("C20", "1"), ("C18", "1"),
                    ("U3", "4"), (X, yr), ("U3", "6"), ("C19", "1")])

    # ---------------------------------------------------------- HSE crystal (left, above the OSC pins)
    yc = Y - 42 * G
    xy = X - 40 * G
    b.sym("Y1", "Device:Crystal_GND24", "8MHz", "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm",
          {"1": "OSC_IN", "2": "GND", "3": "OSC_OUT", "4": "GND"}, at=(xy, yc), mirror="y",
          fields={"LCSC": "C7424755"}, desc="HSE, energy time base; 3225 20 pF 10 ppm")
    b.sym("C22", "Device:C", "33p", C0402, {"1": "OSC_IN", "2": "GND"}, at=(xy + 8 * G, yc + 5 * G), desc="8 MHz crystal CL 20 pF")
    b.sym("C23", "Device:C", "33p", C0402, {"1": "OSC_OUT", "2": "GND"}, at=(xy - 8 * G, yc + 5 * G))
    b.wire("GND", [("Y1", "2"), ("Y1", "4")])
    xv = XL - 4 * G                    # OSC_IN riser
    b.wire("OSC_IN", [("Y1", "1"), (xy + 8 * G, yc), ("C22", "1")])
    b.wire("OSC_IN", [("U3", "8"), (xv, pin("8")[1]), (xv, yc), (xy + 8 * G, yc)])
    xw = xy - 12 * G                   # OSC_OUT riser, outside the crystal
    b.wire("OSC_OUT", [("Y1", "3"), (xy - 8 * G, yc), ("C23", "1")])
    b.wire("OSC_OUT", [("U3", "9"), (xw, pin("9")[1]), (xw, yc), (xy - 8 * G, yc)])

    # ---------------------------------------------------------- SWD header J5 (top left) and NRST
    xj, yj = X - 43 * G, Y - 60 * G
    b.sym("J5", "Connector:Conn_ARM_SWD_TagConnect_TC2030-NL", "SWD", "Connector:Tag-Connect_TC2030-IDC-NL_2x03_P1.27mm_Vertical",
          {"1": "+3V3", "2": "SWDIO", "3": "NRST", "4": "SWCLK", "5": "GND", "6": "NC"}, at=(xj, yj), rot=90,
          desc="bench programming")
    j = b.syms["J5"]
    b.wire("+3V3", [(j.pin_xy("1")[0] - 2 * G, j.pin_xy("1")[1] - 2 * G), ("J5", "1")])
    b.wire("GND", [(j.pin_xy("5")[0] + 2 * G, j.pin_xy("5")[1] + 2 * G), ("J5", "5")])
    b.sym("C21", "Device:C", "100n", C0402, {"1": "NRST", "2": "GND"}, at=(X - 40 * G, pin("10")[1] + 5 * G), desc="NRST")
    xn = xw - 4 * G
    yjt = j.pin_xy("3")[1]
    b.wire("NRST", [("U3", "10"), (xn, pin("10")[1]), (xn, yjt - 6 * G), (j.pin_xy("3")[0], yjt - 6 * G), ("J5", "3")])
    b.wire("NRST", [(X - 40 * G, pin("10")[1]), ("C21", "1")])

    # ---------------------------------------------------------- right-hand pins
    right = sorted([(u.pin_xy(p["num"])[1], p["num"]) for p in u.pins if u.pin_out(p["num"]) == (1, 0)])
    top, bot = [], []
    for y, num in right:
        net = nets[num]
        if net == "NC" or net in RIGHT_PORTS or net == "LED_RUN": continue
        (top if y < Y + 12 * G else bot).append((y, num, net))

    # fan-out over the top: innermost = topmost pin; labels face left above U3
    x = XR
    row = Y - 40 * G
    yint = pin("28")[1]
    yswd = j.pin_xy("2")[1] - 2 * G        # SWD rows clear J5's pin tops
    jx = {"SWDIO": j.pin_xy("2")[0], "SWCLK": j.pin_xy("4")[0]}
    for k, (y, num, net) in enumerate(top):
        x += 2 * G
        if y > yint: x = max(x, XR + 10 * G)          # clear the INT_OUT label
        if y > Y - G: x = max(x, XR + 14 * G)         # clear U3's value text
        row -= 2 * G
        if net in jx:
            row = min(row, yswd)
            yswd = row - 2 * G
            b.wire(net, [("U3", num), (x, y), (x, row), (jx[net], row), ("J5", "2" if net == "SWDIO" else "4")])
        else:
            b.wire(net, [("U3", num), (x, y), (x, row), (x - 2 * G, row)])
            b.port(net, shape(net), side="left", pos=(x - 2 * G, row))
    xmax = x

    # fan-out under the bottom: innermost = lowest pin
    x = XR
    row = Y + 39 * G
    for y, num, net in sorted(bot, reverse=True):
        x += 2 * G
        row += 2 * G
        b.wire(net, [("U3", num), (x, y), (x, row), (x - 2 * G, row)])
        b.port(net, shape(net), side="left", pos=(x - 2 * G, row))

    # left-hand signal pins: labels at the pins
    for p in u.pins:
        num, net = p["num"], nets[p["num"]]
        if u.pin_out(num) == (-1, 0) and net in SHAPES:
            b.port(net, shape(net), side="left", pos=("U3", num))

    # ---------------------------------------------------------- run LED (right)
    yl = pin("47")[1]
    b.sym("R19", "Device:R", "1k", R0402, {"1": "LED_RUN", "2": "LEDRUN_A"}, at=(XR + 11 * G, yl), rot=90,
          desc="firmware heartbeat LED")
    b.sym("D4", "Device:LED", "GREEN RUN", LED0603, {"1": "GND", "2": "LEDRUN_A"}, at=(XR + 27 * G, yl), rot=180,
          fields={"LCSC": "C2297"})
    b.wire("GND", [(XR + 33 * G, yl), ("D4", "1")])     # GND clear of the LED arrows
    xled = XR + 33 * G

    # ---------------------------------------------------------- bus ports and pull-ups (right)
    for net in ("INT_OUT", "SCL1", "SDA1"):
        b.port(net, shape(net), side="right", pos=("U3", {"INT_OUT": "28", "SCL1": "45", "SDA1": "46"}[net]))
    xp = max(xmax, xled) + 5 * G
    for ref, val, net, num, desc in (("R24", "2.2k", "SCL2", "22", ""),
                                     ("R23", "2.2k", "SDA2", "23", "I2C2 pull-up (mux upstream, tile INA238)")):
        y = pin(num)[1]
        b.sym(ref, "Device:R", val, R0402, {"1": "+3V3", "2": net}, at=(xp, y - 6 * G), desc=desc)
        b.wire(net, [("U3", num), (xp + (4 + len(net)) * G, y)])
        b.wire(net, [(xp, y), (ref, "2")])
        b.port(net, shape(net), side="right", pos=(xp + (4 + len(net)) * G, y))
        xp += (14 + len(net)) * G

    # ---------------------------------------------------------- address straps (below left)
    for i, num in enumerate(("1", "2", "3")):
        net = nets[num]
        xa = XL - (4 + 10 * (2 - i)) * G
        ya = pin(num)[1]
        b.sym("R%d" % (20 + i), "Device:R", "100k", R0402, {"1": "+3V3", "2": net}, at=(xa - 6 * G, Y + 6 * G),
              desc="address strap pull-up")
        b.sym("JP%d" % (1 + i), "Jumper:SolderJumper_2_Open", "ADR%d" % i, SJ, {"1": net, "2": "GND"},
              at=(xa, Y + 16 * G), rot=270, desc="close = 0; plane address 0x30 + ADR[2:0]")
        b.wire(net, [("U3", num), (xa, ya), ("JP%d" % (1 + i), "1")])
        b.wire(net, [("R%d" % (20 + i), "2"), (xa - 6 * G, Y + 11 * G), (xa, Y + 11 * G)])

    return b.finish()
