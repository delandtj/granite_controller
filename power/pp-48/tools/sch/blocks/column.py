"""Column sheet: one slot column of the PP-48 plane, drawn once, instantiated N times.

docs/adr/0004-plane-column-layout.md. The front end (shunts, LM5069-2,
pass FET, UVLO / OVLO, timer, PGD, output clamp), the INA238, the enable
chain and the OV latch are the PP-48-2S slot block (power/pp-48-2s/tools/
sch/blocks/slot.py) unchanged, at the same coordinates. What changed:

- The module is screwed down (docs/pm-interface.md): four standoffs SO1-SO4
  (S1 VIN = OUT, S2 / S3 GND, S4 VOUT = NODE) and the 2x3 1.27 mm aux socket
  J3 (1 OV_KILL, 2 PGOOD, 3 ID, 4 RUN, 5 / 6 GND) replace the DOSA brick
  and its 7-pin row. TEMP, SDA / SCL to the module are gone.
- The column has its own I2C segment (one TCA9548A channel in the core), so
  every column carries the same addresses: INA238 0x40 (A0 A1 GND), TCA9535
  0x20 (A0-A2 GND), ADC121C027 0x51 (ADR0 GND). SDA and SCL are the only
  ports; the segment pull-ups are in the column.
- The TCA9535 carries the slot's control and status lines (port 0 out:
  OFF LCLR RUNOFF PWRD RSTD DISARM, inputs PGD PG LQ LED OVK; pins chosen by
  the side of the board each signal goes to, see U5's description). They are
  tied to the front end by local labels, not wires. INT is not used (the MCU
  polls). At reset the expander pins are inputs: OFF, LCLR and RUNOFF fall
  to their 100k pull-downs (slot ON, latch kept, module running) and the
  photoMOS LEDs are dark.
- ID: 10k pull-up, 1k series, 100 nF at the ADC121C027 input; the ADC reads
  ID / 3V3 (VA is its reference). RUN: Q5 pulls the module's RUN pin low
  when the expander drives RUNOFF high (open drain, never driven high).
- Node: 2-pole screw terminal J1 (Degson DG128) for 19 V, JST GH 4-pin J2
  for the front panel (PWR_SW, RST_SW, PLED+, PLED-; drawn as a 2x2 so the
  PP-48-2S photoMOS / opto wiring stays as it was). The slot ON and node
  LEDs are dropped (nobody sees them in the tank), and so is the cable-side
  NODE cap (the terminal sits ~10 mm from S4, next to the module's own
  output caps).

- Slot latch U3 is set (/PRE = OVK, low) by three open-drain sources: the
  module's OV_KILL, the INA238 ALERT (power limit, ALATCH; firmware sets
  the limit at boot, the reset value never trips) and the PGOOD-loss trip
  Q6. The INA238 DIAG_ALRT register tells a power trip from the other two.
- PGOOD-loss blanking (ADR 0003): U10 holds ARM. It is set while the
  module's PGOOD and the LM5069 PGD are both high (Q8 / Q9 in series on
  /PRE, level, so either may rise first) and cleared while PGD is low (any
  slot-off: OFF, latch, UVLO / OVLO, LM5069 fault). Armed, PGOOD going low
  turns Q6 on (gate ARM, source PG), which pulls OVK low and sets U3; the
  slot cut then drops PGD and disarms U10. DISARM (expander P10, Q7 pulls
  PGD low) keeps it disarmed around RUNOFF: set DISARM before RUNOFF,
  release it whenever (it re-arms once PGOOD is back).

Coordinates are in grid units of 1.27 mm (P(x, y)).
"""
from sch.draw import Block, R0402, R0805, R2512, C0402, C0603, C1210
from sch.kicad import by_name

G = 1.27
FET = "Transistor_FET:2N7002"
SOT23 = "Package_TO_SOT_SMD:SOT-23"
TLP176 = ("pp:TLP176AM", "TLP176AM", "Package_SO:SOIC-4_4.55x3.7mm_P2.54mm")
STANDOFF = "pp48:PM_Standoff_M2x7_SMTSO"


def P(x, y):
    return (x * G, y * G)


def block():
    b = Block("Column", date="2026-10-28")
    b.power("VBUS", "power:VBUS")

    def sym(ref, lib, val, fp, nets, at, rot=0, mirror=None, **kw):
        b.sym(ref, lib, val, fp, nets, at=P(*at), rot=rot, mirror=mirror, **kw)

    def R(n, val, fp, nets, at, rot=0, **kw):
        sym("R%d" % n, "Device:R", val, fp, nets, at, rot, **kw)

    def C(n, val, fp, nets, at, rot=0, **kw):
        sym("C%d" % n, "Device:C", val, fp, nets, at, rot, **kw)

    def wire(net, *pts):
        b.wire(net, [p if isinstance(p[0], str) else P(*p) for p in pts])

    def name(net, at, ang=0):
        b.name(net, at=P(*at), ang=ang)

    def tie(net, at, ang=0):
        b.tie(net, at if isinstance(at[0], str) else P(*at), ang)

    def port(net, shape, side, at):
        b.port(net, shape, side=side, pos=P(*at))

    # ========================================================== hot-swap front end (top rail y = 32), as PP-48-2S
    R(1, "7m 2W", R2512, {"1": "VBUS", "2": "SENSE"}, (35, 32), rot=90, fields={"LCSC": "C2904232"},
      desc="shunt, two 7 mOhm in parallel = 3.5 mOhm: limit 15.7 A, breaker 30 A")
    R(2, "7m 2W", R2512, {"1": "VBUS", "2": "SENSE"}, (35, 38), rot=90, fields={"LCSC": "C2904232"},
      desc="shunt (parallel)")
    sym("Q1", "Transistor_FET:BSC070N10NS5", "BSC070N10NS5", "Package_TO_SOT_SMD:TDSON-8-1",
        by_name("Transistor_FET:BSC070N10NS5", {"G": "GATE", "S": "OUT", "D": "SENSE"}), (98, 34), rot=90,
        desc="100 V pass MOSFET, placeholder: SOA check open (ADR 0001)")
    sym("U1", "Power_Management:LM5069MM-2", "LM5069MM-2", "Package_SO:MSOP-10_3x3mm_P0.5mm",
        {"1": "SENSE", "2": "VBUS", "3": "UVLO", "4": "OVLO", "5": "GND", "6": "TIMER", "7": "PWR", "8": "PGD", "9": "OUT", "10": "GATE"},
        (96, 56), fields={"LCSC": "C111822"}, desc="hot-swap, 55 mV current limit, power limit, auto-retry 0.5 %")
    R(3, "143k", R0805, {"1": "VBUS", "2": "UVLO"}, (68, 69), desc="UVLO: on 38.3 V, off 35.3 V (21 uA hysteresis)")
    R(4, "10k", R0402, {"1": "UVLO", "2": "GND"}, (68, 77))
    R(5, "124k", R0805, {"1": "VBUS", "2": "OVLO"}, (76, 55),
      desc="OVLO: off 64.6 V (60.9-68.5 V over 2.4-2.6 V and 1 %), back on ~2.6 V lower (12-30 uA x 124k); module trips at 67.5 V")
    R(6, "4.99k", R0402, {"1": "OVLO", "2": "GND"}, (76, 63))
    R(7, "13.7k", R0402, {"1": "PWR", "2": "GND"}, (116, 66),
      desc="power limit ~50 W at 60 V (RPWR = 1.3e5 x (RSNS x PLIM - 1.18mV x VDS))")
    C(1, "220n", C0603, {"1": "TIMER", "2": "GND"}, (108, 68), desc="insertion 160 ms, fault timeout 10 ms")
    R(8, "10k", R0402, {"1": "+3V3", "2": "PGD"}, (126, 52), desc="PGD pull-up")
    sym("D1", "Device:D_TVS", "SMBJ60A", "Diode_SMD:D_SMB", {"1": "GND", "2": "OUT"}, (112, 38), rot=90,
        fields={"LCSC": "C353345"}, desc="slot output clamp: 60 V standoff = top of the bus range, breakdown >= 66.7 V; SMBJ64A would clamp at 103 V, over the 100 V FET and module")
    C(2, "2.2u 100V", C1210, {"1": "OUT", "2": "GND"}, (122, 38), fields={"LCSC": "C338133"},
      desc="slot output, in front of the module")

    # ========================================================== INA238 under the shunt
    sym("U2", "Sensor_Energy:INA238", "INA238AIDGSR", "Package_SO:TSSOP-10_3x3mm_P0.5mm",
        {"1": "GND", "2": "GND", "3": "OVK", "4": "SDA", "5": "SCL", "6": "+3V3", "7": "GND", "8": "OUT", "9": "SENSE", "10": "VBUS"},
        (44, 60), rot=270,
        desc="16-bit monitor on the shunt, address 0x40 (A0 A1 GND, same in every column); VBUS pin reads the slot output")
    C(3, "100n", C0402, {"1": "+3V3", "2": "GND"}, (58, 63), desc="INA238 VS")

    # ========================================================== enable chain and OV latch, as PP-48-2S
    sym("Q2", FET, "2N7002", SOT23, {"1": "OFF", "2": "GND", "3": "UVLO"}, (82, 90),
        fields={"LCSC": "C8545"}, desc="expander OFF: high = slot off; expander in reset = pull-down = ON")
    R(9, "100k", R0402, {"1": "OFF", "2": "GND"}, (76, 95), desc="default ON")
    sym("Q3", FET, "2N7002", SOT23, {"1": "LQ", "2": "GND", "3": "UVLO"}, (94, 82), rot=90,
        fields={"LCSC": "C8545"}, desc="OV latch output cuts the slot")
    R(10, "100k", R0402, {"1": "LQ", "2": "GND"}, (98, 101))
    sym("U3", "74xGxx:74AUC1G74", "74LVC1G74", "Package_SO:VSSOP-8_2.3x2mm_P0.5mm",
        {"1": "GND", "2": "GND", "3": "NC", "4": "GND", "5": "LQ", "6": "CLRN", "7": "OVK", "8": "+3V3"},
        (112, 100), mirror="y", fields={"LCSC": "C70285"},
        desc="OV latch: /PRE = module OV_KILL (low = set), /CLR = RC power-on clear + expander; Q holds UVLO low until cleared")
    C(4, "100n", C0402, {"1": "+3V3", "2": "GND"}, (104, 84), desc="latch VCC")
    R(11, "100k", R0402, {"1": "+3V3", "2": "CLRN"}, (128, 111), desc="power-on clear RC (10 ms)")
    C(5, "100n", C0402, {"1": "CLRN", "2": "GND"}, (122, 118))
    sym("Q4", FET, "2N7002", SOT23, {"1": "LCLR", "2": "GND", "3": "CLRN"}, (132, 127),
        fields={"LCSC": "C8545"}, desc="expander clears the latch: drive high -> Q4 pulls /CLR low")
    R(12, "100k", R0402, {"1": "LCLR", "2": "GND"}, (118, 131), desc="Q4 gate pull-down: expander in reset = latch not cleared")

    # ========================================================== PGOOD-loss trip with blanking
    sym("U10", "74xGxx:74AUC1G74", "74LVC1G74", "Package_SO:VSSOP-8_2.3x2mm_P0.5mm",
        {"1": "GND", "2": "GND", "3": "NC", "4": "GND", "5": "ARM", "6": "PGD", "7": "ARMS_N", "8": "+3V3"},
        (130, 170), fields={"LCSC": "C70285"},
        desc="PGOOD-loss arm: set while PG and PGD are high, cleared while PGD is low")
    C(12, "100n", C0402, {"1": "+3V3", "2": "GND"}, (148, 160), desc="arm FF VCC")
    R(28, "100k", R0402, {"1": "+3V3", "2": "ARMS_N"}, (112, 156), desc="arm set pull-up")
    sym("Q8", FET, "2N7002", SOT23, {"1": "PG", "2": "ARMS_M", "3": "ARMS_N"}, (112, 170),
        fields={"LCSC": "C8545"}, desc="arm set, upper: module PGOOD high")
    sym("Q9", FET, "2N7002", SOT23, {"1": "PGD", "2": "GND", "3": "ARMS_M"}, (112, 190),
        fields={"LCSC": "C8545"}, desc="arm set, lower: LM5069 PGD high")
    sym("Q6", FET, "2N7002", SOT23, {"1": "ARM", "2": "PG", "3": "OVK"}, (160, 176),
        fields={"LCSC": "C8545"}, desc="PGOOD-loss trip: armed and PG low -> OVK low -> slot latch set")
    sym("Q7", FET, "2N7002", SOT23, {"1": "DISARM", "2": "GND", "3": "PGD"}, (176, 160),
        fields={"LCSC": "C8545"}, desc="expander DISARM high -> PGD low -> arm cleared (around RUNOFF)")
    R(29, "100k", R0402, {"1": "DISARM", "2": "GND"}, (170, 172), desc="Q7 gate pull-down: expander in reset = armed path live")

    # ========================================================== module mount: standoffs and aux socket
    sym("SO1", "Connector_Generic:Conn_01x01", "S1 VIN", STANDOFF, {"1": "OUT"}, (150, 32),
        fields={"LCSC": "C2916355", "MPN": "Sinhoo SMTSO2070CTJ"}, desc="module screw S1: VIN (docs/pm-interface.md)")
    sym("SO2", "Connector_Generic:Conn_01x01", "S2 GND", STANDOFF, {"1": "GND"}, (150, 40),
        fields={"LCSC": "C2916355", "MPN": "Sinhoo SMTSO2070CTJ"}, desc="module screw S2: GND, input end")
    sym("SO4", "Connector_Generic:Conn_01x01", "S4 VOUT", STANDOFF, {"1": "NODE"}, (176, 32), mirror="y",
        fields={"LCSC": "C2916355", "MPN": "Sinhoo SMTSO2070CTJ"}, desc="module screw S4: VOUT 19 V")
    sym("SO3", "Connector_Generic:Conn_01x01", "S3 GND", STANDOFF, {"1": "GND"}, (176, 40), mirror="y",
        fields={"LCSC": "C2916355", "MPN": "Sinhoo SMTSO2070CTJ"}, desc="module screw S3: GND, output end")
    sym("J3", "Connector_Generic:Conn_01x06", "AUX", "pp48:PM_Aux_Socket_2x03_P1.27",
        {"1": "OVK", "2": "PG", "3": "ID", "4": "RUN", "5": "GND", "6": "GND"}, (168, 68),
        fields={"LCSC": "C41397112", "MPN": "hanxia HX PM1.27-2x3P TP"},
        desc="module aux: 1 OV_KILL, 2 PGOOD, 3 ID, 4 RUN, 5-6 GND (pin n = An, docs/pm-interface.md)")
    R(13, "10k", R0402, {"1": "+3V3", "2": "OVK"}, (150, 53), desc="OV_KILL pull-up (module comparator is open drain)")
    R(14, "10k", R0402, {"1": "+3V3", "2": "PG"}, (156, 63), desc="module PGOOD pull-up (open = good; presence comes from ID)")
    R(15, "10k", R0402, {"1": "+3V3", "2": "ID"}, (148, 77), desc="ID divider top (module resistor to GND)")
    R(24, "1k", R0402, {"1": "ID_F", "2": "ID"}, (142, 82), rot=90, desc="ID series to the ADC")
    sym("Q5", FET, "2N7002", SOT23, {"1": "RUNOFF", "2": "GND", "3": "RUN"}, (154, 104),
        fields={"LCSC": "C8545"}, desc="module RUN: expander high -> module off (pulls the UVLO divider node low, never drives it)")
    R(23, "100k", R0402, {"1": "RUNOFF", "2": "GND"}, (146, 109), desc="Q5 gate pull-down: expander in reset = module on")

    # ========================================================== node power terminal and front panel
    sym("J1", "Connector:Screw_Terminal_01x02", "NODE 19V", "pp48:TerminalBlock_Degson_DG128-5.0-02P",
        {"1": "GND", "2": "NODE"}, (206, 34), mirror="x", fields={"LCSC": "C711349", "MPN": "Degson DG128-5.0-02P-14-00A(H)"},
        desc="node 19 V: 1 GND, 2 +19 V (pin 2 sits straight under S4 VOUT); 14 AWG pair to the node's DC-ATX converter (UL 15 A)")
    sym("J2", "Connector_Generic:Conn_02x02_Odd_Even", "FRONT PANEL",
        "Connector_JST:JST_GH_SM04B-GHS-TB_1x04-1MP_P1.25mm_Horizontal",
        {"1": "PWR_SW", "2": "RST_SW", "3": "PLED_P", "4": "PLED_N"}, (226, 36),
        fields={"LCSC": "C189895", "MPN": "JST SM04B-GHS-TB"},
        desc="JST GH 1x4, latching: 1 PWR_SW, 2 RST_SW, 3 PLED+, 4 PLED- (drawn 2x2, pads 1-4)")
    sym("U6", *TLP176, {"1": "PWRD_A", "3": "GND", "4": "GND", "6": "PWR_SW"}, (208, 58),
        fields={"LCSC": "C5370540"}, desc="photoMOS front-panel PWR_SW contact")
    R(19, "270", R0402, {"1": "PWRD", "2": "PWRD_A"}, (192, 56), rot=90, fields={"LCSC": "C22775"},
      desc="photoMOS LED, ~6.5 mA from the expander")
    sym("U7", *TLP176, {"1": "RSTD_A", "3": "GND", "4": "GND", "6": "RST_SW"}, (246, 46), mirror="y",
        fields={"LCSC": "C5370540"}, desc="photoMOS front-panel RST_SW contact")
    R(20, "270", R0402, {"1": "RSTD", "2": "RSTD_A"}, (262, 44), rot=270, fields={"LCSC": "C22775"},
      desc="photoMOS LED, ~6.5 mA from the expander")
    sym("U8", "Isolator:TLP291", "TLP291", "Package_SO:SOIC-4_4.55x2.6mm_P1.27mm",
        {"1": "PLED_A", "2": "PLED_N", "3": "GND", "4": "LED"}, (226, 62), rot=270, mirror="y",
        fields={"LCSC": "C55144"}, desc="power LED sense opto: LED lit -> output low")
    R(21, "220", R0402, {"1": "PLED_P", "2": "PLED_A"}, (220, 48), desc="opto LED series (as rev C)")
    R(22, "47k", R0402, {"1": "+3V3", "2": "LED"}, (216, 69), desc="opto collector pull-up")

    # ========================================================== I2C segment: pull-ups, expander, ID ADC
    R(26, "4.7k", R0402, {"1": "+3V3", "2": "SDA"}, (20, 132), desc="segment SDA pull-up (one per mux channel)")
    R(27, "4.7k", R0402, {"1": "+3V3", "2": "SCL"}, (26, 132), desc="segment SCL pull-up")
    sym("U5", "Interface_Expansion:TCA9535PWR", "TCA9535PWR", "Package_SO:TSSOP-24_4.4x7.8mm_P0.65mm",
        {"1": "NC", "2": "GND", "3": "GND", "21": "GND", "22": "SCL", "23": "SDA", "24": "+3V3", "12": "GND",
         "4": "NC", "5": "NC", "6": "NC", "7": "NC", "8": "NC", "9": "PWRD", "10": "RSTD", "11": "LED",
         "13": "DISARM", "14": "RUNOFF", "15": "PG", "16": "OVK", "17": "LCLR", "18": "LQ", "19": "PGD", "20": "OFF"},
        (80, 180), fields={"LCSC": "C130204"},
        desc="slot GPIO, address 0x20 (A0-A2 GND); pins by board side: P10 DISARM out, P05 PWRD out, P06 RSTD out, P07 LED in; P17 OFF out, P16 PGD in, P15 LQ in, P14 LCLR out, P13 OVK in, P12 PG in, P11 RUNOFF out; INT unused (polled)")
    C(9, "100n", C0402, {"1": "+3V3", "2": "GND"}, (92, 152), desc="TCA9535 VCC")
    sym("U4", "Analog_ADC:ADC101C021CIMK", "ADC121C027CIMK", "Package_TO_SOT_SMD:TSOT-23-6",
        {"1": "+3V3", "2": "GND", "3": "ID_F", "4": "GND", "5": "SCL", "6": "SDA"}, (40, 175),
        fields={"LCSC": "C2670157", "MPN": "TI ADC121C027CIMKX/NOPB"},
        desc="ID ADC, 12 bit, VA = reference (reads ID / 3V3), address 0x51 (ADR0 GND); symbol of the pin-equal ADC101C021CIMK, pin 4 = ADR0")
    C(6, "100n", C0402, {"1": "ID_F", "2": "GND"}, (26, 179), desc="ID filter at the ADC input")
    C(10, "100n", C0402, {"1": "+3V3", "2": "GND"}, (12, 164), desc="ADC VA (its reference)")
    C(11, "1u", C0402, {"1": "+3V3", "2": "GND"}, (22, 164), fields={"LCSC": "C52923"}, desc="ADC VA bulk")

    # ========================================================== wiring: power path
    wire("VBUS", (16, 32), ("R1", "1"), (30, 32), ("R2", "1"))      # one VBUS symbol, both shunts
    wire("SENSE", ("R1", "2"), ("Q1", "5"))
    wire("SENSE", (40, 32), ("R2", "2"))
    wire("SENSE", (40, 38), ("U2", "9"))                            # Kelvin tap to INA238 IN-
    wire("SENSE", ("U1", "1"), ("Q1", "5"))
    wire("GATE", ("Q1", "4"), ("U1", "10"))
    name("GATE", (98, 40))
    wire("OUT", ("Q1", "1"), ("SO1", "1"))
    wire("OUT", ("U1", "9"), ("Q1", "1"))
    wire("OUT", (102, 42), (50, 42), ("U2", "8"))                   # INA238 VBUS reads the slot output
    for x, ref, pin in ((112, "D1", "2"), (122, "C2", "1")):
        wire("OUT", (x, 32), (ref, pin))
    b.flag("OUT", P(108, 32))
    wire("NODE", ("SO4", "1"), ("J1", "2"))

    # ========================================================== LM5069 pins
    wire("OVLO", ("U1", "4"), ("R5", "2"), ("R6", "1"))
    wire("UVLO", ("U1", "3"), (84, 62), ("Q2", "3"))                # UVLO spine down to the enable chain
    wire("UVLO", (84, 72), ("R3", "2"), ("R4", "1"))
    wire("UVLO", (84, 80), ("Q3", "3"))
    name("OVLO", (82, 58))
    name("UVLO", (84, 70), 90)
    wire("PGD", ("U1", "8"), ("R8", "2"))
    tie("PGD", (128, 58))
    wire("PWR", ("U1", "7"), ("R7", "1"))
    wire("TIMER", ("U1", "6"), ("C1", "1"))

    # ========================================================== INA238 supply, alert, I2C segment
    wire("+3V3", (58, 56), ("C3", "1"), ("U2", "6"))
    wire("GND", (32, 64), ("U2", "7"))
    tie("OVK", ("U2", "3"))                                         # ALERT (open drain) sets the slot latch
    wire("SDA", ("U2", "4"), (44, 138), (12, 138))                  # segment along the bottom
    wire("SCL", ("U2", "5"), (42, 140), (12, 140))
    wire("SDA", ("R26", "2"), (20, 138))
    wire("SCL", ("R27", "2"), (26, 140))
    port("SDA", "bidirectional", "left", (12, 138))
    port("SCL", "bidirectional", "left", (12, 140))
    wire("SDA", (44, 138), (62, 138), (62, 164), ("U5", "23"))
    wire("SCL", (42, 140), (60, 140), (60, 166), ("U5", "22"))
    wire("SDA", ("U4", "6"), (62, 173), (62, 164))
    wire("SCL", ("U4", "5"), (60, 175), (60, 166))

    # ========================================================== enable chain, latch
    wire("OFF", ("Q2", "1"), (76, 90), ("R9", "1"))
    tie("OFF", (64, 90), 180)
    wire("LQ", ("U3", "5"), (94, 96), ("Q3", "1"))
    wire("LQ", (98, 96), ("R10", "1"))
    wire("LQ", (94, 96), (94, 104), (90, 104))
    tie("LQ", (90, 104), 180)
    wire("GND", (100, 84), ("Q3", "2"))
    wire("GND", (124, 108), ("U3", "1"), ("U3", "2"))
    wire("CLRN", ("U3", "6"), (114, 114), ("R11", "2"))            # latch /CLR node
    wire("CLRN", (128, 114), ("Q4", "3"))
    wire("CLRN", (122, 114), ("C5", "1"))
    wire("LCLR", ("Q4", "1"), (118, 127), ("R12", "1"))            # expander drives the Q4 gate
    wire("LCLR", (118, 127), (102, 127))
    tie("LCLR", (102, 127), 180)
    wire("OVK", ("J3", "1"), (162, 56), ("R13", "2"), (136, 56), (136, 80), (114, 80), ("U3", "7"))
    tie("OVK", (144, 56))

    # ========================================================== J3 module signals
    wire("PG", ("J3", "2"), ("R14", "2"))
    tie("PG", (152, 66), 180)
    wire("ID", ("J3", "3"), (154, 82), ("R15", "2"))
    wire("ID", (148, 82), ("R24", "2"))
    tie("ID_F", ("R24", "1"))
    wire("RUN", ("J3", "4"), (156, 98), ("Q5", "3"))
    wire("RUNOFF", ("Q5", "1"), (146, 104), ("R23", "1"))
    tie("RUNOFF", (140, 104), 180)

    # ========================================================== front panel
    wire("PWR_SW", ("J2", "1"), (217, 56), ("U6", "6"))
    name("PWR_SW", (217, 54), 90)
    wire("PWRD_A", ("R19", "2"), ("U6", "1"))
    tie("PWRD", (182, 56), 180)
    wire("PLED_P", ("J2", "3"), ("R21", "1"))
    name("PLED_P", (220, 44), 90)
    wire("PLED_A", ("R21", "2"), ("U8", "1"))
    wire("PLED_N", ("J2", "4"), (234, 54), ("U8", "2"))
    wire("RST_SW", ("J2", "2"), (237, 44), ("U7", "6"))
    name("RST_SW", (237, 43), 90)
    wire("RSTD_A", ("R20", "2"), ("U7", "1"))
    tie("RSTD", (270, 44))
    wire("LED", ("U8", "4"), (224, 72), ("R22", "2"))
    tie("LED", (208, 72), 180)

    # ========================================================== expander and ADC ends of the ties
    for pin in ("13", "9", "10", "11", "14", "15", "16", "17", "18", "19", "20"):
        b.tie(b.syms["U5"].nets[pin], ("U5", pin))
    wire("ID_F", ("U4", "3"), (26, 175), ("C6", "1"))
    tie("ID_F", (22, 175), 180)

    for ref, pins in (("U10", "567"), ("R28", "2"), ("Q8", "123"), ("Q9", "13"), ("Q6", "123"), ("Q7", "13"), ("R29", "1")):
        for pin in pins:
            b.tie(b.syms[ref].nets[pin], (ref, pin))

    b.text("COLUMN: 48 V HOT-SWAP, INA238, OV LATCH, MODULE MOUNT, TCA9535, ID ADC, NODE TERMINAL", P(12, 20))
    b.text("I2C SEGMENT: INA238 0x40, TCA9535 0x20, ADC121C027 0x51", P(12, 150), size=1.5)
    return b.finish()
