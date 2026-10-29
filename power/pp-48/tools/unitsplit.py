#!/usr/bin/env python3
"""Split nets of the PP-48 unit board for krt-route, and join them back.
Usage: unitsplit.py split|join <in.kicad_pcb> <out.kicad_pcb>

KRT does not count In2 tracks as connections (tools/netsplit.py has the
story). On the unit board (tools/route_unit.py) the band nets are cut into
the pieces KRT has to join on F / B, by region: In2 copper 'T', margin
x < 100 'M', slot A 'A', slot B 'B', strip x >= 150 'S' (SECTIONS: which
regions of a net get a temporary name <net>_U<region>, the rest keeps the
real name). CH1..CH3, PRESS_EN: J301 side apart from the ladder + J302;
SEG_SDA / SCL: J301 apart from the taps + parts; EXP_RSTN: J301, the
expander tap + U5 / R30, the ladder + J302 each apart; +3V3: J301, each
slot's tap + parts, the strip + J302 pin 14 each apart (the real net is then
empty and krt-route drops it; join makes it again). Slot A's expander lines
/Unit/A_*: their items in slot B (behind the In2 bus) become <net>_SB, as
tools/netsplit.py does on rev1.
join renames them back and drops the temporary nets.
"""
import fnmatch
import os
import sys

import pcbnew

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import netsplit as ns                    # noqa: E402

TAG = '_U'                               # <net>_UT (In2), _UM (margin), _UA / _UB (slot A / B), _US (strip)
# per In2-carried net: which regions get a temporary name (the rest keeps the real one; In2 is always 'T')
SECTIONS = [('/Chain/CH*', 'M'), ('/PRESS_EN', 'M'), ('/SEG_S*', 'M'), ('/EXP_RSTN', 'MB'), ('+3V3', 'MABS')]
ns.BUS_NETS = ('/Unit/A_*',)
ns.NETS = ()


def region(it):
    if isinstance(it, pcbnew.PCB_TRACK) and it.GetLayer() == pcbnew.In2_Cu:
        return 'T'
    x = pcbnew.ToMM(ns.where(it))
    return 'M' if x < 100.0 else 'A' if x < 125.0 else 'B' if x < 150.0 else 'S'


def section(net):
    for pat, regions in SECTIONS:
        if fnmatch.fnmatchcase(net, pat):
            return 'T' + regions
    return None


def split(board):
    ns.split(board)
    nets, moved = {}, 0
    for it in ns.items(board):
        net = it.GetNetname()
        regions = section(net)
        if not regions:
            continue
        r = region(it)
        if r not in regions:
            continue
        name = net + TAG + r
        if name not in nets:
            nets[name] = pcbnew.NETINFO_ITEM(board, name)
            board.Add(nets[name])
        it.SetNet(nets[name])
        moved += 1
    print('split: %d In2-net items into %s' % (moved, ' '.join(sorted(nets))))


def real(name):
    """the real net of a temporary one, else None"""
    base, tag = name[:-3], name[-3:]
    if tag[:2] == TAG and section(base) and tag[2] in section(base):
        return base
    if name.endswith(ns.BUS_TAG) and ns.matches(name[:-len(ns.BUS_TAG)], ns.BUS_NETS):
        return name[:-len(ns.BUS_TAG)]
    return None


def join(board):
    """rename everything first, then drop the temporary nets in one pass (after a net is removed,
    pcbnew's Python board lists stop working)"""
    moved, made = 0, {}
    for it in ns.items(board):
        base = real(it.GetNetname())
        if base:
            ni = board.FindNet(base) or made.get(base)
            if ni is None:                  # KRT drops a net left without items (all of +3V3 is split)
                ni = made[base] = pcbnew.NETINFO_ITEM(board, base)
                board.Add(ni)
            it.SetNet(ni)
            moved += 1
    gone = [ni for name, ni in board.GetNetsByName().items() if real(str(name))]
    for ni in gone:
        board.Remove(ni)
    print('join: %d items back on their nets, %d temporary nets removed' % (moved, len(gone)))


def main(mode, src, dst):
    board = pcbnew.LoadBoard(src)
    {'split': split, 'join': join}[mode](board)
    board.Save(dst)


if __name__ == '__main__':
    main(*sys.argv[1:4])
