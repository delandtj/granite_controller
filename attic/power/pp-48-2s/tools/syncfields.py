#!/usr/bin/env python3
"""Copy Value, Description, LCSC and MPN from the schematic netlist (kicadxml)
into the board footprints by reference, so BOM/CPL (pcbkit jlcfab) and
kicad-cli drc --schematic-parity see what the schematic says without a new
mkboard. Footprints without a schematic component (H1..) are left alone.
Usage: syncfields.py <netlist.xml> <board.kicad_pcb> [--dry]

KiCad 10 pcbnew API: FOOTPRINT.HasField(name) / GetField(name) /
SetField(name, text); GetFieldByName is gone (KiCad 9 had it).
"""
import sys
import xml.etree.ElementTree as ET

import pcbnew

FIELDS = ('Description', 'LCSC', 'MPN')


def main(xml, pcb, dry=False):
    comps = {}
    for c in ET.parse(xml).getroot().find('components'):
        fields = {f.get('name'): (f.text or '') for f in c.findall('./fields/field')}
        comps[c.get('ref')] = (c.findtext('value') or '', fields)
    board = pcbnew.LoadBoard(pcb)
    n = changed = 0
    for fp in board.GetFootprints():
        ref = fp.GetReference()
        if ref not in comps:
            continue
        n += 1
        value, fields = comps[ref]
        if fp.GetValue() != value:
            print('%s Value %r -> %r' % (ref, fp.GetValue(), value))
            fp.SetValue(value)
            changed += 1
        for k in FIELDS:
            v = fields.get(k, '')
            if not v and not fp.HasField(k):
                continue
            old = fp.GetFieldText(k) if fp.HasField(k) else None
            if old == v:
                continue
            print('%s %s %r -> %r' % (ref, k, old, v))
            fp.SetField(k, v)
            fp.GetField(k).SetVisible(False)
            changed += 1
    if not dry:
        board.Save(pcb)
    print('footprints %d, fields changed %d%s' % (n, changed, ' (dry)' if dry else ''))


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2], dry='--dry' in sys.argv[3:])
