"""Unit input sheet: XT60 -> TVS -> unit hot-swap -> VBUS, in the unit board's
input strip (docs/adr/0005-chained-plane-boards.md decisions 2 and 3).

The rev1 tile input (input.py at tag pp48-rev1) scaled to one 2-slot unit:
2 x ~265 W = 530 W, 14 A at 38 V. VIN is the bus at the connector, VBUS the
hot-swap output (this board's In2 plane under both slots' shunts).
References are the drawing's + 200.

- J201 Amass XT60PW-M (C98732), keyed (pin 1 -, pin 2 +); the only
  reverse-polarity protection (ADR 0003 decision 8). Retention pegs are
  unnumbered pads, no net.
- D201 SMDJ60A bus TVS on VIN, as the rev1 core: 60 V standoff, breakdown
  >= 66.7 V (above the 64.6 V OVLO), clamp 96.8 V. C201 bulk damps the
  cable ringing when the hot-swap opens.
- U207 LM5069-1 (latch-off): shunt R201 || R202 = 3 mOhm, current limit
  55 mV / 3 mOhm = 18.3 A (16.2-20.5 A over 48.5-61.5 mV), circuit breaker
  105 mV = 35 A. Q202 || Q203 BSC040N10NS5 with 10 Ohm gate resistors;
  power limit ~75 W at 60 V (RPWR = 1.3e5 x (RSNS x PLIM - 1.18 mV x VDS)
  = 20.0k), insertion 160 ms, fault 10 ms, UVLO / OVLO as the slots. VBUS
  holds only the slot front ends' small caps when this stage starts; the
  slots' own hot-swaps start after it (their 160 ms insertion), so the
  unit FETs are fully on while the slots charge their modules.
- Reset of a latched unit: the MCU drives U_OFF high over the unit's
  TCA6424A (P13), Q204 pulls UVLO low; R214 holds Q204 off while the
  expander is in reset or unpowered (unit ON). U_PGD (P27): open-drain PGD
  with a 3V3 pull-up.
- U208 INA238 on the shunt, on the unit segment at 0x44 (A1 = VS, A0 =
  GND): IN+ VIN, IN- USENSE, VBUS pin = unit output. ALERT is not wired
  (polled, DIAG_ALRT).

Coordinates are in grid units of 1.27 mm (P(x, y)), as input.py.
"""
from sch.draw import Block, R0402, R0805, R2512, C0402, C0603, C1210
from sch.kicad import by_name

G = 1.27
FET = "Transistor_FET:2N7002"
SOT23 = "Package_TO_SOT_SMD:SOT-23"
PFET = "Transistor_FET:BSC040N10NS5"
TDSON = "Package_TO_SOT_SMD:TDSON-8-1"


OX, OY = 26, 16                    # sheet offset (grid units): the layout below starts left of / above 0


def P(x, y):
    return ((x + OX) * G, (y + OY) * G)


def block():
    b = Block("Input", date="2026-10-29")
    b.power("VIN", "power:VBUS", value="VIN")
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

    # ========================================================== connector, TVS, bulk (VIN rail y = 32)
    sym("J201", "Connector_Generic:Conn_01x02", "XT60PW-M", "pp48:AMASS_XT60PW-M_1x02_P7.20mm_Horizontal",
        {"1": "GND", "2": "VIN"}, (-8, 30), mirror="y", fields={"LCSC": "C98732", "MPN": "Amass XT60PW-M"},
        desc="unit input, keyed (pin 1 -, pin 2 +), 30 A rated; cable from the top edge")
    sym("D201", "Device:D_Zener", "SMDJ60A", "Diode_SMD:D_SMC", {"1": "VIN", "2": "GND"}, (2, 38), rot=270,
        fields={"LCSC": "C2990209"}, desc="bus TVS 3 kW, unidirectional: cathode (pad 1) on VIN; 60 V standoff, breakdown >= 66.7 V, clamp 96.8 V")
    sym("C201", "Device:C_Polarized", "47u 100V", "Capacitor_SMD:CP_Elec_10x10", {"1": "VIN", "2": "GND"}, (14, 38),
        fields={"LCSC": "C87862", "MPN": "Lelon RVT2A470M1010"}, desc="input bulk, damps cable ringing when the hot-swap opens")
    C(202, "2.2u 100V", C1210, {"1": "VIN", "2": "GND"}, (24, 38), fields={"LCSC": "C338133"}, desc="input HF")

    # ========================================================== hot-swap (as the slot front end)
    R(201, "6m 2W", R2512, {"1": "VIN", "2": "USENSE"}, (35, 32), rot=90, fields={"LCSC": "C2692028"},
      desc="shunt, two 6 mOhm in parallel = 3 mOhm: limit 18.3 A, breaker 35 A")
    R(202, "6m 2W", R2512, {"1": "VIN", "2": "USENSE"}, (35, 38), rot=90, fields={"LCSC": "C2692028"}, desc="shunt (parallel)")
    sym("Q202", PFET, "BSC040N10NS5", TDSON, by_name(PFET, {"G": "GATE_A", "S": "VBUS", "D": "USENSE"}), (98, 34), rot=90,
        fields={"LCSC": "C534334"}, desc="100 V 4 mOhm pass FET; carries the start-up SOA alone (power limit 75 W, 10 ms)")
    sym("Q203", PFET, "BSC040N10NS5", TDSON, by_name(PFET, {"G": "GATE_B", "S": "VBUS", "D": "USENSE"}), (98, 8), rot=90,
        fields={"LCSC": "C534334"}, desc="parallel pass FET, for conduction loss")
    R(210, "10", R0402, {"1": "GATE", "2": "GATE_A"}, (90, 41), rot=90, desc="gate resistor (parallel FETs)")
    R(211, "10", R0402, {"1": "GATE_B", "2": "GATE"}, (98, 17), desc="gate resistor (parallel FETs)")
    sym("U207", "Power_Management:LM5069MM-1", "LM5069MM-1", "Package_SO:MSOP-10_3x3mm_P0.5mm",
        {"1": "USENSE", "2": "VIN", "3": "UVLO", "4": "OVLO", "5": "GND", "6": "TIMER", "7": "PWR", "8": "U_PGD", "9": "VBUS", "10": "GATE"},
        (96, 56), fields={"LCSC": "C486026"},
        desc="unit hot-swap, 55 mV current limit, power limit, LATCH-OFF (-1); LCSC stock 1 on 2026-10-29 (LM5069MM-2 C111822 has 7k)")
    R(233, "143k", R0805, {"1": "VIN", "2": "UVLO"}, (68, 69), desc="UVLO: on 38.3 V, off 35.3 V (21 uA hysteresis)")
    R(234, "10k", R0402, {"1": "UVLO", "2": "GND"}, (68, 77))
    R(235, "124k", R0805, {"1": "VIN", "2": "OVLO"}, (76, 55), desc="OVLO: off 64.6 V, as the slots")
    R(236, "4.99k", R0402, {"1": "OVLO", "2": "GND"}, (76, 63))
    R(237, "20.0k", R0402, {"1": "PWR", "2": "GND"}, (116, 66),
      desc="power limit ~75 W at 60 V (RPWR = 1.3e5 x (RSNS x PLIM - 1.18mV x VDS), RSNS 3 mOhm)")
    C(203, "220n", C0603, {"1": "TIMER", "2": "GND"}, (108, 68), desc="insertion 160 ms, fault timeout 10 ms")
    R(238, "10k", R0402, {"1": "+3V3", "2": "U_PGD"}, (126, 52), desc="PGD pull-up (3V3 from the chain)")
    C(214, "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, (112, 38), fields={"LCSC": "C338133"}, desc="VBUS at the FETs")
    C(215, "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, (122, 38), fields={"LCSC": "C338133"}, desc="VBUS at the FETs")

    # ========================================================== INA238 under the shunt
    sym("U208", "Sensor_Energy:INA238", "INA238AIDGSR", "Package_SO:TSSOP-10_3x3mm_P0.5mm",
        {"1": "+3V3", "2": "GND", "3": "NC", "4": "SDA", "5": "SCL", "6": "+3V3", "7": "GND", "8": "VBUS", "9": "USENSE", "10": "VIN"},
        (44, 60), rot=270, fields={"LCSC": "C2868250", "MPN": "TI INA238AIDGSR"}, desc="unit monitor on the shunt, segment address 0x44 (A1 VS, A0 GND); VBUS pin reads the unit output; ALERT unused")
    C(216, "100n", C0402, {"1": "+3V3", "2": "GND"}, (58, 63), desc="INA238 VS")

    # ========================================================== expander control: U_OFF
    sym("Q204", FET, "2N7002", SOT23, {"1": "U_OFF", "2": "GND", "3": "UVLO"}, (82, 90),
        fields={"LCSC": "C8545"}, desc="expander U_OFF (P13) high = unit off (and resets the latched LM5069-1)")
    R(214, "100k", R0402, {"1": "U_OFF", "2": "GND"}, (76, 95), desc="expander in reset or no 3V3 = unit ON")

    # ========================================================== wiring
    wire("VIN", ("J201", "2"), (2, 32), ("D201", "1"), (2, 32), (14, 32), ("C201", "1"), (14, 32), (24, 32), ("C202", "1"),
         (24, 32), (30, 32), ("R201", "1"), (30, 32), ("R202", "1"))
    wire("USENSE", ("R201", "2"), ("Q202", "5"))
    wire("USENSE", (40, 32), ("R202", "2"))
    wire("USENSE", (40, 38), ("U208", "9"))                        # Kelvin tap to INA238 IN-
    tie("USENSE", ("U207", "1"))                                    # by label: the gate labels need x = 94-98
    tie("USENSE", (70, 32))
    wire("USENSE", (40, 32), (40, 6), ("Q203", "5"))
    wire("VBUS", (130, 32), (122, 32), ("C215", "1"), (122, 32), (112, 32), ("C214", "1"), (112, 32),
         ("Q202", "1"), ("Q202", "2"), ("Q202", "3"), ("U207", "9"))  # one VBUS symbol at the right end
    wire("VBUS", ("Q203", "1"), ("Q203", "2"), ("Q203", "3"))       # Q203 source: its own VBUS symbol
    wire("GATE_A", ("Q202", "4"), (98, 41), ("R210", "2"))
    wire("GATE_B", ("Q203", "4"), ("R211", "1"))
    tie("GATE", ("U207", "10"))                                     # gate drive to both resistors by label
    tie("GATE", ("R210", "1"))
    tie("GATE", ("R211", "2"))
    wire("OVLO", ("U207", "4"), ("R235", "2"), ("R236", "1"))
    wire("UVLO", ("U207", "3"), (84, 62), ("Q204", "3"))
    wire("UVLO", (84, 72), ("R233", "2"), ("R234", "1"))
    b.name("GATE_A", at=False)                                      # short stubs, no label room
    b.name("GATE_B", at=False)
    name("OVLO", (82, 58))
    name("UVLO", (84, 70), 90)
    wire("U_PGD", ("U207", "8"), ("R238", "2"))
    wire("U_PGD", (126, 58), (134, 58))
    wire("PWR", ("U207", "7"), ("R237", "1"))
    wire("TIMER", ("U207", "6"), ("C203", "1"))
    wire("+3V3", (58, 56), ("C216", "1"), ("U208", "6"))
    wire("SDA", ("U208", "4"), (44, 74), (12, 74))
    wire("SCL", ("U208", "5"), (42, 76), (12, 76))
    wire("U_OFF", ("Q204", "1"), (76, 90), ("R214", "1"))
    wire("U_OFF", (76, 90), (64, 90))

    b.port("SDA", "bidirectional", side="left", pos=P(12, 74))
    b.port("SCL", "bidirectional", side="left", pos=P(12, 76))
    b.port("U_OFF", "input", side="left", pos=P(64, 90))
    b.port("U_PGD", "output", side="right", pos=P(134, 58))
    b.text("UNIT INPUT  J201 XT60 -> TVS -> U207 LM5069-1 (3 mOhm, Q202 || Q203) -> VBUS; U208 INA238 0x44", P(-14, -4))
    return b.finish()
