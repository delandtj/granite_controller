#!/usr/bin/env python3
"""Silkscreen legends for the test points and LEDs (their Value: the net or
function name), in a group "legend" that a re-run replaces.
Usage: legend.py <in.kicad_pcb> <out.kicad_pcb>   (after pcbkit silk)
"""
import sys

import pcbnew

GROUP = 'legend'
# ref prefix: (dx, dy) of the text centre from the footprint origin
OFFSET = {'TP': (0.0, 2.9), 'LED': (0.0, -1.9)}
# per-ref overrides (the Kelvin GND loops sit under the brick's bottom edge)
OFFSET_REF = {'TP2': (-5.6, 0.0), 'TP4': (5.6, 0.0)}
# free texts: (text, x, y)  -- terminal polarity beside the blocks
TEXTS = [('+', 115.0, 125.0), ('-', 115.0, 130.08), ('+', 215.0, 124.0), ('-', 215.0, 118.92)]


def main(src, dst):
    board = pcbnew.LoadBoard(src)
    for g in list(board.Groups()):
        if g.GetName() == GROUP:
            for it in list(g.GetItems()):
                board.Remove(it)
            board.Remove(g)
    grp = pcbnew.PCB_GROUP(board)
    grp.SetName(GROUP)
    board.Add(grp)
    n = 0

    def text(s, x, y, size=0.8):
        t = pcbnew.PCB_TEXT(board)
        t.SetText(s)
        t.SetLayer(pcbnew.F_SilkS)
        t.SetPosition(pcbnew.VECTOR2I(pcbnew.FromMM(x), pcbnew.FromMM(y)))
        t.SetTextSize(pcbnew.VECTOR2I(pcbnew.FromMM(size), pcbnew.FromMM(size)))
        t.SetTextThickness(pcbnew.FromMM(0.15))
        t.SetHorizJustify(pcbnew.GR_TEXT_H_ALIGN_CENTER)
        board.Add(t)
        grp.AddItem(t)

    for fp in board.GetFootprints():
        ref = fp.GetReference()
        pre = ref.rstrip('0123456789')
        if pre not in OFFSET:
            continue
        dx, dy = OFFSET_REF.get(ref, OFFSET[pre])
        p = fp.GetPosition()
        text(fp.GetValue(), pcbnew.ToMM(p.x) + dx, pcbnew.ToMM(p.y) + dy)
        n += 1
    for s, x, y in TEXTS:
        text(s, x, y, 1.2)
        n += 1
    board.Save(dst)
    print('legends', n)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
