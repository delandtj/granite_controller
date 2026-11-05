#!/usr/bin/env python3
"""Split +3V3 and the UNITn_SDA / SCL nets per unit for krt-route, and join them back.
Usage: netsplit.py split|join <in.kicad_pcb> <out.kicad_pcb>

The core reaches the units' +3V3 and I2C channels only through In2 (the trunk and
the channel rows of tools/route_hand.py), and KRT does not count In2 tracks as
connections: it then tries to join the core and every unit on F / B, cannot cross
the unit-edge fences, and leaves the units' own routes open. split renames the
pads and copper of these nets in unit c to temporary nets <net>_Cc (the core keeps
the real names), so KRT routes each part on its own; join renames them back and
drops the temporary nets. The same for slot A's expander lines (/Unit u/A_*),
whose expander sits in slot B behind the In2 bus: their items in slot B become
<net>_SB. Board flow: route_hand.py ->
split -> krt-route -> join -> route_hand.py again.
"""
import fnmatch
import sys

import pcbnew

PITCH, SLOT, X0 = 50.0, 25.0, 100.0   # unit pitch, slot pitch
NETS = ('+3V3', '/UNIT*')
TAG = '_C'
BUS_NETS = ('/Unit */A_*',)            # slot A expander lines: In2 bus to the expander in slot B
BUS_TAG = '_SB'


def col_of(x):
    return int((pcbnew.ToMM(x) - X0) // PITCH) + 1


def in_slot_b(x):
    return (pcbnew.ToMM(x) - X0) % PITCH >= SLOT and pcbnew.ToMM(x) >= X0


def matches(name, pats):
    return any(fnmatch.fnmatchcase(name, pat) for pat in pats)


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
        net = it.GetNetname()
        if matches(net, BUS_NETS):
            if not in_slot_b(where(it)):
                continue
            name = net + BUS_TAG
        elif matches(net, NETS):
            c = col_of(where(it))
            if c < 1:
                continue
            name = '%s%s%d' % (net, TAG, c)
        else:
            continue
        if name not in nets:
            ni = pcbnew.NETINFO_ITEM(board, name)
            board.Add(ni)
            nets[name] = ni
        it.SetNet(nets[name])
        moved += 1
    print('split: %d items into %s' % (moved, ' '.join(sorted(nets))))


def join(board):
    def real(name):
        if name.endswith(BUS_TAG) and matches(name[:-len(BUS_TAG)], BUS_NETS):
            return name[:-len(BUS_TAG)]
        base, tag, c = name.rpartition(TAG)
        if tag and c.isdigit() and matches(base, NETS):
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
