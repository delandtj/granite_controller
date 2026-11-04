#!/usr/bin/env python3
"""Bring a routed rev D board onto the CAN FD netlist (ADR 0006): footprints in / out / swapped (a swapped one
keeps its place), values, fields, symbol paths, pad nets. Copper of nets that no longer exist is deleted;
everything else stays. Usage: fdsync.py <in.kicad_pcb> <netlist.xml> <proj_dir> <out.kicad_pcb>"""
import os
import re
import sys
import xml.etree.ElementTree as ET

import pcbnew

src, netlist, proj, out = sys.argv[1:5]
libs = {}
t = os.path.join(proj, 'fp-lib-table')
for name, uri in re.findall(r'\(name "([^"]*)"\)\s*\(type "[^"]*"\)\s*\(uri "([^"]*)"\)', open(t).read()):
    libs[name] = uri.replace('${KIPRJMOD}', proj)


def libpath(lib):
    return libs.get(lib, '/usr/share/kicad/footprints/%s.pretty' % lib)


def bname(n):
    return n.replace('/', '{slash}') if n.startswith(('Net-(', 'unconnected-(')) else n


root = ET.parse(netlist).getroot()
b = pcbnew.LoadBoard(src)
pins = {}
names = set()
for n in root.find('nets'):
    name = bname(n.get('name'))
    names.add(name)
    ni = b.FindNet(name)
    if ni is None:
        ni = pcbnew.NETINFO_ITEM(b, name)
        b.Add(ni)
    for node in n.findall('node'):
        pins[(node.get('ref'), node.get('pin'))] = name

comps = {c.get('ref'): c for c in root.find('components')}
added, swapped, dropped = [], [], []
for fp in list(b.GetFootprints()):
    r = fp.GetReference()
    if r not in comps and not (fp.GetAttributes() & pcbnew.FP_BOARD_ONLY):
        dropped.append(r)
        b.Delete(fp)
for r, c in comps.items():
    fpid = c.findtext('footprint')
    lib, name = fpid.split(':', 1)
    old = b.FindFootprintByReference(r)
    if old is not None and old.GetFPID().GetUniStringLibId() == fpid:
        fp = old
    else:
        fp = pcbnew.FootprintLoad(libpath(lib), name)
        if fp is None:
            raise SystemExit('cannot load ' + fpid)
        fp.SetReference(r)
        fp.SetFPID(pcbnew.LIB_ID(lib, name))
        b.Add(fp)
        if old is not None:
            if old.IsFlipped():
                fp.Flip(fp.GetPosition(), pcbnew.FLIP_DIRECTION_TOP_BOTTOM)
            fp.SetOrientation(old.GetOrientation())
            fp.SetPosition(old.GetPosition())
            b.Delete(old)
            swapped.append(r)
        else:
            fp.SetPosition(pcbnew.VECTOR2I_MM(20, 100 + 5 * len(added)))
            added.append(r)
    fp.SetValue(c.findtext('value'))
    sp = c.find('sheetpath')
    fp.SetPath(pcbnew.KIID_PATH(sp.get('tstamps') + c.findtext('tstamps').split()[0]))
    props = {p.get('name'): p.get('value') for p in c.findall('property')}
    fp.SetSheetname(props.get('Sheetname', ''))
    fp.SetSheetfile(props.get('Sheetfile', ''))
    for f in c.findall('fields/field'):
        fn = f.get('name')
        if fn in ('Footprint', 'Reference', 'Value'):
            continue
        had = fp.HasField(fn)
        fp.SetField(fn, f.text or '')
        if not had:
            fld = fp.GetField(fn)
            fld.SetVisible(False)
            fld.SetLayer(pcbnew.B_Fab if fp.IsFlipped() else pcbnew.F_Fab)
    attrs = fp.GetAttributes() & ~pcbnew.FP_EXCLUDE_FROM_BOM
    if 'exclude_from_bom' in props:
        attrs |= pcbnew.FP_EXCLUDE_FROM_BOM
    fp.SetAttributes(attrs)
    fp.SetDNP('dnp' in props)
    for pad in fp.Pads():
        n = pins.get((r, pad.GetNumber()))
        pad.SetNet(b.FindNet(n) if n else b.FindNet(''))
t = b.Tracks()
items = [t[i] for i in range(len(t))]
gone = [it for it in items if it.GetNetname() and it.GetNetname() not in names]
for it in gone:
    b.Delete(it)
zg = [z for z in b.Zones() if not z.GetIsRuleArea() and z.GetNetname() not in names]
for z in zg:
    b.Delete(z)
b.Save(out)
print('dropped', dropped)
print('swapped', swapped)
print('added', added)
print('deleted copper of old nets:', len(gone), 'items,', len(zg), 'zones')
