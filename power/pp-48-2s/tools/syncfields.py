#!/usr/bin/env python3
"""Copy Value, LCSC and MPN from the schematic netlist (kicadxml) into the board
footprints by reference, so BOM/CPL (pcbkit jlcfab) see what the schematic
says without a new mkboard. Usage: syncfields.py <netlist.xml> <board.kicad_pcb>
"""
import sys
import xml.etree.ElementTree as ET

import pcbnew


def main(xml, pcb):
    comps = {}
    for c in ET.parse(xml).getroot().find('components'):
        fields = {f.get('name'): (f.text or '') for f in c.findall('./fields/field')}
        comps[c.get('ref')] = (c.findtext('value') or '', fields)
    board = pcbnew.LoadBoard(pcb)
    n = 0
    for fp in board.GetFootprints():
        ref = fp.GetReference()
        if ref not in comps:
            continue
        value, fields = comps[ref]
        if fp.GetValue() != value:
            fp.SetValue(value)
        for k in ('LCSC', 'MPN'):
            v = fields.get(k, '')
            if v or fp.HasFieldByName(k):
                fp.SetField(k, v)
                fp.GetFieldByName(k).SetVisible(False)
        n += 1
    board.Save(pcb)
    print('synced', n)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
