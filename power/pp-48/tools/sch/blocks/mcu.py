"""MCU sheet: STM32G070 with decoupling, HSE crystal, NRST, SWD, run LED,
plane address straps, I2C2 pull-ups.

Copied from power/pp-48-2s (same pins for the crystal, SWD, straps, LED and
the two I2C ports) and cut down for ADR 0004: the columns are reached
through the TCA9548A (Mux sheet), so the per-slot OFF / LCLR / ID / TEMP
pins are gone. block(tile=False) is the chained core (docs/adr/0005): no
TILE_* pins, the unit hot-swaps are read and reset over each unit's
expander; its pins (CORE_PINS) are picked for the fewest ratsnest crossings
on the core board: I2C1 (PB8 / PB9) the slave link to the controller, I2C2
(PA11 / PA12) the master for the mux, ADC VIN_SNS (PB11), GPIO PRESS_TRIG,
MUX_RST, LED_RUN, EXP_RSTN, INT_OUT and the address straps. The sheet
draws a port (sheet nets) or a label (straps, LED) at each signal pin and
the strap, LED and pull-up circuits beside U3; SWD and NRST are wired.
"""
from sch.draw import Block, R0402, C0402, C0603, LED0603

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
        # pins picked for the fewest ratsnest crossings on the core board (tools/place_core.py placement):
        # each function on the side of U3 that faces its parts
        for k in list(nets):
            if nets[k] not in ("+3V3", "GND", "OSC_IN", "OSC_OUT", "NRST", "SWDIO", "SWCLK"):
                nets[k] = "NC"
        nets.update(CORE_PINS)
    return nets


# chained core (block(tile=False)): pin -> net
CORE_PINS = {
    "47": "SCL1", "48": "SDA1",      # PB8 / PB9 I2C1 AF6: slave link, up to the ISO1640 U4
    "33": "SCL2", "34": "SDA2",      # PA11 / PA12 I2C2 AF6: master, mux U6 and the units
    "23": "VIN_SNS",                 # PB11 ADC_IN15
    "24": "PRESS_TRIG",              # PB12: edges retrigger the press deadline (docs/press-deadline-line.md)
    "25": "MUX_RST",                 # PB13
    "26": "LED_RUN",                 # PB14
    "38": "EXP_RSTN",                # PD0 open drain: the units' TCA6424A /RESET (R30 pull-ups on the units)
    "40": "INT_OUT",                 # PD2: EXP_INT to the controller through the ISO7721 U5
    "37": "ADR0", "39": "ADR1", "28": "ADR2",   # PA15 / PD1 / PA8 straps (DIP switch SW1 to GND, R20-22 up)
}

# what the firmware needs per pin (written to firmware/plane-core-pins.toml by gensch.py core):
# pin -> (port, mode, use); the fixed pins too
PIN_USE = {
    "47": ("PB8", "I2C1_SCL AF6", "I2C slave to the controller through the ISO1640 U4"),
    "48": ("PB9", "I2C1_SDA AF6", "I2C slave to the controller through the ISO1640 U4"),
    "33": ("PA11", "I2C2_SCL AF6", "I2C master: TCA9548A 0x70, then one unit segment per channel"),
    "34": ("PA12", "I2C2_SDA AF6", "I2C master: TCA9548A 0x70, then one unit segment per channel"),
    "23": ("PB11", "ADC_IN15", "core 48 V input, 220k / 10k divider"),
    "24": ("PB12", "output push-pull", "toggle to retrigger the press deadline U301 (~1 s); never toggle when no press runs"),
    "25": ("PB13", "output push-pull", "high = mux reset, all channels off (Q6 inverts; 100k pull-down)"),
    "26": ("PB14", "output push-pull", "heartbeat LED, high = on"),
    "38": ("PD0", "output open-drain", "low = reset every unit's TCA6424A (bused, pull-ups on the units)"),
    "40": ("PD2", "output push-pull", "high = assert EXP_INT to the controller (ISO7721 U5)"),
    "37": ("PA15", "input", "address strap ADR0: pulled up, DIP switch SW1 ON = 0; slave address 0x30 + ADR[2:0]"),
    "39": ("PD1", "input", "address strap ADR1"),
    "28": ("PA8", "input", "address strap ADR2"),
    "35": ("PA13", "SWDIO", "debug (fixed)"),
    "36": ("PA14", "SWCLK", "debug (fixed)"),
    "8": ("PF0", "OSC_IN", "8 MHz HSE crystal (fixed)"),
    "9": ("PF1", "OSC_OUT", "8 MHz HSE crystal (fixed)"),
    "10": ("NRST", "NRST", "reset, SWD header (fixed)"),
}


def pins_toml(nets):
    """the core's pin designations for the firmware, from the drawn pin map"""
    missing = [n for n, net in nets.items() if net not in ("NC", "+3V3", "GND") and n not in PIN_USE]
    wrong = [n for n in PIN_USE if nets.get(n, "NC") == "NC"]
    if missing or wrong:
        raise SystemExit("mcu PIN_USE out of step with the pin map: missing %s, not connected %s" % (missing, wrong))
    out = ["# STM32G070CBT6 (LQFP-48) pin designations of the PP-48 core board.",
           "# GENERATED by power/pp-48/tools/gensch.py core from tools/sch/blocks/mcu.py",
           "# (CORE_PINS / PIN_USE): do not edit here, change the generator and regenerate.",
           "# The schematic (attic/power/pp-48-core/sheets/mcu.kicad_sch) and the board follow the same table.",
           "", '[mcu]', 'part = "STM32G070CBT6"', 'package = "LQFP-48"', 'hse_hz = 8000000', ""]
    for num in sorted(PIN_USE, key=lambda n: (PIN_USE[n][0][:2], int(PIN_USE[n][0][2:]) if PIN_USE[n][0][2:].isdigit() else 0)):
        port, mode, use = PIN_USE[num]
        out += ['[[pin]]', 'port = "%s"' % port, 'pin = %s' % num, 'net = "%s"' % nets[num],
                'mode = "%s"' % mode, 'use = "%s"' % use, ""]
    unused = sorted((n for n, net in nets.items() if net == "NC"), key=int)
    out += ["# not connected (pins 29 / 32 are not bonded on the G070)", "[unused]", "pins = [%s]" % ", ".join(unused), ""]
    return "\n".join(out)


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
              at=(X, Y), fields={"LCSC": "C529338"}, ref_at=(5.08, -45.72, "left"), val_at=(8.89, -45.72, "left"),
              desc="plane MCU: I2C1 slave to the controller (PB8/PB9), I2C2 master to the mux and the units (PA11/PA12), SWD on PA13/PA14")
    pin = lambda n: u.pin_xy(n)
    XR, XL = X + 22 * G, X - 22 * G    # right / left pin ends

    # ---------------------------------------------------------- +3V3 rail and decoupling (top)
    yr = Y - 50 * G
    xc20, xc18, xc19 = X - 19 * G, X - 10 * G, X + 6 * G
    b.sym("C20", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=(xc20, yr + 5 * G), desc="U3 VREF+/VBAT")
    b.sym("C18", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=(xc18, yr + 5 * G), desc="U3 VDD")
    b.sym("C19", "Device:C", "4.7u", C0603, {"1": "+3V3", "2": "GND"}, at=(xc19, yr + 5 * G), desc="U3 VDD bulk")
    b.wire("+3V3", [(XL - 4 * G, yr), (xc20, yr), ("C20", "1"), (xc20, yr), (xc18, yr), ("C18", "1"), (xc18, yr),
                    ("U3", "4"), (X, yr), ("U3", "6"), ("C19", "1")])
    b.wire("+3V3", [(pin("5")[0] - 4 * G, pin("5")[1], (-1, 0)), ("U3", "5")])     # VREF+: its own symbol, clear of the OSC rows

    # ---------------------------------------------------------- HSE crystal (left, in line with the OSC pins)
    xy, yc = X - 39 * G, Y - 20 * G
    b.sym("Y1", "Device:Crystal_GND24", "8MHz", "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm",
          {"1": "OSC_IN", "2": "GND", "3": "OSC_OUT", "4": "GND"}, at=(xy, yc),
          fields={"LCSC": "C7424755"}, desc="HSE, energy time base; 3225 20 pF 10 ppm",
          ref_at=(-1.27, -5.08, "right"), val_at=(1.27, -5.08, "left"))
    b.sym("C22", "Device:C", "33p", C0402, {"1": "OSC_IN", "2": "GND"}, at=(xy - 6 * G, yc + 3 * G), desc="8 MHz crystal CL 20 pF",
          ref_at=(-2.54, -1.27, "right"), val_at=(-2.54, 1.27, "right"))
    b.sym("C23", "Device:C", "33p", C0402, {"1": "OSC_OUT", "2": "GND"}, at=(xy + 11 * G, yc + 3 * G),
          ref_at=(-2.54, -1.27, "right"), val_at=(-2.54, 1.27, "right"))

    b.wire("GND", [("Y1", "2"), ("Y1", "4")])
    yo = Y - 26 * G                    # OSC_IN over the crystal
    b.wire("OSC_IN", [("U3", "8"), (X - 26 * G, pin("8")[1]), (X - 26 * G, yo), (xy - 6 * G, yo), ("C22", "1"), ("Y1", "1")])
    b.name("OSC_IN", at=(X - 41 * G, yo))
    b.wire("OSC_OUT", [("U3", "9"), ("C23", "1"), ("Y1", "3")])
    b.name("OSC_OUT", at=(X - 35 * G, yc))
    xw = X - 52 * G                    # NRST riser column

    # ---------------------------------------------------------- SWD header J5 (top left) and NRST
    xj, yj = X - 43 * G, Y - 60 * G
    b.sym("J5", "Connector:Conn_ARM_SWD_TagConnect_TC2030-NL", "SWD", "Connector:Tag-Connect_TC2030-IDC-NL_2x03_P1.27mm_Vertical",
          {"1": "+3V3", "2": "SWDIO", "3": "NRST", "4": "SWCLK", "5": "GND", "6": "NC"}, at=(xj, yj), rot=90,
          desc="bench programming", ref_at=(0, 8.89), val_at=(0, 11.43))
    j = b.syms["J5"]
    b.wire("+3V3", [(j.pin_xy("1")[0] - 2 * G, j.pin_xy("1")[1] - 2 * G), ("J5", "1")])
    b.wire("GND", [(j.pin_xy("5")[0] + 2 * G, j.pin_xy("5")[1] + 2 * G), ("J5", "5")])
    # the same SWD on the ARM 10-pin Cortex header (1.27 mm, keyed shroud): stock probe ribbons
    b.sym("J6", "Connector_Generic:Conn_02x05_Odd_Even", "SWD", "Connector_PinHeader_1.27mm:PinHeader_2x05_P1.27mm_Vertical_SMD",
          {"1": "+3V3", "2": "SWDIO", "3": "GND", "4": "SWCLK", "5": "GND", "6": "NC", "7": "NC", "8": "NC", "9": "GND",
           "10": "NRST"}, at=(xj - 29 * G, yj + 4 * G),
          fields={"LCSC": "C5155080", "MPN": "Samtec FTSH-105-01-L-DV-K"}, desc="bench programming, ARM Cortex 10-pin",
          ref_at=(0, -11.43), val_at=(0, -8.89))

    j6 = b.syms["J6"]                               # GND: 3 / 5 joined, 9 from below (clear of 7)
    xg = j6.pin_xy("5")[0] - 2 * G
    b.wire("GND", [("J6", "3"), ("J6", "5")])
    b.wire("GND", [(xg, j6.pin_xy("9")[1] + 3 * G), (xg, j6.pin_xy("9")[1]), ("J6", "9")])
    for num, net in (("2", "SWDIO"), ("4", "SWCLK"), ("10", "NRST")):   # 6 SWO, 7 key, 8 TDI open
        b.tie(net, ("J6", num), 0)
    b.sym("C21", "Device:C", "100n", C0402, {"1": "NRST", "2": "GND"}, at=(xw, pin("10")[1] + 3 * G), desc="NRST")
    xn = xw - 4 * G
    yjt = j.pin_xy("3")[1]
    b.wire("NRST", [("U3", "10"), (xn, pin("10")[1]), (xn, yjt - 6 * G), (j.pin_xy("3")[0], yjt - 6 * G), ("J5", "3")])
    b.wire("NRST", [(xw, pin("10")[1]), ("C21", "1")])
    b.tie("NRST", (xn + G, yjt - 6 * G))

    # ---------------------------------------------------------- SWD fan-out (right-hand pins, over the top to J5)
    inv = {net: num for num, net in nets.items()}
    x = XR + 16 * G                                   # outside the pull-ups and ports at the right-hand pins
    yswd = j.pin_xy("2")[1] - 4 * G                   # SWD rows clear J5's pin tops
    jx = {"SWDIO": j.pin_xy("2")[0], "SWCLK": j.pin_xy("4")[0]}
    for net in ("SWDIO", "SWCLK"):                    # SWDIO is the lower pin: outer column, lower row
        num = inv[net]
        x += 2 * G
        b.wire(net, [("U3", num), (x, pin(num)[1]), (x, yswd), (jx[net], yswd), ("J5", "2" if net == "SWDIO" else "4")])
        b.tie(net, (X - 30 * G, yswd))
        yswd -= 4 * G

    # ---------------------------------------------------------- signal pins: ports for the sheet's nets, labels for
    # the local ones (straps, LED), on whichever side the pin is
    pullup = {"SCL2": XR, "SDA2": XR + 10 * G}       # I2C2 ports past their pull-ups
    wired = {"ADR1", "LED_RUN"}                      # drawn to their circuits below
    for num, net in sorted(nets.items(), key=lambda kv: int(kv[0])):
        if net in RIGHT_PORTS or net in SHAPES:
            pos = (pullup[net], pin(num)[1]) if net in pullup else ("U3", num)
            b.port(net, shape(net), side="right" if u.pin_out(num) == (1, 0) else "left", pos=pos)
        elif (net.startswith("ADR") or net == "LED_RUN") and net not in wired:
            b.tie(net, ("U3", num), 0 if u.pin_out(num) == (1, 0) else 180)

    # ---------------------------------------------------------- run LED (right, below the pins)
    yl = Y + 38 * G
    b.sym("R19", "Device:R", "1k", R0402, {"1": "LED_RUN", "2": "LEDRUN_A"}, at=(XR - G, yl), rot=90,
          desc="firmware heartbeat LED")
    b.sym("D4", "Device:LED", "GREEN RUN", LED0603, {"1": "GND", "2": "LEDRUN_A"}, at=(XR + 14 * G, yl), rot=180,
          fields={"LCSC": "C2297"})
    b.wire("GND", [(XR + 22 * G, yl), ("D4", "1")])     # GND clear of the LED arrows
    num = inv["LED_RUN"]
    b.wire("LED_RUN", [("U3", num), (XR - 6 * G, pin(num)[1]), (XR - 6 * G, yl), ("R19", "1")])
    b.name("LED_RUN", at=(XR - 6 * G, yl - G), ang=90)

    # ---------------------------------------------------------- I2C2 pull-ups (right, on the pin rows)
    for ref, net, x, y, desc in (("R24", "SCL2", XR - 2 * G, Y - 15 * G, ""),
                                 ("R23", "SDA2", XR + 8 * G, Y - 13 * G, "I2C2 pull-up (mux upstream, units)")):
        b.sym(ref, "Device:R", "2.2k", R0402, {"1": "+3V3", "2": net}, at=(x, y), desc=desc)
        num = inv[net]
        b.wire(net, [("U3", num), (ref, "2"), (pullup[net], pin(num)[1])])

    # ---------------------------------------------------------- address straps (below left): SW1, pull-ups on the rows
    b.sym("SW1", "Switch:SW_DIP_x03", "ADR", "Button_Switch_SMD:SW_DIP_SPSTx03_Slide_KingTek_DSHP03TS_W7.62mm_P1.27mm",
          {"1": "ADR0", "2": "ADR1", "3": "ADR2", "4": "GND", "5": "GND", "6": "GND"}, at=(X - 52 * G, Y + 6 * G), mirror="y",
          fields={"LCSC": "C50844", "MPN": "KingTek DSHP03TSGET"}, ref_at=(0, -10.16), val_at=(0, -7.62),
          desc="address straps, ON = 0; plane address 0x30 + ADR[2:0]")
    fat = [dict(ref_at=(-1.905, -1.27, "right"), val_at=(-1.905, 1.27, "right")), {},
           dict(ref_at=(1.905, -2.54, "left"), val_at=(1.905, 0, "left"))]
    for i, dx in enumerate((-42, -32, -20)):
        b.sym("R%d" % (20 + i), "Device:R", "100k", R0402, {"1": "+3V3", "2": "ADR%d" % i}, at=(X + dx * G, Y + (1 + 2 * i) * G),
              desc="address strap pull-up", **fat[i])
    b.wire("ADR0", [("SW1", "1"), ("R20", "2"), (X - 36 * G, Y + 4 * G)])
    b.tie("ADR0", (X - 36 * G, Y + 4 * G), 180)
    b.wire("ADR1", [("SW1", "2"), ("R21", "2"), (X - 24 * G, Y + 6 * G), (X - 24 * G, pin(inv["ADR1"])[1]), ("U3", inv["ADR1"])])
    b.name("ADR1", at=(X - 24 * G, Y + G), ang=90)
    b.wire("ADR2", [("SW1", "3"), ("R22", "2"), (X - 14 * G, Y + 8 * G)])
    b.tie("ADR2", (X - 14 * G, Y + 8 * G), 180)


    return b.finish()
