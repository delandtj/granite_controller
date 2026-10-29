#!/usr/bin/env python3
"""Split +3V3 and the COLn_SDA / SCL nets per column for krt-route, and join them back.
Usage: netsplit.py split|join <in.kicad_pcb> <out.kicad_pcb>

The core reaches the columns' +3V3 and I2C channels only through In2 (the trunk and
the channel rows of tools/route_hand.py), and KRT does not count In2 tracks as
connections: it then tries to join the core and every column on F / B, cannot cross
the column-edge fences, and leaves the columns' own routes open. split renames the
pads and copper of these nets in column c to temporary nets <net>_Cc (the core keeps
the real names), so KRT routes each part on its own; join renames them back and
drops the temporary nets. Board flow: route_hand.py ->
split -> krt-route -> join -> route_hand.py again.
"""
import fnmatch
import sys

import pcbnew

PITCH, X0 = 25.0, 100.0
NETS = ('+3V3', '/COL*')
TAG = '_C'


def col_of(x):
    return int((pcbnew.ToMM(x) - X0) // PITCH) + 1


def items(board):
    t = board.Tracks()
    out = [t[i] for i in range(len(t))]
    out += [p for fp in board.GetFootprints() for p in fp.Pads()]
    out += [z for z in board.Zones() if not z.GetIsRuleArea()]
    return out


def where(it):
    if isinstance(it, pcbnew.ZONE):
        return it.GetBoundingBox().Centre().x
    if isinstance(it, pcbnew.PCB_TRACK) and not isinstance(it, pcbnew.PCB_VIA):
        return min(it.GetStart().x, it.GetEnd().x)
    return it.GetPosition().x


def split(board):
    nets, moved = {}, 0
    for it in items(board):
        if not any(fnmatch.fnmatchcase(it.GetNetname(), pat) for pat in NETS):
            continue
        c = col_of(where(it))
        if c < 1:
            continue
        name = '%s%s%d' % (it.GetNetname(), TAG, c)
        if name not in nets:
            ni = pcbnew.NETINFO_ITEM(board, name)
            board.Add(ni)
            nets[name] = ni
        it.SetNet(nets[name])
        moved += 1
    print('split: %d items into %s' % (moved, ' '.join(sorted(nets))))


def join(board):
    def real(name):
        base, tag, c = name.rpartition(TAG)
        if tag and c.isdigit() and any(fnmatch.fnmatchcase(base, pat) for pat in NETS):
            return base
        return None

    moved = 0
    for it in items(board):
        base = real(it.GetNetname())
        if base:
            it.SetNet(board.FindNet(base))
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
