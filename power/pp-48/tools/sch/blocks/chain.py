"""Chain sheet: the JST PH 2.0 12-pin link between neighbouring plane boards
(docs/adr/0005-chained-plane-boards.md decision 4).

Signals: the rotating channel P0..P3 (SDA / SCL each), +3V3 from the core,
GND x 3 between the pairs. No VBUS.

Pinouts. Both headers are side entry (S12B-PH-K-S) on the F side; the core's
and a unit's right-hand header face right ("OUT"), a unit's left-hand
header faces left ("IN", the footprint turned 180 deg). The cable is flat
and straight: every wire runs straight across, so header pin n at one end
meets pin 13 - n at the other, and the IN pinout is the OUT pinout read
backwards:

    OUT  1 3V3   2 P0_SDA  3 P0_SCL  4 GND  5 P1_SDA  6 P1_SCL
         7 GND   8 P2_SDA  9 P2_SCL 10 GND 11 P3_SDA 12 P3_SCL
    IN   pin n = OUT pin 13 - n (1 P3_SCL ... 12 3V3)

Cable: two PHR-12 housings, 12 wires, wire k from pin k to pin 13 - k.

Core: one OUT header J301, Pk = mux channel k (ports UNITk+1_SDA / SCL, as
the Mux sheet names them). Unit: IN header J301 (left edge) and OUT header
J302 (strip, right edge); IN P0 is this unit's segment (ports SDA / SCL),
IN P1..P3 are wired to OUT P0..P2, OUT P3 is not connected.

Coordinates are in grid units of 1.27 mm (P(x, y)).
"""
from sch.draw import Block

G = 1.27
PH12 = "Connector_JST:JST_PH_S12B-PH-K_1x12_P2.00mm_Horizontal"
FIELDS = {"LCSC": "C157943", "MPN": "JST S12B-PH-K-S(LF)(SN)"}
OUT = {1: "+3V3", 2: "P0_SDA", 3: "P0_SCL", 4: "GND", 5: "P1_SDA", 6: "P1_SCL",
       7: "GND", 8: "P2_SDA", 9: "P2_SCL", 10: "GND", 11: "P3_SDA", 12: "P3_SCL"}
IN = {n: OUT[13 - n] for n in OUT}


def P(x, y):
    return (x * G, y * G)


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
        nets = {str(n): ("UNIT%d_%s" % (int(v[1]) + 1, v[3:]) if v.startswith("P") else v) for n, v in OUT.items()}
        b.sym("J301", "Connector_Generic:Conn_01x12", "CHAIN OUT", PH12, nets, at=P(60, 40), mirror="y",
              fields=FIELDS, desc="chain to unit 1 (right edge): Pk = mux channel k; OUT pinout, flat straight cable")
        for n, net in nets.items():
            if net.startswith("UNIT"):
                b.port(net, "bidirectional", side="left", pos=stub(b, "J301", n, net))
        b.text("CHAIN OUT  J301 JST PH 12, Pk = TCA9548A channel k (unit k + 1)", P(30, 30))
        return b.finish()

    # unit: own segment = IN P0; IN P1..P3 -> OUT P0..P2; OUT P3 open
    def in_net(v):
        if not v.startswith("P"): return v
        k = int(v[1])
        return v[3:] if k == 0 else "CH%d_%s" % (k, v[3:])

    def out_net(v):
        if not v.startswith("P"): return v
        k = int(v[1])
        return "NC" if k == 3 else "CH%d_%s" % (k + 1, v[3:])

    nin = {str(n): in_net(v) for n, v in IN.items()}
    nout = {str(n): out_net(v) for n, v in OUT.items()}
    b.sym("J301", "Connector_Generic:Conn_01x12", "CHAIN IN", PH12, nin, at=P(40, 40), mirror="y",
          fields=FIELDS, desc="chain from the core side (left edge, faces left): IN pinout; P0 = this unit's segment")
    b.sym("J302", "Connector_Generic:Conn_01x12", "CHAIN OUT", PH12, nout, at=P(100, 40),
          fields=FIELDS, desc="chain to the next unit (strip, right edge): OUT pinout; P0..P2 = IN P1..P3, P3 open")
    for n, net in nin.items():
        if net in ("SDA", "SCL"):
            b.port(net, "bidirectional", side="left", pos=stub(b, "J301", n, net))
        elif net.startswith("CH"):
            b.tie(net, stub(b, "J301", n, net))
    for n, net in nout.items():
        if net.startswith("CH"):
            b.tie(net, stub(b, "J302", n, net), 180)
    b.text("CHAIN  J301 IN (P0 = own segment), J302 OUT (IN P1..P3 -> OUT P0..P2, P3 open)", P(20, 30))
    return b.finish()
