"""Input sheet: the tile input, XT90 -> TVS -> tile hot-swap -> VBUS (docs/adr/0003 decisions 8-10).

VIN is the bus at the connector, ahead of the hot-swap: it feeds the core
(3V3 buck, isolated 12 V aux; Supplies sheet), so the core stays up and
reports when the tile trips. VBUS (the In2 plane under the columns) is the
hot-swap's output.

- J1 Amass XT90PW-M, keyed (the only reverse-polarity protection, ADR 0003
  decision 8). Main contacts 30 A.
- D1 SMDJ60A bus TVS on VIN: 60 V standoff = top of the 48-60 V window,
  breakdown >= 66.7 V (above the 64.6 V OVLO), clamp 96.8 V at Ipp (under
  the 100 V parts). C1 bulk damps the cable ringing when the hot-swap opens.
- Hot-swap U7 LM5069-1 (latch-off after a fault; ADR 0003 decision 14 Q&A):
  shunt R1 || R2 = 2 mOhm, current limit 55 mV / 2 mOhm = 27.5 A
  (24.3-30.8 A over 48.5-61.5 mV), so a sustained overload above the XT90's
  30 A ends in a fault timeout; circuit breaker 105 mV = 52.5 A. Load: N = 4
  x ~265 W = ~22 A at 48 V. Q2 || Q3 BSC040N10NS5 with their own 10 Ohm gate
  resistors; power limit ~75 W (RPWR = 1.3e5 x (RSNS x PLIM - 1.18 mV x
  VDS) at 60 V), so one FET carries the start-up SOA alone (ADR 0003 risk);
  VBUS holds only the column front ends' small caps, the columns' own
  hot-swaps start later (their 160 ms insertion time). UVLO / OVLO and timer
  as the columns: on 38.3 V, off 64.6 V, insertion 160 ms, fault 10 ms.
- Reset of a latched tile: the MCU drives TILE_OFF high (Q4 pulls UVLO low)
  and releases it; R14 holds Q4 off while the MCU is in reset (tile ON).
  TILE_PGD: open-drain PGD with a 3V3 pull-up.
- U8 INA238 on the shunt, on the MCU's I2C2 at 0x40 (A0 A1 GND; the mux is
  0x70): IN+ VIN, IN- TSENSE, VBUS pin = tile output. ALERT -> TILE_AL
  (open drain, pull-up here; firmware sets the limit). Full scale 163.84 mV
  (ADCRANGE 0) = 82 A, 60 mV at 30 A.
- VIN_SNS: 220k / 10k + 100 nF to the MCU ADC (3.3 V at 75.9 V).

Coordinates are in grid units of 1.27 mm (P(x, y)), front end as the
column sheet's.
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


def block(N=2):
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
    sym("J1", "Connector_Generic_MountingPin:Conn_01x02_MountingPin", "XT90PW-M",
        "Connector_AMASS:AMASS_XT90PW-M_1x02_P10.90mm_Horizontal", {"1": "GND", "2": "VIN", "MP": "GND"}, (-8, 30),
        mirror="y", fields={"MPN": "Amass XT90PW-M"},
        desc="tile input, keyed (pin 1 -, pin 2 +), 30 A; retention pegs (MP) soldered to GND; LCSC code to check")
    sym("D1", "Device:D_Zener", "SMDJ60A", "Diode_SMD:D_SMC", {"1": "VIN", "2": "GND"}, (2, 38), rot=270,
        desc="bus TVS 3 kW, unidirectional: cathode (pad 1) on VIN; 60 V standoff, breakdown >= 66.7 V, clamp 96.8 V; LCSC code to check")
    sym("C1", "Device:C_Polarized", "47u 100V", "Capacitor_SMD:CP_Elec_10x10", {"1": "VIN", "2": "GND"}, (14, 38),
        desc="input bulk, damps cable ringing when the hot-swap opens")
    C(2, "2.2u 100V", C1210, {"1": "VIN", "2": "GND"}, (24, 38), fields={"LCSC": "C338133"}, desc="input HF")

    # ========================================================== hot-swap (as the column front end)
    R(1, "4m 2W", R2512, {"1": "VIN", "2": "TSENSE"}, (35, 32), rot=90,
      desc="shunt, two 4 mOhm in parallel = 2 mOhm: limit 27.5 A, breaker 52.5 A; LCSC code to check")
    R(2, "4m 2W", R2512, {"1": "VIN", "2": "TSENSE"}, (35, 38), rot=90, desc="shunt (parallel)")
    sym("Q2", PFET, "BSC040N10NS5", TDSON, by_name(PFET, {"G": "GATE_A", "S": "VBUS", "D": "TSENSE"}), (98, 34), rot=90,
        fields={"LCSC": "C534334"}, desc="100 V 4 mOhm pass FET; carries the start-up SOA alone (power limit 75 W, 10 ms)")
    sym("Q3", PFET, "BSC040N10NS5", TDSON, by_name(PFET, {"G": "GATE_B", "S": "VBUS", "D": "TSENSE"}), (98, 8), rot=90,
        fields={"LCSC": "C534334"}, desc="parallel pass FET, for conduction loss")
    R(10, "10", R0402, {"1": "GATE", "2": "GATE_A"}, (90, 41), rot=90, desc="gate resistor (parallel FETs)")
    R(11, "10", R0402, {"1": "GATE_B", "2": "GATE"}, (98, 17), desc="gate resistor (parallel FETs)")
    sym("U7", "Power_Management:LM5069MM-1", "LM5069MM-1", "Package_SO:MSOP-10_3x3mm_P0.5mm",
        {"1": "TSENSE", "2": "VIN", "3": "UVLO", "4": "OVLO", "5": "GND", "6": "TIMER", "7": "PWR", "8": "TILE_PGD", "9": "VBUS", "10": "GATE"},
        (96, 56), fields={"LCSC": "C486026"},
        desc="tile hot-swap, 55 mV current limit, power limit, LATCH-OFF (-1); LCSC stock to check")
    R(33, "143k", R0805, {"1": "VIN", "2": "UVLO"}, (68, 69), desc="UVLO: on 38.3 V, off 35.3 V (21 uA hysteresis)")
    R(34, "10k", R0402, {"1": "UVLO", "2": "GND"}, (68, 77))
    R(35, "124k", R0805, {"1": "VIN", "2": "OVLO"}, (76, 55), desc="OVLO: off 64.6 V, as the columns")
    R(36, "4.99k", R0402, {"1": "OVLO", "2": "GND"}, (76, 63))
    R(37, "10.2k", R0402, {"1": "PWR", "2": "GND"}, (116, 66),
      desc="power limit ~75 W at 60 V (RPWR = 1.3e5 x (RSNS x PLIM - 1.18mV x VDS))")
    C(3, "220n", C0603, {"1": "TIMER", "2": "GND"}, (108, 68), desc="insertion 160 ms, fault timeout 10 ms")
    R(38, "10k", R0402, {"1": "+3V3", "2": "TILE_PGD"}, (126, 52), desc="PGD pull-up")
    C(14, "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, (112, 38), fields={"LCSC": "C338133"}, desc="VBUS at the FETs")
    C(15, "2.2u 100V", C1210, {"1": "VBUS", "2": "GND"}, (122, 38), fields={"LCSC": "C338133"}, desc="VBUS at the FETs")

    # ========================================================== INA238 under the shunt
    sym("U8", "Sensor_Energy:INA238", "INA238AIDGSR", "Package_SO:TSSOP-10_3x3mm_P0.5mm",
        {"1": "GND", "2": "GND", "3": "TILE_AL", "4": "SDA2", "5": "SCL2", "6": "+3V3", "7": "GND", "8": "VBUS", "9": "TSENSE", "10": "VIN"},
        (44, 60), rot=270, desc="tile monitor on the shunt, I2C2 address 0x40 (A0 A1 GND); VBUS pin reads the tile output")
    C(16, "100n", C0402, {"1": "+3V3", "2": "GND"}, (58, 63), desc="INA238 VS")
    R(12, "10k", R0402, {"1": "+3V3", "2": "TILE_AL"}, (30, 52), desc="ALERT pull-up")

    # ========================================================== MCU controls: TILE_OFF, VIN_SNS
    sym("Q4", FET, "2N7002", SOT23, {"1": "TILE_OFF", "2": "GND", "3": "UVLO"}, (82, 90),
        fields={"LCSC": "C8545"}, desc="MCU TILE_OFF high = tile off (and resets the latched LM5069-1)")
    R(14, "100k", R0402, {"1": "TILE_OFF", "2": "GND"}, (76, 95), desc="MCU in reset = tile ON")
    R(15, "220k", R0805, {"1": "VIN", "2": "VIN_SNS"}, (20, 90), desc="VIN sense: 3.3 V at 75.9 V")
    R(16, "10k", R0402, {"1": "VIN_SNS", "2": "GND"}, (20, 100))
    C(17, "100n", C0402, {"1": "VIN_SNS", "2": "GND"}, (28, 100), desc="ADC filter")

    # ========================================================== wiring
    wire("VIN", ("J1", "2"), (2, 32), ("D1", "1"), (2, 32), (14, 32), ("C1", "1"), (14, 32), (24, 32), ("C2", "1"),
         (24, 32), (30, 32), ("R1", "1"), (30, 32), ("R2", "1"))
    wire("TSENSE", ("R1", "2"), ("Q2", "5"))
    wire("TSENSE", (40, 32), ("R2", "2"))
    wire("TSENSE", (40, 38), ("U8", "9"))                          # Kelvin tap to INA238 IN-
    tie("TSENSE", ("U7", "1"))                                      # by label: the gate labels need x = 94-98
    tie("TSENSE", (70, 32))
    wire("TSENSE", (40, 32), (40, 6), ("Q3", "5"))
    wire("VBUS", (130, 32), (122, 32), ("C15", "1"), (122, 32), (112, 32), ("C14", "1"), (112, 32),
         ("Q2", "1"), ("Q2", "2"), ("Q2", "3"), ("U7", "9"))          # one VBUS symbol at the right end
    wire("VBUS", ("Q3", "1"), ("Q3", "2"), ("Q3", "3"))             # Q3 source: its own VBUS symbol
    wire("GATE_A", ("Q2", "4"), (98, 41), ("R10", "2"))
    wire("GATE_B", ("Q3", "4"), ("R11", "1"))
    tie("GATE", ("U7", "10"))                                       # gate drive to both resistors by label
    tie("GATE", ("R10", "1"))
    tie("GATE", ("R11", "2"))
    wire("OVLO", ("U7", "4"), ("R35", "2"), ("R36", "1"))
    wire("UVLO", ("U7", "3"), (84, 62), ("Q4", "3"))
    wire("UVLO", (84, 72), ("R33", "2"), ("R34", "1"))
    b.name("GATE_A", at=False)                                      # short stubs, no label room
    b.name("GATE_B", at=False)
    name("OVLO", (82, 58))
    name("UVLO", (84, 70), 90)
    wire("TILE_PGD", ("U7", "8"), ("R38", "2"))
    wire("TILE_PGD", (126, 58), (134, 58))
    wire("PWR", ("U7", "7"), ("R37", "1"))
    wire("TIMER", ("U7", "6"), ("C3", "1"))
    wire("+3V3", (58, 56), ("C16", "1"), ("U8", "6"))
    wire("TILE_AL", ("U8", "3"), (38, 70), (30, 70), ("R12", "2"))
    wire("TILE_AL", (30, 70), (12, 70))
    wire("SDA2", ("U8", "4"), (44, 74), (12, 74))
    wire("SCL2", ("U8", "5"), (42, 76), (12, 76))
    wire("TILE_OFF", ("Q4", "1"), (76, 90), ("R14", "1"))
    wire("TILE_OFF", (76, 90), (64, 90))
    wire("VIN_SNS", ("R15", "2"), (20, 95), ("R16", "1"))
    wire("VIN_SNS", (12, 95), (20, 95), (28, 95), ("C17", "1"))

    b.port("SDA2", "bidirectional", side="left", pos=P(12, 74))
    b.port("SCL2", "bidirectional", side="left", pos=P(12, 76))
    b.port("TILE_AL", "output", side="left", pos=P(12, 70))
    b.port("TILE_OFF", "input", side="left", pos=P(64, 90))
    b.port("VIN_SNS", "output", side="left", pos=P(12, 95))
    b.port("TILE_PGD", "output", side="right", pos=P(134, 58))
    b.text("TILE INPUT  J1 XT90 -> TVS -> U7 LM5069-1 (2 mOhm, Q2 || Q3) -> VBUS; U8 INA238 0x40 on I2C2", P(-14, -4))
    return b.finish()
