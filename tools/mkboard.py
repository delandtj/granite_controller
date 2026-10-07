#!/usr/bin/env python3
"""Create a fresh 4-layer .kicad_pcb from a KiCad XML netlist.

Footprints are loaded from the system libs or the project 'granite' lib,
linked to their schematic symbols by KIID path (so F8 / parity sees them
as in sync), and every pad gets its net. Parts are dropped in a grid per
sheet, outside any outline; real placement comes later.
"""
import sys
import xml.etree.ElementTree as ET
import pcbnew

netlist, proj, out = sys.argv[1:4]

LIBS = {"granite": f"{proj}/footprints/granite.pretty"}


def libpath(lib):
    return LIBS.get(lib, f"/usr/share/kicad/footprints/{lib}.pretty")


root = ET.parse(netlist).getroot()
board = pcbnew.BOARD()
board.SetCopperLayerCount(4)

# nets
pins = {}  # (ref, pad) -> NETINFO_ITEM
for n in root.find("nets"):
    name = n.get("name")
    # the XML export unescapes '/' inside auto-generated pin-name nets
    if name.startswith(("Net-(", "unconnected-(")):
        name = name.replace("/", "{slash}")
    ni = pcbnew.NETINFO_ITEM(board, name)
    board.Add(ni)
    for node in n.findall("node"):
        pins[(node.get("ref"), node.get("pin"))] = ni

sheet_col = {}
sheet_count = {}
missing = []
for c in root.find("components"):
    ref = c.get("ref")
    fpid = c.findtext("footprint")
    if not fpid:
        missing.append(ref)
        continue
    lib, name = fpid.split(":", 1)
    fp = pcbnew.FootprintLoad(libpath(lib), name)
    if fp is None:
        sys.exit(f"cannot load {fpid} for {ref}")
    fp.SetReference(ref)
    fp.SetValue(c.findtext("value"))
    fpidobj = pcbnew.LIB_ID(lib, name)
    fp.SetFPID(fpidobj)

    sp = c.find("sheetpath")
    fp.SetPath(pcbnew.KIID_PATH(sp.get("tstamps") + c.findtext("tstamps")))
    props = {p.get("name"): p.get("value") for p in c.findall("property")}
    fp.SetSheetname(props.get("Sheetname", ""))
    fp.SetSheetfile(props.get("Sheetfile", ""))
    for f in c.findall("fields/field"):
        fname = f.get("name")
        if fname in ("Footprint", "Reference", "Value"):
            continue
        fp.SetField(fname, f.text or "")
        fld = fp.GetField(fname)
        fld.SetVisible(False)
        fld.SetLayer(pcbnew.F_Fab)
    # mirror the symbol's BOM flag, as Update PCB from Schematic would
    attrs = fp.GetAttributes() & ~pcbnew.FP_EXCLUDE_FROM_BOM
    if "exclude_from_bom" in props:
        attrs |= pcbnew.FP_EXCLUDE_FROM_BOM
    fp.SetAttributes(attrs)
    if "dnp" in props:
        fp.SetDNP(True)

    # staging grid, one column block per sheet, left of origin
    sheet = props.get("Sheetname", "root")
    col = sheet_col.setdefault(sheet, len(sheet_col))
    i = sheet_count.get(sheet, 0)
    sheet_count[sheet] = i + 1
    x = -400 + col * 90 + (i % 6) * 14
    y = (i // 6) * 14
    fp.SetPosition(pcbnew.VECTOR2I_MM(x, y))

    board.Add(fp)
    for pad in fp.Pads():
        ni = pins.get((ref, pad.GetNumber()))
        if ni is not None:
            pad.SetNet(ni)

board.Save(out)
print(f"saved {out}: {len(board.GetFootprints())} footprints, "
      f"{board.GetNetCount()} nets, missing fp: {missing}")
