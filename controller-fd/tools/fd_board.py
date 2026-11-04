#!/usr/bin/env python3
"""FD board from the routed rev D board: drop the I2C plane link, place the
CAN link. Nothing else moves.

Usage: fd_board.py <revd.kicad_pcb> <net.xml> <out.kicad_pcb>

<revd> is controller/granite-controller.kicad_pcb (rev D, routed, DRC
clean); <net.xml> the FD netlist (kicad-cli sch export netlist --format
kicadxml), for the new parts' fields (MANUFACTURER / MPN / LCSC /
Datasheet / Description). Write <out> outside the project (pcbnew rewrites the .kicad_pro
next to it), then:

  pcbkit relink <net.xml> <out> --save          nets / symbol paths from the FD schematic
  fd_hand.py <out>                              plane vias and straps of the new parts
  krt-reroute (see docs below: the net list fd_board.py prints)

Steps here:
 1. Removed parts (J11-J14, D11, U19-U22, R66-R74, C53-C56) go, with all
    copper of the nets only they (or the U1 pins whose net changed, D10's
    anode) carried; the 'conn-labels' text of J11-J14 / J9 goes too.
 2. J9 (Qwiic) leaves the right edge for J15: it goes to the bottom edge
    where J11 / J12 sat, its ESD / PTC / TVS / 330R array (U6, F1, D6, RN1)
    beside it in the strip below the W5500 bus, so only I2C_LOC_* cross the
    bus. Their nets (HDR_*, I2C_LOC_*) lose all copper and are rerouted.
 3. New parts from the stock libraries:
    J15 2x3 right-angle socket on B at the right edge, rows vertical, pin 1
        at the top at Y0 + 30.0 mm, socket face 4.0 mm past the edge (the
        plane boards' link geometry, power/pp-48/tools/boardfit.py link()).
    U24 TCAN1044V below J15 (CANH / CANL toward J15, STB facing U1 pin 8,
        VCC / GND toward the edge with C60 / C61), C62 on VIO, D13
        PESD2CANFD24V and the split termination R77 / R78 / C63 beside J15.
    U23 MCP2518FD + Y3 / C58 / C59 left of U1 next to the W5500 bus via
        column (SPI on U23's right side), R75 / R76 pull-ups, R79 SDO 33R,
        C57 on VDD.
 4. Copper of other nets that now overlaps a new or moved pad / courtyard
    region (+0.3 mm) is deleted: for GND / power nets only the hit items,
    for signals the whole net (printed: reroute those).
"""
import sys
import xml.etree.ElementTree as ET
import pcbnew

MM = pcbnew.FromMM
LIB = "/usr/share/kicad/footprints"
X0, Y0, X1, Y1 = 25.0, 28.6, 130.0, 78.6      # rev D outline (105 x 50)

REMOVED = "J11 J12 J13 J14 D11 U19 U20 U21 U22 R66 R67 R68 R69 R70 R71 R72 R73 R74 C53 C54 C55 C56".split()
# nets whose copper is kept although a removed part touched them (pruned later)
KEEP = {"GND", "+3V3", "/Power/VIN_12V"}
# rev D nets that change at kept pads: U1 pins 8 / 26 / 27, D10 anode
CHANGED = {"EXP_INT", "PLANE_SCL", "PLANE_SDA", "/Power/12V_IN_A"}
POWER = {"GND", "+3V3", "+5V", "VBUS", "/Power/VIN_12V", "/Power/BUCK_5V"}

NEW = {
    # ref: (lib, footprint, value)
    "J15": ("Connector_PinSocket_2.54mm", "PinSocket_2x03_P2.54mm_Horizontal", "PLANE_LINK"),
    "U23": ("Package_SO", "SOIC-14_3.9x8.7mm_P1.27mm", "MCP2518FDT-E/SL"),
    "U24": ("Package_SO", "SOIC-8_3.9x4.9mm_P1.27mm", "TCAN1044VDRQ1"),
    "Y3": ("Crystal", "Crystal_SMD_Abracon_ABM8G-4Pin_3.2x2.5mm", "40MHz 15pF YXC"),
    "D13": ("Package_TO_SOT_SMD", "SOT-23", "PESD2CANFD24V-TR"),
    "C57": ("Capacitor_SMD", "C_0402_1005Metric", "100n"),
    "C58": ("Capacitor_SMD", "C_0402_1005Metric", "22p"),
    "C59": ("Capacitor_SMD", "C_0402_1005Metric", "22p"),
    "C60": ("Capacitor_SMD", "C_0402_1005Metric", "100n"),
    "C61": ("Capacitor_SMD", "C_0603_1608Metric", "4.7u"),
    "C62": ("Capacitor_SMD", "C_0402_1005Metric", "100n"),
    "C63": ("Capacitor_SMD", "C_0402_1005Metric", "4.7n"),
    "R75": ("Resistor_SMD", "R_0402_1005Metric", "10k"),
    "R76": ("Resistor_SMD", "R_0402_1005Metric", "10k"),
    "R77": ("Resistor_SMD", "R_0402_1005Metric", "60.4"),
    "R78": ("Resistor_SMD", "R_0402_1005Metric", "60.4"),
    "R79": ("Resistor_SMD", "R_0402_1005Metric", "33"),
}

# J15: plane-board link geometry (boardfit.py FRONT: the 2x3 socket has the
# 2x7's body depth, face 10.03 mm from the outer pad row)
J15_FRONT = 10.03
J15_PAST = 4.0
J15_PIN1_Y = Y0 + 30.0

# ref -> (rot, courtyard centre x, y); F side
PLACE = {
    # J9 group: J14's place on the bottom edge, protection above the bus
    # (all in the strip below the W5500 bus: only I2C_LOC_* cross it)
    # (J9 pin 1 2 mm clear of rev D's +3V3 feed to the W5500 at x 57)
    "J9": (0, 62.0, Y1 - 2.8),
    "F1": (0, 69.4, 76.0), "D6": (0, 73.6, 76.0), "U6": (90, 78.0, 76.0), "RN1": (90, 81.6, 76.0),
    # MCP2518FD block left of U1's SPI column
    "U23": (0, 84.6, 63.6),
    "Y3": (0, 78.8, 65.4), "C58": (90, 75.6, 64.3), "C59": (90, 75.6, 66.5),
    "C57": (180, 86.0, 58.3), "R75": (0, 88.6, 58.3), "R76": (90, 80.4, 61.6),
    "R79": (0, 90.6, 62.6),
    # transceiver below J15, caps on the edge side
    # (C62 on VIO sits in the 1.3 mm gap between J15 pin 5 and U24 pin 5,
    # keeping the channel beside U1's right pad column free for its escapes)
    "U24": (180, 124.5, 68.35),
    "C60": (270, 129.0, 68.35), "C61": (90, 129.0, 65.6), "C62": (0, 122.5, 65.1),
    # ESD beside J15 (edge side), split termination above J15
    "D13": (0, 127.6, 60.6),
    "R77": (0, 125.6, 55.0), "R78": (0, 128.0, 55.0), "C63": (90, 126.9, 56.9),
}


def mm(v):
    return pcbnew.ToMM(v)


def pad_xy(fp, num):
    for p in fp.Pads():
        if p.GetNumber() == num:
            return mm(p.GetPosition().x), mm(p.GetPosition().y)
    raise SystemExit("%s has no pad %s" % (fp.GetReference(), num))


def place_c(fp, rot, cx, cy):
    fp.SetOrientationDegrees(rot)
    fp.SetPosition(pcbnew.VECTOR2I(0, 0))
    lay = pcbnew.B_CrtYd if fp.IsFlipped() else pcbnew.F_CrtYd
    c = fp.GetCourtyard(lay).BBox()
    fp.SetPosition(pcbnew.VECTOR2I(MM(cx) - c.GetCenter().x, MM(cy) - c.GetCenter().y))


def link(fp, edge_x, pin1_y, past):
    """boardfit.py link() for face 'right': on B, rows vertical, pin 1 at
    the top, outer row (pin 2) toward the edge"""
    if not fp.IsFlipped():
        fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
    n = len(list(fp.Pads()))
    for rot in (0, 90, 180, 270):
        fp.SetOrientationDegrees(rot)
        fp.SetPosition(pcbnew.VECTOR2I(0, 0))
        (ax, ay), (bx, by), (ox, _) = pad_xy(fp, "1"), pad_xy(fp, str(n - 1)), pad_xy(fp, "2")
        if abs(ax - bx) < 1e-3 and ay < by and ox > ax:
            fp.SetPosition(pcbnew.VECTOR2I(MM(edge_x + past - J15_FRONT - ox), MM(pin1_y - ay)))
            return rot
    raise SystemExit("J15: no orientation fits")


def hide_fields(fp):
    for f in fp.GetFields():
        if not f.IsReference():
            f.SetVisible(False)


def main(argv):
    src, netxml, out = argv
    comps = {c.get("ref"): c for c in ET.parse(netxml).getroot().iter("comp")}
    b = pcbnew.LoadBoard(src)
    fps = {f.GetReference(): f for f in b.GetFootprints()}

    # 1. removed parts and their nets' copper; 2. J9 group nets
    drop = set(CHANGED)
    for r in REMOVED:
        for p in fps[r].Pads():
            n = str(p.GetNetname())
            if n and n not in KEEP:
                drop.add(n)
    for r in ("J9", "U6", "RN1", "F1", "D6"):
        for p in fps[r].Pads():
            n = str(p.GetNetname())
            if "HDR_" in n or n.startswith("I2C_LOC_"):
                drop.add(n)
    tr = b.Tracks()
    gone = [tr[i] for i in range(len(tr)) if str(tr[i].GetNetname()) in drop]
    labels = [g for g in b.Groups() if g.GetName() == "conn-labels"]
    labels = [(g, list(g.GetItems())) for g in labels]
    dead = [fps.pop(r) for r in REMOVED]
    for t in gone:
        b.Remove(t)
    for f in dead:
        b.Remove(f)
    for g, its in labels:
        for it in its:
            b.Remove(it)
        b.Remove(g)
    fps = {f.GetReference(): f for f in b.GetFootprints()}
    print("dropped nets:", " ".join(sorted(drop)), "(%d items)" % len(gone))

    # 3. new parts
    for r, (lib, name, val) in NEW.items():
        fp = pcbnew.FootprintLoad("%s/%s.pretty" % (LIB, lib), name)
        fp.SetParent(b)
        fp.SetFPID(pcbnew.LIB_ID(lib, name))
        fp.SetReference(r)
        fp.SetValue(val)
        for f in comps[r].iter("field"):
            if f.get("name") in ("MANUFACTURER", "MPN", "LCSC", "Datasheet", "Description"):
                fp.SetField(f.get("name"), f.text or "")
        hide_fields(fp)
        fp.Value().SetVisible(False)
        b.Add(fp)
        fps[r] = fp
    rot = link(fps["J15"], X1, J15_PIN1_Y, J15_PAST)
    for r, (rr, cx, cy) in PLACE.items():
        place_c(fps[r], rr, cx, cy)
    j = fps["J15"]
    print("J15 on B rot %d, pin 1 at (%.3f, %.3f) = top + %.3f, pin 2 x %.3f, face x %.3f" % (
        rot, *pad_xy(j, "1"), pad_xy(j, "1")[1] - Y0, pad_xy(j, "2")[0], pad_xy(j, "2")[0] + J15_FRONT))

    # 4. foreign copper on the new / moved parts
    moved = list(NEW) + ["J9", "U6", "RN1", "F1", "D6"]
    rip, hit = set(), []
    clr = MM(0.3)
    tr = b.Tracks()
    items = [tr[i] for i in range(len(tr))]
    for r in moved:
        fp = fps[r]
        lay = pcbnew.B_CrtYd if fp.IsFlipped() else pcbnew.F_CrtYd
        crt = fp.GetCourtyard(lay)
        smd = not any(p.HasHole() for p in fp.Pads())
        for t in items:
            n = str(t.GetNetname())
            via = t.GetClass() == "PCB_VIA"
            layers = (pcbnew.F_Cu, pcbnew.B_Cu) if via else (t.GetLayer(),)
            bad = any(p.IsOnLayer(L) and p.GetEffectiveShape(L).Collide(t.GetEffectiveShape(L), clr)
                      for p in fp.Pads() for L in layers)
            if not bad and via and smd and crt.OutlineCount():
                bad = crt.Collide(t.GetPosition(), 0)      # no vias under SMD bodies
            if bad:
                if n in POWER or n == "":
                    hit.append(t)
                else:
                    rip.add(n)
    for n in list(rip):          # diff pairs go together
        for a, c in (("D+", "D-"), ("D-", "D+")):
            if n.endswith(a):
                rip.add(n[:-2] + c)
    for t in items:
        if str(t.GetNetname()) in rip and t not in hit:
            hit.append(t)
    for t in hit:
        b.Remove(t)
    print("ripped nets:", " ".join(sorted(rip)))
    print("also removed %d power/GND items" % len(hit))
    for z in b.Zones():        # relink reads fills as shorts
        z.UnFill()
    b.Save(out)


if __name__ == "__main__":
    main(sys.argv[1:])
