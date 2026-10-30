#!/usr/bin/env python3
"""Rev D placement: 100 x 50 mm board, blocks carried over from rev C.

Usage: place.py <revc.kicad_pcb> <in.kicad_pcb> <out.kicad_pcb>

<revc> is the routed rev C board (granite_controller, or this project's
board before rev D: git show da0f13c:controller/granite-controller.kicad_pcb).
<in> is the rev D board from `pcbkit mkboard` + `pcbkit outline` (correct
footprints, nets and symbol links, parts parked off the board).

Blocks copied from rev C with their routing, shifted by (dx, dy):
  ETH  J1 / U3 / Y2 / magnetics and pairs (all /Ethernet/ but the W5500
       strap pull-ups R16-R18)                      dx -24
  BUCK U8 loop + U2 3V3 buck + D2 / D9               dx +31, dy -19
  MCU  U1 with C12-C14, C51, R34 / R35               dx -12
  USB  J2, U4, R24                                   dx 0
Copper is copied only where both ends lie inside the block's rev C region
and the net is one of the block's pads (mapped to the rev D net through
the pads); open ends are pruned back to the pads. The copied copper goes
into group "hand" (pcbkit.toml [krt] keep_groups).

Everything else is placed at explicit coordinates (PLACE below).
Same scratch-dir rule as the pcbkit tools: write <out> outside the project.
"""
import sys
import pcbnew

MM = pcbnew.FromMM


def mm(v):
    return pcbnew.ToMM(v)


ETH_EXCL = {"R16", "R17", "R18"}
BLOCKS = [
    # name, refs (None = all of a sheet), sheet, dx, dy, rev C region (x0, y0, x1, y1)
    ("ETH", None, "Ethernet", -24.0, 0.0, (55.5, 28.0, 79.6, 79.0)),
    ("BUCK", ["U8", "L3", "C40", "C41", "C42", "C43", "C44", "C45", "R37", "R38",
              "U2", "L1", "C3", "C4", "R1", "R2", "D2", "D9"], None, 31.0, -19.0,
     (25.5, 54.6, 56.0, 76.0)),
    ("MCU", ["U1", "C12", "C13", "C14", "C51", "R34", "R35"], None, -12.0, 0.0,
     (104.0, 58.0, 129.9, 79.0)),
    ("USB", ["J2", "U4", "R24"], None, 0.0, 0.0,
     (93.0, 28.0, 117.0, 48.5)),
]

# blocks whose copper is copied for these (rev D) nets only; the rest of
# their rev C routing served nets that changed
ONLY = {
    "MCU": {"GND", "+3V3", "/MCU/C6_EN", "/MCU/C6_USB_D+", "/MCU/C6_USB_D-"},
    "USB": {"GND", "VBUS", "/MCU/USB_CONN_D+", "/MCU/USB_CONN_D-", "/MCU/USB_CC1",
            "/MCU/USB_CC2", "/MCU/USB_D+", "/MCU/USB_D-"},
}
PLANE_NETS = {"GND", "+3V3"}

# rest: ref -> (rot, courtyard centre x, y)
CON_T = 28.65 + 2.8          # PH row along the top edge (courtyard 5.59 deep)
CON_B = 78.55 - 2.8          # along the bottom edge
CON_R = 124.95 - 2.8         # along the right edge (rot 90)
PLACE = {
    # probes on the top edge between the magjack and USB-C
    "J4": (0, 59.8, CON_T), "J5": (0, 69.5, CON_T), "J6": (0, 79.2, CON_T), "J7": (0, 88.9, CON_T),
    "R30": (0, 86.0, 35.6), "D5": (0, 88.6, 35.6),
    # 12 V inputs and plane link on the bottom edge, link next to U1's I2C pins
    "J11": (0, 54.8, CON_B), "J12": (0, 61.9, CON_B), "J14": (0, 72.0, CON_B), "J13": (0, 85.1, CON_B),
    # OR-ing diodes and TVS under the buck, VIN_SENSE divider beside them
    "D10": (0, 61.0, 56.8), "D11": (0, 68.6, 56.8), "D12": (0, 76.2, 56.8),
    "R63": (90, 81.0, 56.8), "R64": (90, 82.4, 56.8),
    # +5V bulk, VBUS OR diode, VBUS parts
    "C2": (90, 87.4, 45.0), "D3": (90, 92.0, 44.0), "C52": (90, 92.0, 38.9), "R65": (90, 90.3, 38.9),
    # test pads, W5500 strap pull-ups
    "TP1": (0, 86.5, 51.5), "TP2": (0, 89.5, 51.5), "TP3": (0, 86.5, 54.5), "TP4": (0, 89.5, 54.5),
    "R16": (90, 87.5, 57.0), "R17": (90, 89.0, 57.0), "R18": (90, 90.5, 57.0),
    # above U1: TMP1075, local I2C pull-ups, LED, CC1
    "U5": (0, 98.8, 53.5), "C15": (90, 103.6, 53.5), "R13": (90, 105.0, 53.5), "R14": (90, 106.4, 53.5),
    "D4": (0, 98.0, 47.5), "R8": (0, 98.0, 49.3), "R25": (0, 109.0, 42.6),
    "R27": (0, 114.6, 65.7), "R7": (0, 114.6, 70.3),
    # J9 (Qwiic) and J10 (dry contacts) on the right edge with their protection
    "J9": (90, CON_R, 44.0), "J10": (90, CON_R, 62.5),
    "U6": (0, 117.8, 44.0), "RN1": (90, 115.2, 44.0), "F1": (90, 117.8, 40.3), "D6": (90, 117.8, 48.4),
    "U18": (0, 117.6, 58.0),
    "R59": (90, 109.0, 50.0), "R60": (90, 110.5, 50.0), "R61": (90, 112.0, 50.0), "R62": (90, 113.5, 50.0),
    "R55": (90, 109.0, 52.4), "R56": (90, 110.5, 52.4), "R57": (90, 112.0, 52.4), "R58": (90, 113.5, 52.4),
    "C47": (90, 109.0, 54.8), "C48": (90, 110.5, 54.8), "C49": (90, 112.0, 54.8), "C50": (90, 113.5, 54.8),
    # plane link: ESD + 33R at the connectors, load switches and pull-ups above them
    "R36": (90, 90.5, 66.0),
    "U19": (90, 85.0, 67.6), "R69": (90, 81.0, 67.5), "R70": (90, 82.4, 67.5),
    "U20": (90, 72.0, 67.6), "R72": (90, 68.0, 67.5), "R73": (90, 69.4, 67.5),
    "U21": (0, 85.0, 62.5), "R71": (90, 88.3, 62.5), "C53": (90, 81.6, 62.5), "C54": (90, 89.8, 62.5),
    "U22": (0, 72.0, 62.5), "R74": (90, 75.3, 62.5), "C55": (90, 68.6, 62.5), "C56": (90, 76.8, 62.5),
    "R66": (90, 78.6, 62.5), "R67": (90, 80.0, 62.5), "R68": (90, 79.3, 65.5),
}

def main(argv):
    src_p, in_p, out_p = argv
    src = pcbnew.LoadBoard(src_p)
    dst = pcbnew.LoadBoard(in_p)
    sf = {f.GetReference(): f for f in src.GetFootprints()}
    df = {f.GetReference(): f for f in dst.GetFootprints()}

    group = pcbnew.PCB_GROUP(dst)
    group.SetName("hand")
    dst.Add(group)
    nets = dst.GetNetsByName()

    copied = []
    for name, refs, sheet, dx, dy, reg in BLOCKS:
        only = ONLY.get(name)
        if refs is None:
            refs = [r for r, f in df.items()
                    if f.GetSheetname() == sheet and r not in ETH_EXCL]
        netmap = {}
        for r in refs:
            s, d = sf[r], df[r]
            if s.IsFlipped() != d.IsFlipped():
                d.Flip(d.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
            d.SetOrientation(s.GetOrientation())
            d.SetPosition(pcbnew.VECTOR2I(s.GetPosition().x + MM(dx), s.GetPosition().y + MM(dy)))
            dp = {p.GetNumber(): p.GetNetname() for p in d.Pads()}
            for p in s.Pads():
                old, new = p.GetNetname(), dp.get(p.GetNumber())
                if old and new:
                    netmap.setdefault(str(old), set()).add(str(new))
        netmap = {o: n.pop() for o, n in netmap.items() if len(n) == 1}
        x0, y0, x1, y1 = reg

        def inside(v):
            return x0 <= mm(v.x) <= x1 and y0 <= mm(v.y) <= y1
        n = 0
        for t in src.GetTracks():
            new = netmap.get(str(t.GetNetname()))
            if not new or (only and new not in only):
                continue
            if t.GetClass() == "PCB_VIA":
                if not inside(t.GetPosition()):
                    continue
                v = pcbnew.PCB_VIA(dst)
                v.SetPosition(pcbnew.VECTOR2I(t.GetPosition().x + MM(dx), t.GetPosition().y + MM(dy)))
                v.SetWidth(t.GetWidth(pcbnew.F_Cu))
                v.SetDrill(t.GetDrill())
                v.SetViaType(t.GetViaType())
                v.SetLayerPair(pcbnew.F_Cu, pcbnew.B_Cu)
                it = v
            else:
                if not (inside(t.GetStart()) and inside(t.GetEnd())):
                    continue
                if t.GetClass() == "PCB_ARC":
                    it = pcbnew.PCB_ARC(dst)
                    it.SetMid(pcbnew.VECTOR2I(t.GetMid().x + MM(dx), t.GetMid().y + MM(dy)))
                else:
                    it = pcbnew.PCB_TRACK(dst)
                it.SetStart(pcbnew.VECTOR2I(t.GetStart().x + MM(dx), t.GetStart().y + MM(dy)))
                it.SetEnd(pcbnew.VECTOR2I(t.GetEnd().x + MM(dx), t.GetEnd().y + MM(dy)))
                it.SetWidth(t.GetWidth())
                it.SetLayer(t.GetLayer())
            it.SetNet(nets[new])
            dst.Add(it)
            group.AddItem(it)
            copied.append(it)
            n += 1
        print("%-5s %2d parts, %4d copper items copied" % (name, len(refs), n))

    for r, (rot, cx, cy) in PLACE.items():
        f = df[r]
        f.SetOrientationDegrees(rot)
        f.SetPosition(pcbnew.VECTOR2I(0, 0))
        c = f.GetCourtyard(pcbnew.F_CrtYd).BBox()
        f.SetPosition(pcbnew.VECTOR2I(MM(cx) - c.GetCenter().x, MM(cy) - c.GetCenter().y))

    prune(dst, copied)
    rule_areas(src, dst, df)
    dst.Save(out_p)
    left = [r for r, f in df.items() if mm(f.GetPosition().x) < 0]
    print("not placed:", " ".join(sorted(left)) or "-")
    overlaps(dst)


def overlaps(b):
    """Courtyard box overlaps (front side), as a quick placement check."""
    box = {}
    for f in b.GetFootprints():
        c = f.GetCourtyard(pcbnew.F_CrtYd)
        if c.OutlineCount():
            bb = c.BBox()
            box[f.GetReference()] = (mm(bb.GetLeft()), mm(bb.GetTop()), mm(bb.GetRight()), mm(bb.GetBottom()))
    refs = sorted(box)
    for i, a in enumerate(refs):
        for o in refs[i + 1:]:
            p, q = box[a], box[o]
            dx = min(p[2], q[2]) - max(p[0], q[0])
            dy = min(p[3], q[3]) - max(p[1], q[1])
            if dx > 0.01 and dy > 0.01:
                print("overlap %s %s %.2f x %.2f" % (a, o, dx, dy))


def rule_areas(src, dst, df):
    """Rev C keep-outs: U1 body (no F.Cu tracks) moved with U1, the screw-head
    keep-out square of H1 copied to every hole."""
    zs = [z for z in src.Zones() if z.GetIsRuleArea()]
    body = [z for z in zs if z.GetZoneName() == "U1 body: no F.Cu tracks"][0]
    h1 = [z for z in zs if z.GetZoneName().startswith("H1 ")][0]
    hx = src.FindFootprintByReference("H1").GetPosition()
    todo = [(body, MM(-12.0), 0, body.GetZoneName())]
    for h in ("H1", "H2", "H3", "H4"):
        p = df[h].GetPosition()
        todo.append((h1, p.x - hx.x, p.y - hx.y, h + " screw head keep-out"))
    for z, dx, dy, name in todo:
        n = pcbnew.ZONE(z)
        n.SetParent(dst)
        n.Move(pcbnew.VECTOR2I(int(dx), int(dy)))
        n.SetZoneName(name)
        dst.Add(n)


def prune(b, items):
    """Delete copied tracks/vias with an end that touches nothing of their net."""
    live = list(items)
    tol = MM(0.002)
    pads = [p for p in b.GetPads()]
    while True:
        dead = []
        for it in live:
            net = it.GetNetCode()
            if it.GetClass() == "PCB_VIA":
                links = sum(1 for o in live if o is not it and o.GetNetCode() == net and
                            o.GetClass() != "PCB_VIA" and touches(o, it.GetPosition(), None, tol)) + \
                    sum(1 for p in pads if p.GetNetCode() == net and p.HitTest(it.GetPosition()))
                # a plane net's via may end a stub (it lands on In1 / In2); any
                # other via needs copper on two sides
                if links < (1 if it.GetNetname() in PLANE_NETS else 2):
                    dead.append(it)
                continue
            for e in (it.GetStart(), it.GetEnd()):
                lay = it.GetLayer()
                if any(p.GetNetCode() == net and p.IsOnLayer(lay) and p.HitTest(e) for p in pads):
                    continue
                if any(o is not it and o.GetNetCode() == net and touches(o, e, lay, tol) for o in live):
                    continue
                dead.append(it)
                break
        if not dead:
            break
        for it in dead:
            live.remove(it)
            b.Remove(it)
    print("pruned to %d copper items" % len(live))


def touches(o, p, lay, tol):
    if o.GetClass() == "PCB_VIA":
        d = (o.GetPosition() - p)
        return (d.x * d.x + d.y * d.y) ** 0.5 <= o.GetWidth(pcbnew.F_Cu) / 2
    if lay is not None and o.GetLayer() != lay:
        return False
    if lay is None:
        # via centre on a track of an outer layer
        pass
    return o.HitTest(p, int(tol))


if __name__ == "__main__":
    main(sys.argv[1:])
