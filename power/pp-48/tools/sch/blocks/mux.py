"""Mux sheet: TCA9548A, one channel per 2-slot unit (docs/adr/0004 decisions 6, 9).

Channel k = unit k + 1 (UNITk+1_SDA / UNITk+1_SCL, the In2 channel to the
units; the unit has its own segment pull-ups). Channels with no unit on
this plane size (always 4-7) are left open. Upstream on the MCU's I2C2 (SDA2 / SCL2,
pull-ups on the MCU sheet). Address 0x70 (A0-A2 GND). /RESET: 10k pull-up,
MUX_RST from the MCU pulls it low (all channels off); an MCU in reset leaves
the mux running.
"""
from sch.draw import Block, R0402, C0402

G = 1.27


def P(x, y):
    return (x * G, y * G)


def block(N=2):
    b = Block("Mux")
    nets = {"1": "GND", "2": "GND", "21": "GND", "3": "MUX_RSTN", "12": "GND", "24": "+3V3",
            "22": "SCL2", "23": "SDA2"}
    chans = [("4", "5"), ("6", "7"), ("8", "9"), ("10", "11"), ("13", "14"), ("15", "16"), ("17", "18"), ("19", "20")]
    for k, (pd, pc) in enumerate(chans):
        u = k + 1
        nets[pd] = "UNIT%d_SDA" % u if u <= N // 2 else "NC"
        nets[pc] = "UNIT%d_SCL" % u if u <= N // 2 else "NC"
    b.sym("U6", "Interface_Expansion:TCA9548APWR", "TCA9548APWR", "Package_SO:TSSOP-24_4.4x7.8mm_P0.65mm", nets,
          at=P(80, 70), fields={"LCSC": "C130026"},
          desc="I2C switch, 0x70; channel k = unit k + 1")
    b.sym("C28", "Device:C", "100n", C0402, {"1": "+3V3", "2": "GND"}, at=P(120, 50), desc="TCA9548A VCC")
    b.sym("R31", "Device:R", "10k", R0402, {"1": "+3V3", "2": "MUX_RSTN"}, at=P(52, 60), desc="/RESET pull-up")
    b.sym("Q6", "Transistor_FET:2N7002", "2N7002", "Package_TO_SOT_SMD:SOT-23", {"1": "MUX_RST", "2": "GND", "3": "MUX_RSTN"},
          at=P(44, 74), fields={"LCSC": "C8545"}, desc="MCU MUX_RST high -> mux reset")
    b.sym("R32", "Device:R", "100k", R0402, {"1": "MUX_RST", "2": "GND"}, at=P(36, 82), desc="Q6 gate pull-down: MCU in reset = mux running")
    for pin in ("22", "23"):
        b.port(nets[pin], "bidirectional", side="left", pos=("U6", pin))
    b.port("MUX_RST", "input", side="left", pos=("Q6", "1"))
    for pin in ("3",):
        b.tie(nets[pin], ("U6", pin))
    b.tie("MUX_RSTN", ("R31", "2"))
    b.tie("MUX_RSTN", ("Q6", "3"))
    b.wire("MUX_RST", [("Q6", "1"), ("R32", "1")])
    for pin, net in sorted(nets.items(), key=lambda kv: int(kv[0])):
        if net.startswith("UNIT"):
            b.port(net, "bidirectional", side="right", pos=("U6", pin))
    b.text("I2C SWITCH  U6 TCA9548A 0x70, channel k = unit k + 1", P(30, 40))
    return b.finish()
