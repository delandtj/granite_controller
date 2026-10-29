"""Core input sheet: XT30 -> TVS -> input caps -> VIN (docs/adr/0005-chained-plane-boards.md decision 6).

The core board's own small 48 V input: it feeds only the 3V3 buck and the
isolated 12 V module (Supplies sheet), which limit on their own, so there is
no hot-swap and no fuse (ADR 0003 decision 8: no fuses in the fluid).

- J1 Amass XT30PW-M (C431092), keyed (pin 1 -, pin 2 +), 15 A rated.
- D1 SMDJ60A bus TVS on VIN, as the units: 60 V standoff, breakdown
  >= 66.7 V, clamp 96.8 V.
- C1 47 uF / 100 V electrolytic: its ESR damps the cable ringing when the
  XT30 is plugged live (the ceramics alone would ring towards 2 x VIN);
  C2 2.2 uF HF.
- VIN_SNS: 220k / 10k + 100 nF to the MCU ADC (3.3 V at 75.9 V), as rev1.

Coordinates are in grid units of 1.27 mm (P(x, y)), as input.py.
"""
from sch.draw import Block, R0402, R0805, C0402, C1210

G = 1.27
OX, OY = 26, 16


def P(x, y):
    return ((x + OX) * G, (y + OY) * G)


def block():
    b = Block("Input", date="2026-10-29")
    b.power("VIN", "power:VBUS", value="VIN")

    def sym(ref, lib, val, fp, nets, at, rot=0, mirror=None, **kw):
        b.sym(ref, lib, val, fp, nets, at=P(*at), rot=rot, mirror=mirror, **kw)

    def wire(net, *pts):
        b.wire(net, [p if isinstance(p[0], str) else P(*p) for p in pts])

    sym("J1", "Connector_Generic:Conn_01x02", "XT30PW-M", "Connector_AMASS:AMASS_XT30PW-M_1x02_P2.50mm_Horizontal",
        {"1": "GND", "2": "VIN"}, (-8, 30), mirror="y", fields={"LCSC": "C431092", "MPN": "Amass XT30PW-M"},
        desc="core input, keyed (pin 1 -, pin 2 +), 15 A rated")
    sym("D1", "Device:D_Zener", "SMDJ60A", "Diode_SMD:D_SMC", {"1": "VIN", "2": "GND"}, (2, 38), rot=270,
        desc="bus TVS 3 kW, unidirectional: cathode (pad 1) on VIN; 60 V standoff, breakdown >= 66.7 V, clamp 96.8 V; LCSC code to check")
    sym("C1", "Device:C_Polarized", "47u 100V", "Capacitor_SMD:CP_Elec_10x10", {"1": "VIN", "2": "GND"}, (14, 38),
        desc="input bulk, damps the ringing of a live plug-in; LCSC code to check")
    sym("C2", "Device:C", "2.2u 100V", C1210, {"1": "VIN", "2": "GND"}, (24, 38), fields={"LCSC": "C338133"}, desc="input HF")
    sym("R15", "Device:R", "220k", R0805, {"1": "VIN", "2": "VIN_SNS"}, (20, 90), desc="VIN sense: 3.3 V at 75.9 V")
    sym("R16", "Device:R", "10k", R0402, {"1": "VIN_SNS", "2": "GND"}, (20, 100))
    sym("C17", "Device:C", "100n", C0402, {"1": "VIN_SNS", "2": "GND"}, (28, 100), desc="ADC filter")

    wire("VIN", ("J1", "2"), (2, 32), ("D1", "1"), (2, 32), (14, 32), ("C1", "1"), (14, 32), (24, 32), ("C2", "1"))
    wire("VIN_SNS", ("R15", "2"), (20, 95), ("R16", "1"))
    wire("VIN_SNS", (12, 95), (20, 95), (28, 95), ("C17", "1"))
    b.port("VIN_SNS", "output", side="left", pos=P(12, 95))
    b.text("CORE INPUT  J1 XT30 -> TVS -> C1 / C2 -> VIN (3V3 buck, 12 V module)", P(-14, -4))
    return b.finish()
