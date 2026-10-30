"""Chain sheet: the JST PH 2.0 14-pin link between neighbouring plane boards
(docs/adr/0005-chained-plane-boards.md decision 4, docs/press-deadline-line.md).

Signals: the rotating channel P0..P3 (SDA / SCL each), +3V3 from the core,
GND x 3 between the pairs, PRESS_EN (the press deadline, bused: one line
for all units) and EXP_RSTN (the units' TCA6424A /RESET, bused, open drain
from the core, pulled up by R30 on every unit). No VBUS.

Pinouts. Both headers are side entry (S14B-PH-K-S) on the B side; the core's
and a unit's right-hand header face right ("OUT"), a unit's left-hand
header faces left ("IN"). The cable is flat and straight: every wire runs
straight across, so header pin n at one end meets pin 15 - n at the other,
and the OUT pinout is the IN pinout read backwards:

    IN   1 3V3   2 P0_SDA  3 P0_SCL  4 GND  5 P1_SDA  6 P1_SCL  7 GND
         8 P2_SDA  9 P2_SCL 10 GND 11 P3_SDA 12 P3_SCL 13 PRESS_EN 14 EXP_RSTN
    OUT  pin n = IN pin 15 - n (1 EXP_RSTN ... 14 3V3)

On B the headers' handedness puts an IN header's pin 1 at the bottom
(body to the left) and an OUT header's pin 1 at the top: 3V3 and P0 sit at
the bottom end of every header, by the In2 channel; the two static lines
at the top end, PRESS_EN next to the P3 pair.

Cable: two PHR-14 housings, 14 wires, wire k from pin k to pin 15 - k.

Core: one OUT header J301, Pk = mux channel k (ports UNITk+1_SDA / SCL, as
the Mux sheet names them), EXP_RSTN from the MCU (port), PRESS_EN from the
deadline monostable U301 (74LVC1G123): every rising edge of PRESS_TRIG
(MCU PB12, a GPIO the press task toggles) retriggers it, Q stays high for ~1 s (R301 x C301)
after the last edge; a stuck-high, stuck-low or floating trigger (R302
pull-down) ends in PRESS_EN low. Unit: IN header J301 (left edge) and OUT
header J302 (strip, right edge); IN P0 is this unit's segment (ports SDA /
SCL), IN P1..P3 are wired to OUT P0..P2, OUT P3 is not connected; PRESS_EN
and EXP_RSTN pass straight through (ports to the Unit sheet).

Coordinates are in grid units of 1.27 mm (P(x, y)).
"""
from sch.draw import Block

G = 1.27
PH14 = "Connector_JST:JST_PH_S14B-PH-K_1x14_P2.00mm_Horizontal"
FIELDS = {"LCSC": "C157938", "MPN": "JST S14B-PH-K-S(LF)(SN)"}
IN = {1: "+3V3", 2: "P0_SDA", 3: "P0_SCL", 4: "GND", 5: "P1_SDA", 6: "P1_SCL",
      7: "GND", 8: "P2_SDA", 9: "P2_SCL", 10: "GND", 11: "P3_SDA", 12: "P3_SCL", 13: "PRESS_EN", 14: "EXP_RSTN"}
OUT = {n: IN[15 - n] for n in IN}
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
        b.sym("J301", "Connector_Generic:Conn_01x14", "CHAIN OUT", PH14, nets, at=P(60, 40), mirror="y",
              fields=FIELDS, desc="chain to unit 1 (right edge): Pk = mux channel k; OUT pinout, flat straight cable")
        for n, net in nets.items():
            if net.startswith("UNIT"):
                b.port(net, "bidirectional", side="left", pos=stub(b, "J301", n, net))
            elif net == "EXP_RSTN":
                b.port(net, "input", side="left", pos=stub(b, "J301", n, net))
            elif net == "PRESS_EN":
                b.tie(net, stub(b, "J301", n, net))
        deadline(b)
        b.text("CHAIN OUT  J301 JST PH 14, Pk = TCA9548A channel k (unit k + 1), PRESS_EN / EXP_RSTN bused", P(30, 30))
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
    b.sym("J301", "Connector_Generic:Conn_01x14", "CHAIN IN", PH14, nin, at=P(40, 40), mirror="y",
          fields=FIELDS, desc="chain from the core side (left edge, faces left): IN pinout; P0 = this unit's segment")
    b.sym("J302", "Connector_Generic:Conn_01x14", "CHAIN OUT", PH14, nout, at=P(100, 40),
          fields=FIELDS, desc="chain to the next unit (strip, right edge): OUT pinout; P0..P2 = IN P1..P3, P3 open")
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
          desc="press deadline: B rising edge (A low, /CLR high) retriggers, Q high for tw = R301 x C301 ~1 s")
    b.sym("R301", "Device:R", "100k", "Resistor_SMD:R_0402_1005Metric", {"1": "+3V3", "2": "RCEXT"}, at=P(140, 50),
          desc="Rext: tw ~ 1.0 x 100k x 10u = 1 s")
    b.sym("C301", "Device:C", "10u", "Capacitor_SMD:C_0805_2012Metric", {"1": "RCEXT", "2": "CEXT"}, at=P(150, 62),
          fields={"LCSC": "C15850"}, desc="Cext, X5R")
    b.sym("C302", "Device:C", "100n", "Capacitor_SMD:C_0402_1005Metric", {"1": "+3V3", "2": "GND"}, at=P(130, 80),
          desc="U301 VCC")
    b.sym("R303", "Device:R", "10k", "Resistor_SMD:R_0402_1005Metric", {"1": "+3V3", "2": "PRESS_CLRN"}, at=P(96, 84),
          desc="/CLR pull-up (spare: an MCU pin could clear the press at once)")
    b.sym("R302", "Device:R", "100k", "Resistor_SMD:R_0402_1005Metric", {"1": "PRESS_TRIG", "2": "GND"}, at=P(80, 72),
          desc="trigger pull-down: an MCU in reset (pin floating) gives no edges")
    for ref, pin, net in (("U301", "2", "PRESS_TRIG"), ("U301", "5", "PRESS_EN"), ("U301", "6", "CEXT"),
                          ("U301", "7", "RCEXT"), ("U301", "3", "PRESS_CLRN"), ("R303", "2", "PRESS_CLRN"), ("R301", "2", "RCEXT"), ("C301", "1", "RCEXT"), ("C301", "2", "CEXT"),
                          ("R302", "1", "PRESS_TRIG")):
        end = stub(b, ref, pin, net)
        if (ref, pin) == ("U301", "2"):
            b.port(net, "input", side="left", pos=end)
        else:
            b.tie(net, end)
