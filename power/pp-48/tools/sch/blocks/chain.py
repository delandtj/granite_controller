"""Chain sheet: the board-to-board link between neighbouring plane boards, a
right-angle 2x7 2.54 mm plug / socket pair that mates directly in the plane
of the boards, no cable (docs/adr/0005-chained-plane-boards.md decision 4,
docs/press-deadline-line.md).

Signals: the rotating channel P0..P3 (SDA / SCL each), +3V3 from the core,
GND x 3, PRESS_EN (the press deadline, bused: one line for all units) and
EXP_RSTN (the units' TCA6424A /RESET, bused, open drain from the core,
pulled up by R30 on every unit). No VBUS.

Parts (CJT A2541 series, gold over nickel, one series so the rows mate at
the same height): "IN" (a unit's left edge) is the plug, A2541WR-2x7P
(C5136987), body flush with the edge, pins 6 mm out past it; "OUT" (the
core's and a unit's right edge) is the socket, A2541HWR-2x7P (C19100281),
its face 4 mm out past the edge. Mated fully the boards stand 4 mm apart;
4..6 mm gap = 6..4 mm of the pins inside. Both on B with pin 1 at the top
and the even pins in the row nearer the edge, so plug pin n meets socket
pin n and IN and OUT share one pinout (top to bottom, odd / even row):

     1 EXP_RSTN   2 PRESS_EN
     3 P3_SDA     4 P3_SCL
     5 P2_SDA     6 P2_SCL
     7 GND        8 GND
     9 P1_SDA    10 P1_SCL
    11 P0_SDA    12 P0_SCL
    13 +3V3      14 GND

3V3 and P0 at the bottom, by the unit's In2 channel; the static lines at
the top.

Core: one OUT socket J301, Pk = mux channel k (ports UNITk+1_SDA / SCL, as
the Mux sheet names them), EXP_RSTN from the MCU (port), PRESS_EN from the
deadline monostable U301 (74LVC1G123): every rising edge of PRESS_TRIG
(MCU PB12, a GPIO the press task toggles) retriggers it, Q stays high for ~1 s (R301 x C301)
after the last edge; a stuck-high, stuck-low or floating trigger (R302
pull-down) ends in PRESS_EN low. Unit: IN plug J301 (left edge) and OUT
socket J302 (strip, right edge); IN P0 is this unit's segment (ports SDA /
SCL), IN P1..P3 are wired to OUT P0..P2, OUT P3 is not connected; PRESS_EN
and EXP_RSTN pass straight through (ports to the Unit sheet).

Coordinates are in grid units of 1.27 mm (P(x, y)).
"""
from sch.draw import Block

G = 1.27
SYM = "Connector_Generic:Conn_02x07_Odd_Even"
PLUG = "Connector_PinHeader_2.54mm:PinHeader_2x07_P2.54mm_Horizontal"
SOCKET = "Connector_PinSocket_2.54mm:PinSocket_2x07_P2.54mm_Horizontal"
PLUG_FIELDS = {"LCSC": "C5136987", "MPN": "CJT A2541WR-2x7P"}
SOCKET_FIELDS = {"LCSC": "C19100281", "MPN": "CJT A2541HWR-2x7P"}
IN = {1: "EXP_RSTN", 2: "PRESS_EN", 3: "P3_SDA", 4: "P3_SCL", 5: "P2_SDA", 6: "P2_SCL", 7: "GND",
      8: "GND", 9: "P1_SDA", 10: "P1_SCL", 11: "P0_SDA", 12: "P0_SCL", 13: "+3V3", 14: "GND"}
OUT = IN                              # plug pin n mates socket pin n
STATIC = {"PRESS_EN": "input", "EXP_RSTN": "input"}      # unit side: port shapes


def P(x, y):
    return (x * G, y * G)


def pair(v):
    """P0_SDA .. P3_SCL (not PRESS_EN)"""
    return len(v) == 6 and v[0] == "P" and v[1].isdigit()


STUB = 10 * G                         # labels clear of the pin-end power symbols


def stub(b, ref, pin, net):
    """wire from the pin straight out; returns the far end for the label / port"""
    s = b.syms[ref]
    (x, y), (dx, dy) = s.pin_xy(pin), s.pin_out(pin)
    end = (x + dx * STUB, y + dy * STUB)
    b.wire(net, [(ref, pin), end])
    return end


def block(kind):
    b = Block("Chain", date="2026-10-29")
    if kind == "core":
        nets = {str(n): ("UNIT%d_%s" % (int(v[1]) + 1, v[3:]) if pair(v) else v) for n, v in OUT.items()}
        b.sym("J301", SYM, "CHAIN OUT", SOCKET, nets, at=P(60, 40), mirror="y",
              fields=SOCKET_FIELDS, desc="chain to unit 1 (right edge socket): Pk = mux channel k",
              ref_at=(-1.27, -13.97), val_at=(-1.27, -11.43))
        for n, net in nets.items():
            ang = 0 if b.syms["J301"].pin_out(n) == (1, 0) else None     # odd row: the label reads outward
            if net.startswith("UNIT"):
                b.port(net, "bidirectional", side="left", pos=stub(b, "J301", n, net), ang=ang)
            elif net == "EXP_RSTN":
                b.port(net, "input", side="left", pos=stub(b, "J301", n, net), ang=ang)
            elif net == "PRESS_EN":
                b.tie(net, stub(b, "J301", n, net))
        deadline(b)
        b.text("CHAIN OUT  J301 2x7 socket, Pk = TCA9548A channel k (unit k + 1), PRESS_EN / EXP_RSTN bused", P(30, 25))
        return b.finish()

    # unit: own segment = IN P0; IN P1..P3 -> OUT P0..P2; OUT P3 open
    def in_net(v):
        if not pair(v): return v
        k = int(v[1])
        return v[3:] if k == 0 else "CH%d_%s" % (k, v[3:])

    def out_net(v):
        if not pair(v): return v
        k = int(v[1])
        return "NC" if k == 3 else "CH%d_%s" % (k + 1, v[3:])

    nin = {str(n): in_net(v) for n, v in IN.items()}
    nout = {str(n): out_net(v) for n, v in OUT.items()}
    b.sym("J301", SYM, "CHAIN IN", PLUG, nin, at=P(40, 40), mirror="y",
          fields=PLUG_FIELDS, desc="chain from the core side (left edge plug): P0 = this unit's segment")
    b.sym("J302", SYM, "CHAIN OUT", SOCKET, nout, at=P(100, 40),
          fields=SOCKET_FIELDS, desc="chain to the next unit (right edge socket): P0..P2 = IN P1..P3, P3 open")
    for n, net in nin.items():
        if net in STATIC:
            b.port(net, STATIC[net], side="left", pos=stub(b, "J301", n, net))
        elif net in ("SDA", "SCL"):
            b.port(net, "bidirectional", side="left", pos=stub(b, "J301", n, net))
        elif net.startswith("CH"):
            b.tie(net, stub(b, "J301", n, net))
    for n, net in nout.items():
        if net.startswith("CH") or net in STATIC:
            b.tie(net, stub(b, "J302", n, net), 180)
    b.text("CHAIN  J301 IN (P0 = own segment), J302 OUT (IN P1..P3 -> OUT P0..P2, P3 open; PRESS_EN, EXP_RSTN through)", P(20, 30))
    return b.finish()


def deadline(b):
    """core: PRESS_EN = 74LVC1G123 Q, retriggered by PRESS_TRIG rising edges (B), ~1 s after the last"""
    b.sym("U301", "pp:74LVC1G123", "74LVC1G123", "Package_SO:SSOP-8_2.95x2.8mm_P0.65mm",
          {"1": "GND", "2": "PRESS_TRIG", "3": "PRESS_CLRN", "4": "GND", "5": "PRESS_EN", "6": "CEXT", "7": "RCEXT", "8": "+3V3"},
          at=P(110, 60), fields={"LCSC": "C123302", "MPN": "TI SN74LVC1G123DCTR"},
          desc="press deadline: B rising edge (A low, /CLR high) retriggers, Q high for tw = R301 x C301 ~1 s",
          ref_at=(2.54, -15.24, "left"), val_at=(2.54, -12.7, "left"))
    b.sym("R301", "Device:R", "100k", "Resistor_SMD:R_0402_1005Metric", {"1": "+3V3", "2": "RCEXT"}, at=P(88, 49),
          desc="Rext: tw ~ 1.0 x 100k x 10u = 1 s")
    b.sym("C301", "Device:C", "10u", "Capacitor_SMD:C_0805_2012Metric", {"1": "RCEXT", "2": "CEXT"}, at=P(84, 57),
          fields={"LCSC": "C15850"}, desc="Cext, X5R", ref_at=(-2.54, -1.27, "right"), val_at=(-2.54, 1.27, "right"))
    b.sym("C302", "Device:C", "100n", "Capacitor_SMD:C_0402_1005Metric", {"1": "+3V3", "2": "GND"}, at=P(124, 47),
          desc="U301 VCC")
    b.sym("R303", "Device:R", "10k", "Resistor_SMD:R_0402_1005Metric", {"1": "+3V3", "2": "PRESS_CLRN"}, at=P(98, 77),
          desc="/CLR pull-up (spare: an MCU pin could clear the press at once)",
          ref_at=(-2.54, -1.27, "right"), val_at=(-2.54, 1.27, "right"))
    b.sym("R302", "Device:R", "100k", "Resistor_SMD:R_0402_1005Metric", {"1": "PRESS_TRIG", "2": "GND"}, at=P(82, 67),
          desc="trigger pull-down: an MCU in reset (pin floating) gives no edges")
    b.wire("+3V3", [P(110, 42), P(110, 44), P(124, 44), ("C302", "1"), P(124, 44), P(110, 44), ("U301", "8")])   # C302 at VCC
    b.wire("RCEXT", [("U301", "7"), P(92, 56), P(92, 54), P(88, 54), ("C301", "1")])
    b.wire("RCEXT", [P(88, 54), ("R301", "2")])
    b.name("RCEXT", at=P(94, 56))
    b.wire("CEXT", [("U301", "6"), P(94, 58), P(94, 60), ("C301", "2")])
    b.name("CEXT", at=P(86, 60))
    b.wire("PRESS_CLRN", [("U301", "3"), P(110, 80), ("R303", "2")])
    b.name("PRESS_CLRN", at=P(100, 80))
    b.wire("PRESS_TRIG", [P(76, 64), ("R302", "1"), ("U301", "2")])
    b.port("PRESS_TRIG", "input", side="left", pos=P(76, 64))
    b.tie("PRESS_EN", stub(b, "U301", "5", "PRESS_EN"))

