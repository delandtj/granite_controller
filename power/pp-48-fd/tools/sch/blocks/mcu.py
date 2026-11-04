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
               "CAN_TX": "output", "CAN_RX": "input"}
# local nets between U3 and the MCP2518FD U5 (labels, no ports)
MCP_NETS = ("MCP_CS", "SPI_SCK", "SPI_MISO", "SPI_MOSI", "MCP_INT")


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
    "47": "CAN_RX", "48": "CAN_TX",  # PB8 / PB9: ISO1044 U4 RXD / TXD (FDCAN1 AF3 on the G0B1; G070: PB9 left in reset, high-Z)
    "15": "MCP_CS", "16": "SPI_SCK", "17": "SPI_MISO", "18": "SPI_MOSI",   # PA4..PA7 SPI1 AF0: MCP2518FD U5
    "11": "MCP_INT",                 # PA0: MCP2518FD nINT (EXTI)
    "33": "SCL2", "34": "SDA2",      # PA11 / PA12 I2C2 AF6: master, mux U6 and the units
    "23": "VIN_SNS",                 # PB11 ADC_IN15
    "24": "PRESS_TRIG",              # PB12: edges retrigger the press deadline (docs/press-deadline-line.md)
    "25": "MUX_RST",                 # PB13
    "26": "LED_RUN",                 # PB14
    "38": "EXP_RSTN",                # PD0 open drain: the units' TCA6424A /RESET (R30 pull-ups on the units)
    "37": "ADR0", "39": "ADR1", "28": "ADR2",   # PA15 / PD1 / PA8 straps (DIP switch SW1 to GND, R20-22 up)
}

# what the firmware needs per pin (written to firmware/plane-core-fd-pins.toml by gensch.py core):
# pin -> (port, mode, use); the fixed pins too
PIN_USE = {
    "47": ("PB8", "analog (G070) / FDCAN1_RX AF3 (G0B1)", "CAN_RX: ISO1044 U4 RXD (also the MCP2518FD RXCAN); G070 build: unused, leave in reset (analog)"),
    "48": ("PB9", "analog (G070) / FDCAN1_TX AF3 (G0B1)", "CAN_TX: ISO1044 U4 TXD; G070 build: the MCP2518FD TXCAN drives it, PB9 MUST stay in reset (analog, high-Z)"),
    "15": ("PA4", "output push-pull (SPI1_NSS by software)", "MCP2518FD U5 nCS (100k pull-up R33); G070 build"),
    "16": ("PA5", "SPI1_SCK AF0", "MCP2518FD U5 SCK (<= 17 MHz at 40 MHz SYSCLK); G070 build"),
    "17": ("PA6", "SPI1_MISO AF0", "MCP2518FD U5 SDO; G070 build"),
    "18": ("PA7", "SPI1_MOSI AF0", "MCP2518FD U5 SDI; G070 build"),
    "11": ("PA0", "input, EXTI falling", "MCP2518FD U5 nINT (100k pull-up R34); G070 build"),
    "33": ("PA11", "I2C2_SCL AF6", "I2C master: TCA9548A 0x70, then one unit segment per channel"),
    "34": ("PA12", "I2C2_SDA AF6", "I2C master: TCA9548A 0x70, then one unit segment per channel"),
    "23": ("PB11", "ADC_IN15", "core 48 V input, 220k / 10k divider"),
    "24": ("PB12", "output push-pull", "toggle to retrigger the press deadline U301 (~1 s); never toggle when no press runs"),
    "25": ("PB13", "output push-pull", "high = mux reset, all channels off (Q6 inverts; 100k pull-down)"),
    "26": ("PB14", "output push-pull", "heartbeat LED, high = on"),
    "38": ("PD0", "output open-drain", "low = reset every unit's TCA6424A (bused, pull-ups on the units)"),
    "37": ("PA15", "input", "address strap ADR0: pulled up, DIP switch SW1 ON = 0; CAN node id = ADR[2:0] (ADR 0006)"),
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
    out = ["# STM32G070CBT6 (LQFP-48) pin designations of the PP-48 core board, CAN FD link",
           "# (docs/adr/0006-plane-link-can-fd.md). The G0B1CCT6 build uses the same pins (FDCAN1 on PB8 / PB9,",
           "# MCP2518FD U5 not fitted).",
           "# GENERATED by power/pp-48-fd/tools/gensch.py core from tools/sch/blocks/mcu.py",
           "# (CORE_PINS / PIN_USE): do not edit here, change the generator and regenerate.",
           "# The schematic (power/pp-48-core-fd/sheets/mcu.kicad_sch) and the board follow the same table.",
           "# hse_hz: the STM32's own HSE; the MCP2518FD runs on its own 40 MHz crystal Y2.",
           "", '[mcu]', 'part = "STM32G070CBT6"', 'package = "LQFP-48"', 'hse_hz = 8000000', ""]
    for num in sorted(PIN_USE, key=lambda n: (PIN_USE[n][0][:2], int(PIN_USE[n][0][2:]) if PIN_USE[n][0][2:].isdigit() else 0)):
        port, mode, use = PIN_USE[num]
        out += ['[[pin]]', 'port = "%s"' % port, 'pin = %s' % num, 'net = "%s"' % nets[num],
                'mode = "%s"' % mode, 'use = "%s"' % use, ""]
    unused = sorted((n for n, net in nets.items() if net == "NC"), key=int)
    out += ["# not connected (pins 29 / 32 are not bonded on the G070; PA9 / PA10 on the G0B1, left open)", "[unused]", "pins = [%s]" % ", ".join(unused), ""]
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
              at=(X, Y), fields={"LCSC": "C529338"},
              desc="plane MCU (G070 build; STM32G0B1CCT6 in the G0B1 build, same footprint): CAN_RX / CAN_TX (PB8/PB9), SPI1 to the MCP2518FD (PA4-PA7, nINT PA0), I2C2 master to the mux and the units (PA11/PA12), SWD on PA13/PA14")
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
    # the same SWD on the ARM 10-pin Cortex header (1.27 mm, keyed shroud): stock probe ribbons
    b.sym("J6", "Connector_Generic:Conn_02x05_Odd_Even", "SWD", "Connector_PinHeader_1.27mm:PinHeader_2x05_P1.27mm_Vertical_SMD",
          {"1": "+3V3", "2": "SWDIO", "3": "GND", "4": "SWCLK", "5": "GND", "6": "NC", "7": "NC", "8": "NC", "9": "GND",
           "10": "NRST"}, at=(xj - 29 * G, yj + 4 * G),
          fields={"LCSC": "C5155080", "MPN": "Samtec FTSH-105-01-L-DV-K"}, desc="bench programming, ARM Cortex 10-pin")
    j6 = b.syms["J6"]                               # GND: 3 / 5 joined, 9 from below (clear of 7)
    xg = j6.pin_xy("5")[0] - 2 * G
    b.wire("GND", [("J6", "3"), ("J6", "5")])
    b.wire("GND", [(xg, j6.pin_xy("9")[1] + 3 * G), (xg, j6.pin_xy("9")[1]), ("J6", "9")])
    for num, net in (("2", "SWDIO"), ("4", "SWCLK"), ("10", "NRST")):   # 6 SWO, 7 key, 8 TDI open
        b.tie(net, ("J6", num), 0)
    for num, net in (("2", "SWDIO"), ("4", "SWCLK"), ("3", "NRST")):
        b.tie(net, ("J5", num), 90)
    b.sym("C21", "Device:C", "100n", C0402, {"1": "NRST", "2": "GND"}, at=(X - 40 * G, pin("10")[1] + 5 * G), desc="NRST")
    xn = xw - 4 * G
    yjt = j.pin_xy("3")[1]
    b.wire("NRST", [("U3", "10"), (xn, pin("10")[1]), (xn, yjt - 6 * G), (j.pin_xy("3")[0], yjt - 6 * G), ("J5", "3")])
    b.wire("NRST", [(X - 40 * G, pin("10")[1]), ("C21", "1")])

    # ---------------------------------------------------------- SWD fan-out (right-hand pins, over the top to J5)
    inv = {net: num for num, net in nets.items()}
    x = XR + 10 * G                                   # outside the labels at the right-hand pins
    yswd = j.pin_xy("2")[1] - 2 * G                   # SWD rows clear J5's pin tops
    jx = {"SWDIO": j.pin_xy("2")[0], "SWCLK": j.pin_xy("4")[0]}
    for net in ("SWDIO", "SWCLK"):                    # SWDIO is the lower pin: outer column, lower row
        num = inv[net]
        x += 2 * G
        b.wire(net, [("U3", num), (x, pin(num)[1]), (x, yswd), (jx[net], yswd), ("J5", "2" if net == "SWDIO" else "4")])
        yswd -= 2 * G

    # ---------------------------------------------------------- signal pins: ports for the sheet's nets, labels for
    # the local ones (straps, LED), on whichever side the pin is
    for num, net in sorted(nets.items(), key=lambda kv: int(kv[0])):
        if net in RIGHT_PORTS or net in SHAPES:
            b.port(net, shape(net), side="right" if u.pin_out(num) == (1, 0) else "left", pos=("U3", num))
        elif net.startswith("ADR") or net == "LED_RUN" or net in MCP_NETS:
            b.tie(net, ("U3", num), 0 if u.pin_out(num) == (1, 0) else 180)

    # ---------------------------------------------------------- run LED (right, below the pins)
    yl = Y + 40 * G
    b.sym("R19", "Device:R", "1k", R0402, {"1": "LED_RUN", "2": "LEDRUN_A"}, at=(XR + 11 * G, yl), rot=90,
          desc="firmware heartbeat LED")
    b.sym("D4", "Device:LED", "GREEN RUN", LED0603, {"1": "GND", "2": "LEDRUN_A"}, at=(XR + 27 * G, yl), rot=180,
          fields={"LCSC": "C2297"})
    b.wire("GND", [(XR + 33 * G, yl), ("D4", "1")])     # GND clear of the LED arrows
    b.tie("LED_RUN", ("R19", "1"), 180)

    # ---------------------------------------------------------- I2C2 pull-ups (right, beside the SWD fan-out)
    for k, (ref, net, desc) in enumerate((("R24", "SCL2", ""), ("R23", "SDA2", "I2C2 pull-up (mux upstream, units)"))):
        b.sym(ref, "Device:R", "2.2k", R0402, {"1": "+3V3", "2": net}, at=(XR + (22 + 10 * k) * G, Y - 14 * G), desc=desc)
        b.tie(net, (ref, "2"), 270)

    # ---------------------------------------------------------- address straps (below left)
    for i in range(3):
        net = "ADR%d" % i
        xa = XL - (4 + 10 * (2 - i)) * G
        b.sym("R%d" % (20 + i), "Device:R", "100k", R0402, {"1": "+3V3", "2": net}, at=(xa - 6 * G, Y + 6 * G),
              desc="address strap pull-up")
        b.wire(net, [("R%d" % (20 + i), "2"), (xa - 6 * G, Y + 11 * G), (xa, Y + 11 * G)])
        b.tie(net, (xa, Y + 11 * G), 0)
        b.tie(net, ("SW1", str(1 + i)), 180)
    b.sym("SW1", "Switch:SW_DIP_x03", "ADR", "Button_Switch_SMD:SW_DIP_SPSTx03_Slide_KingTek_DSHP03TS_W7.62mm_P1.27mm",
          {"1": "ADR0", "2": "ADR1", "3": "ADR2", "4": "GND", "5": "GND", "6": "GND"}, at=(XL - 14 * G, Y + 19 * G),
          fields={"LCSC": "C50844", "MPN": "KingTek DSHP03TSGET"},
          desc="address straps, ON = 0; plane address 0x30 + ADR[2:0]")

    canfd(b)
    return b.finish()


G070 = "G070 build only; not fitted in the G0B1 build"


def canfd(b):
    """MCP2518FD U5 on SPI1 with its 40 MHz crystal (ADR 0006 decisions 6 / 7), right of U3"""
    def P(x, y): return (x * G, y * G)
    X, Y = 250, 125
    b.sym("U5", "Interface_CAN_LIN:MCP2517FD-xSL", "MCP2518FDT-E/SL", "Package_SO:SOIC-14_3.9x8.7mm_P1.27mm",
          {"1": "CAN_TX", "2": "CAN_RX", "3": "NC", "4": "MCP_INT", "5": "MCP_OSC2", "6": "MCP_OSC1", "7": "GND",
           "8": "MCP_GP1", "9": "MCP_GP0", "10": "SPI_SCK", "11": "SPI_MOSI", "12": "SPI_MISO", "13": "MCP_CS", "14": "+3V3"},
          at=P(X, Y), fields={"LCSC": "C626758", "MPN": "Microchip MCP2518FDT-E/SL"},
          desc="CAN FD controller on SPI1 (MCP2517FD symbol, same SOIC-14 pinout); TXCAN / RXCAN to the ISO1044 U4. "
               "CLKO open; INT0 / INT1 (GPIO inputs at POR) pulled up. " + G070)
    for num, net in (("1", "CAN_TX"), ("2", "CAN_RX"), ("4", "MCP_INT"), ("8", "MCP_GP1"), ("9", "MCP_GP0")):
        b.tie(net, ("U5", num), 0)
    for num, net in (("10", "SPI_SCK"), ("11", "SPI_MOSI"), ("12", "SPI_MISO"), ("13", "MCP_CS"),
                     ("5", "MCP_OSC2"), ("6", "MCP_OSC1")):
        b.tie(net, ("U5", num), 180)
    b.sym("C35", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=P(X + 16, Y - 14), desc="U5 VDD; " + G070)
    b.wire("+3V3", [P(X + 20, Y - 17), ("C35", "1"), P(X, Y - 17), ("U5", "14")])

    # 40 MHz crystal, CL 15 pF: 2 x (15 - ~4 pF stray) = 22 pF
    xy, yc = X - 14, Y + 25
    b.sym("Y2", "Device:Crystal_GND24", "40MHz", "Crystal:Crystal_SMD_3225-4Pin_3.2x2.5mm",
          {"1": "MCP_OSC1", "2": "GND", "3": "MCP_OSC2", "4": "GND"}, at=P(xy, yc),
          fields={"LCSC": "C9010", "MPN": "YXC X322540MPB4SI"},
          desc="MCP2518FD clock, 40 MHz 3225, CL 15 pF, 10 ppm; " + G070)
    b.sym("C33", "Device:C", "22p", C0402, {"1": "MCP_OSC1", "2": "GND"}, at=P(xy - 8, yc + 5), fields={"LCSC": "C1555"},
          desc="Y2 load, C0G: 2 x (CL 15 pF - ~4 pF stray); " + G070)
    b.sym("C34", "Device:C", "22p", C0402, {"1": "MCP_OSC2", "2": "GND"}, at=P(xy + 8, yc + 5), fields={"LCSC": "C1555"},
          desc="Y2 load; " + G070)
    b.wire("GND", [("Y2", "2"), ("Y2", "4")])
    b.wire("MCP_OSC1", [("Y2", "1"), P(xy - 8, yc), P(xy - 12, yc)])
    b.wire("MCP_OSC1", [P(xy - 8, yc), ("C33", "1")])
    b.tie("MCP_OSC1", P(xy - 12, yc), 180)
    b.wire("MCP_OSC2", [("Y2", "3"), P(xy + 8, yc), P(xy + 12, yc)])
    b.wire("MCP_OSC2", [P(xy + 8, yc), ("C34", "1")])
    b.tie("MCP_OSC2", P(xy + 12, yc), 0)

    # pull-ups: nCS (deselected while U3 is in reset), nINT (PA0 defined in the G0B1 build), INT0 / INT1
    for k, (ref, net, desc) in enumerate((("R33", "MCP_CS", "U5 nCS pull-up: deselected while U3 is in reset"),
                                          ("R34", "MCP_INT", "U5 nINT pull-up (PA0 defined with U5 not fitted)"),
                                          ("R35", "MCP_GP0", "U5 INT0/GPIO0/XSTBY: GPIO input at POR, not used (ISO1044 has no standby); " + G070),
                                          ("R36", "MCP_GP1", "U5 INT1/GPIO1: GPIO input at POR, not used; " + G070))):
        b.sym(ref, "Device:R", "100k", R0402, {"1": "+3V3", "2": net}, at=P(X + 24 + 9 * k, Y - 26), desc=desc)
        b.tie(net, (ref, "2"), 270)
    for ref, mpn in (("C35", "Samsung CL05B104KO5NNNC"), ("C33", "FH 0402CG220J500NT"), ("C34", "FH 0402CG220J500NT"),
                     ("R33", "UniOhm 0402WGF1003TCE"), ("R34", "UniOhm 0402WGF1003TCE"),
                     ("R35", "UniOhm 0402WGF1003TCE"), ("R36", "UniOhm 0402WGF1003TCE")):
        b.syms[ref].fields["MPN"] = mpn
    b.text("CAN FD CONTROLLER  U5 MCP2518FD, G070 build only (not fitted with the G0B1: FDCAN1 on PB8 / PB9)", P(X - 30, Y - 36))
