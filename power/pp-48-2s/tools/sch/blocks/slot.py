"""Slot sheet: one eighth-brick slot, drawn once, instantiated per slot.

Parts and pin -> net tables copied from gensch.py "# SLOTS" with the
"Sn_" prefix dropped and references sheet-local (R1.., the root adds
100 / 200 per instance). Per-slot differences leave the sheet as ports
and are tied at the root: A0 A1 (INA238 address), the expander signals
PWRD RSTD (in) PGD PG LQ LED (out), the MCU signals OFF LCLR (in) ID
TEMP (out), and the bus SDA2 SCL2. VBUS, +3V3 and GND are power
symbols; OUT and NODE are local nets with a PWR_FLAG each.

Flow: 48 V enters top left, through the shunt pair and the pass FET
along the top rail into the module standoffs SO1-SO4 (middle); the 19 V NODE leaves on
the right to the node connector J1. LM5069 under the FET with its
dividers on the left and timer / power / PGD parts on the right; INA238
under the shunt; enable chain and OV latch under the LM5069; the aux
pogo pins PG1-PG7 under the module; photoMOS and LED-sense opto around J1.

Coordinates are in grid units of 1.27 mm (P(x, y)).
"""
from sch.draw import Block, R0402, R0603, R0805, R1206, R2512, C0402, C0603, C1210, LED0603
from sch.kicad import by_name

G = 1.27
FET = "Transistor_FET:2N7002"
SOT23 = "Package_TO_SOT_SMD:SOT-23"
TLP176 = ("pp:TLP176AM", "TLP176AM", "Package_SO:SOIC-4_4.55x3.7mm_P2.54mm")


def P(x, y):
    return (x * G, y * G)


def block(N=2):
    b = Block("Slot")
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
        b.name(net, at=P(*at), ang=ang)     # b.name is the sheet name attribute

    def port(net, shape, side, at):
        b.port(net, shape, side=side, pos=P(*at))

    # ========================================================== hot-swap front end (top rail y = 32)
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
    R(5, "240k", R0805, {"1": "VBUS", "2": "OVLO"}, (76, 55), desc="OVLO: off 62.5 V, back at 57.5 V")
    R(6, "10k", R0402, {"1": "OVLO", "2": "GND"}, (76, 63))
    R(7, "13.7k", R0402, {"1": "PWR", "2": "GND"}, (116, 66),
      desc="power limit ~50 W at 60 V (RPWR = 1.3e5 x (RSNS x PLIM - 1.18mV x VDS))")
    C(1, "220n", C0603, {"1": "TIMER", "2": "GND"}, (108, 68), desc="insertion 160 ms, fault timeout 10 ms")
    R(8, "10k", R0402, {"1": "+3V3", "2": "PGD"}, (126, 52), desc="PGD pull-up")
    sym("D1", "Device:D_TVS", "SMBJ60A", "Diode_SMD:D_SMB", {"1": "GND", "2": "OUT"}, (112, 38), rot=90,
        fields={"LCSC": "C353345"}, desc="slot output clamp (cable inductance on a cut)")
    C(2, "2.2u 100V", C1210, {"1": "OUT", "2": "GND"}, (122, 38), fields={"LCSC": "C338133"},
      desc="slot output, in front of the brick")

    # ========================================================== INA238 under the shunt
    sym("U2", "Sensor_Energy:INA238", "INA238AIDGSR", "Package_SO:TSSOP-10_3x3mm_P0.5mm",
        {"1": "A1", "2": "A0", "3": "NC", "4": "SDA2", "5": "SCL2", "6": "+3V3", "7": "GND", "8": "OUT", "9": "SENSE", "10": "VBUS"},
        (44, 60), rot=270,
        desc="16-bit monitor on the shunt, address 0x40 + slot - 1 (A0 A1 tied at the root); VBUS pin reads the slot output")
    C(3, "100n", C0402, {"1": "+3V3", "2": "GND"}, (58, 63), desc="INA238 VS")

    # ========================================================== enable chain and OV latch
    sym("Q2", FET, "2N7002", SOT23, {"1": "OFF", "2": "GND", "3": "UVLO"}, (82, 90),
        fields={"LCSC": "C8545"}, desc="MCU OFF: high = slot off; MCU dead = pull-down = ON")
    R(9, "100k", R0402, {"1": "OFF", "2": "GND"}, (76, 95), desc="default ON")
    sym("Q3", FET, "2N7002", SOT23, {"1": "LQ", "2": "GND", "3": "UVLO"}, (94, 82), rot=90,
        fields={"LCSC": "C8545"}, desc="OV latch output cuts the slot")
    R(10, "100k", R0402, {"1": "LQ", "2": "GND"}, (98, 101))
    sym("U3", "74xGxx:74AUC1G74", "74LVC1G74", "Package_SO:VSSOP-8_2.3x2mm_P0.5mm",
        {"1": "GND", "2": "GND", "3": "NC", "4": "GND", "5": "LQ", "6": "CLRN", "7": "OVK", "8": "+3V3"},
        (112, 100), mirror="y", fields={"LCSC": "C70285"},
        desc="OV latch: /PRE = module OV_KILL (low = set), /CLR = RC power-on clear + MCU; Q holds UVLO low until cleared")
    C(4, "100n", C0402, {"1": "+3V3", "2": "GND"}, (104, 84), desc="latch VCC")
    R(11, "100k", R0402, {"1": "+3V3", "2": "CLRN"}, (128, 111), desc="power-on clear RC (10 ms)")
    C(5, "100n", C0402, {"1": "CLRN", "2": "GND"}, (122, 118))
    sym("Q4", FET, "2N7002", SOT23, {"1": "LCLR", "2": "GND", "3": "CLRN"}, (132, 127),
        fields={"LCSC": "C8545"}, desc="MCU clears the latch: drive high -> Q4 pulls /CLR low")
    R(12, "100k", R0402, {"1": "LCLR", "2": "GND"}, (118, 131), desc="Q4 gate pull-down: MCU dead = latch not cleared")

    # ========================================================== module mount: standoffs and pogo pins
    # one part per standoff / pogo pin (JLC places one part per designator), each drawn where
    # the old brick socket / aux header had that pin; board positions from
    # power/tools/pmmount.py host_parts()
    def single(ref, value, fp, net, pin, right, lcsc, desc):
        x, y = pin                                      # pin end in mm; the Conn_01x01 pin is 5.08 mm from its origin
        sym(ref, "Connector_Generic:Conn_01x01", value, fp, {"1": net},
            (round((x + (-5.08 if right else 5.08)) / G), round(y / G)), mirror="y" if right else None,
            fields={"LCSC": lcsc}, desc=desc)
    for ref, net, pin, right, what in (("SO1", "OUT", (193.04, 40.64), False, "screw 1, module VIN"),
                                       ("SO2", "GND", (193.04, 50.8), False, "screw 3, module GND (input end)"),
                                       ("SO3", "GND", (228.6, 55.88), True, "screw 4, module GND (output end)"),
                                       ("SO4", "NODE", (228.6, 40.64), True, "screw 8, module VOUT")):
        single(ref, "M2x6 standoff", "pp:PM_Standoff_M2x6_SMTSO", net, pin, right, "C2915628",
               "PM module mount: SMT M2 x 6 standoff, %s; carries the module current" % what)
    for i, (net, what) in enumerate((("OVK", "OV_KILL"), ("PG", "PGOOD"), ("ID", "ID"), ("TEMP", "TEMP"),
                                     ("GND", "GND"), ("SDA2", "SDA (reserved)"), ("SCL2", "SCL (reserved)"))):
        single("PG%d" % (i + 1), "pogo", "pp:PM_Pogo_YZ130115073P", net, (208.28, 81.28 + 2.54 * i), False,
               "C5157258", "PM module aux contact %d %s: pogo pin on the module underside pad%s"
               % (i + 1, what, "; a module monitor must use 0x48-0x4F" if i == 5 else ""))
    R(13, "10k", R0402, {"1": "+3V3", "2": "OVK"}, (150, 53), desc="OV_KILL pull-up (module comparator is open drain)")
    R(14, "10k", R0402, {"1": "+3V3", "2": "PG"}, (156, 63), desc="module PGOOD pull-up")
    R(15, "10k", R0402, {"1": "+3V3", "2": "ID"}, (148, 77), desc="ID divider top (module resistor to GND)")
    C(6, "100n", C0402, {"1": "ID", "2": "GND"}, (148, 87))
    R(16, "10k", R0402, {"1": "+3V3", "2": "TEMP"}, (140, 93), desc="NTC divider top")
    C(7, "100n", C0402, {"1": "TEMP", "2": "GND"}, (140, 103))
    C(8, "2.2u 50V", C1210, {"1": "NODE", "2": "GND"}, (192, 38), desc="node output, cable side")
    R(17, "47k", R1206, {"1": "OUT", "2": "ONL_A"}, (134, 38), desc="slot ON LED from 54 V (1.1 mA, 65 mW)")
    sym("D2", "Device:LED", "GREEN ON", LED0603, {"1": "GND", "2": "ONL_A"}, (134, 47), rot=90, fields={"LCSC": "C2297"})
    R(18, "10k", R0603, {"1": "NODE", "2": "NL_A"}, (204, 37), desc="node 19 V LED")
    sym("D3", "Device:LED", "GREEN NODE", LED0603, {"1": "GND", "2": "NL_A"}, (204, 46), rot=90, fields={"LCSC": "C2297"})

    # ========================================================== node connector and front panel
    sym("J1", "Connector_Generic:Conn_02x04_Odd_Even", "NODE", "Connector_Molex:Molex_Mini-Fit_Jr_5566-08A_2x04_P4.20mm_Vertical",
        {"1": "NODE", "2": "NODE", "3": "GND", "4": "GND", "5": "PWR_SW", "6": "RST_SW", "7": "PLED_P", "8": "PLED_N"},
        (226, 34), fields={"MPN": "Molex 39-28-1083"},
        desc="one cable per node: 1-2 +19 V, 3-4 GND, 5 PWR_SW, 6 RST_SW, 7 PLED+, 8 PLED-")
    sym("U4", *TLP176, {"1": "PWRD_A", "3": "GND", "4": "GND", "6": "PWR_SW"}, (208, 58),
        fields={"LCSC": "C5370540"}, desc="photoMOS front-panel PWR_SW contact")
    R(19, "270", R0402, {"1": "PWRD", "2": "PWRD_A"}, (192, 56), rot=90, fields={"LCSC": "C22775"},
      desc="photoMOS LED, ~6.5 mA from the expander")
    sym("U5", *TLP176, {"1": "RSTD_A", "3": "GND", "4": "GND", "6": "RST_SW"}, (246, 46), mirror="y",
        fields={"LCSC": "C5370540"}, desc="photoMOS front-panel RST_SW contact")
    R(20, "270", R0402, {"1": "RSTD", "2": "RSTD_A"}, (262, 44), rot=270, fields={"LCSC": "C22775"},
      desc="photoMOS LED, ~6.5 mA from the expander")
    sym("U6", "Isolator:TLP291", "TLP291", "Package_SO:SOIC-4_4.55x2.6mm_P1.27mm",
        {"1": "PLED_A", "2": "PLED_N", "3": "GND", "4": "LED"}, (226, 62), rot=270, mirror="y",
        fields={"LCSC": "C55144"}, desc="power LED sense opto: LED lit -> output low")
    R(21, "220", R0402, {"1": "PLED_P", "2": "PLED_A"}, (220, 48), desc="opto LED series (as rev C)")
    R(22, "47k", R0402, {"1": "+3V3", "2": "LED"}, (216, 69), desc="opto collector pull-up")

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
    for x, ref, pin in ((112, "D1", "2"), (122, "C2", "1"), (134, "R17", "1")):
        wire("OUT", (x, 32), (ref, pin))
    b.flag("OUT", P(108, 32))
    wire("ONL_A", ("R17", "2"), ("D2", "2"))
    name("ONL_A", (134, 43))
    wire("GND", (134, 52), ("D2", "1"))                             # GND below the LED arrows
    wire("NODE", ("SO4", "1"), ("J1", "1"))
    wire("NODE", (220, 32), (220, 28), (234, 28), ("J1", "2"))
    for x, ref in ((192, "C8"), (204, "R18")):
        wire("NODE", (x, 32), (ref, "1"))
    # no PWR_FLAG on NODE: SO4 (module VOUT) is a power output and drives it (a flag would be a power-output conflict)
    wire("NL_A", ("R18", "2"), ("D3", "2"))
    name("NL_A", (204, 42))
    wire("GND", (204, 50), ("D3", "1"))

    # ========================================================== LM5069 pins
    wire("OVLO", ("U1", "4"), ("R5", "2"), ("R6", "1"))
    wire("UVLO", ("U1", "3"), (84, 62), ("Q2", "3"))                # UVLO spine down to the enable chain
    wire("UVLO", (84, 72), ("R3", "2"), ("R4", "1"))
    wire("UVLO", (84, 80), ("Q3", "3"))
    name("OVLO", (82, 58))
    name("UVLO", (84, 70), 90)
    wire("PGD", ("U1", "8"), ("R8", "2"))
    port("PGD", "output", "right", (128, 58))
    wire("PWR", ("U1", "7"), ("R7", "1"))
    wire("TIMER", ("U1", "6"), ("C1", "1"))

    # ========================================================== INA238 supply and ports
    wire("+3V3", (58, 56), ("C3", "1"), ("U2", "6"))
    wire("GND", (32, 64), ("U2", "7"))
    port("A1", "input", "right", (54, 72))
    port("A0", "input", "right", (54, 74))
    wire("SDA2", ("PG6", "1"), (158, 138), ("U2", "4"))              # I2C along the bottom
    wire("SCL2", ("PG7", "1"), (160, 140), ("U2", "5"))
    wire("SDA2", (44, 138), (12, 138))
    wire("SCL2", (42, 140), (12, 140))
    port("SDA2", "bidirectional", "left", (12, 138))
    port("SCL2", "bidirectional", "left", (12, 140))

    # ========================================================== enable chain, latch
    wire("OFF", ("Q2", "1"), (76, 90), ("R9", "1"))
    port("OFF", "input", "left", (64, 90))
    wire("LQ", ("U3", "5"), (94, 96), ("Q3", "1"))
    wire("LQ", (98, 96), ("R10", "1"))
    wire("LQ", (94, 96), (94, 104), (90, 104))
    port("LQ", "output", "left", (90, 104))
    wire("GND", (100, 84), ("Q3", "2"))
    wire("GND", (124, 108), ("U3", "1"), ("U3", "2"))
    wire("CLRN", ("U3", "6"), (114, 114), ("R11", "2"))            # latch /CLR node
    wire("CLRN", (128, 114), ("Q4", "3"))
    wire("CLRN", (122, 114), ("C5", "1"))
    wire("LCLR", ("Q4", "1"), (118, 127), ("R12", "1"))            # MCU drives the Q4 gate
    wire("LCLR", (118, 127), (102, 127))
    port("LCLR", "input", "left", (102, 127))
    wire("OVK", ("PG1", "1"), (162, 56), ("R13", "2"), (136, 56), (136, 80), (114, 80), ("U3", "7"))

    # ========================================================== module aux signals
    wire("PG", ("PG2", "1"), ("R14", "2"))
    port("PG", "output", "left", (152, 66))
    wire("ID", ("PG3", "1"), (154, 82), ("R15", "2"))
    wire("ID", (148, 82), ("C6", "1"))
    port("ID", "output", "left", (144, 82))
    wire("TEMP", ("PG4", "1"), (156, 98), ("R16", "2"))
    wire("TEMP", (140, 98), ("C7", "1"))
    port("TEMP", "output", "left", (136, 98))

    # ========================================================== front panel
    wire("PWR_SW", ("J1", "5"), (217, 56), ("U4", "6"))
    name("PWR_SW", (217, 54), 90)
    wire("PWRD_A", ("R19", "2"), ("U4", "1"))
    port("PWRD", "input", "left", (182, 56))
    wire("PLED_P", ("J1", "7"), ("R21", "1"))
    name("PLED_P", (220, 44), 90)
    wire("PLED_A", ("R21", "2"), ("U6", "1"))
    wire("PLED_N", ("J1", "8"), (234, 54), ("U6", "2"))
    wire("RST_SW", ("J1", "6"), (237, 44), ("U5", "6"))
    name("RST_SW", (237, 43), 90)
    wire("RSTD_A", ("R20", "2"), ("U5", "1"))
    port("RSTD", "input", "right", (270, 44))
    wire("LED", ("U6", "4"), (224, 72), ("R22", "2"))
    port("LED", "output", "left", (208, 72))

    b.text("SLOT: 48 V HOT-SWAP, INA238, OV LATCH, BRICK, NODE CONNECTOR", P(12, 20))
    return b.finish()
