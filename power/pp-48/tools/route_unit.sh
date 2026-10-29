#!/usr/bin/env bash
# Route the PP-48 unit board (docs/adr/0005): run in power/pp-48-unit.
#
#   ../pp-48/tools/route_unit.sh <placed.kicad_pcb> [NET ...]
#
# placed board (tools/place_unit.py output) -> route_unit.py (hand copper) ->
# unitsplit.py split -> krt-route -> join -> route_unit.py again (krt-route
# strips some kept copper: U5's +3V3 escapes, U53's OVK escape). Then up to
# three passes: DRC, drop the router's copper on the nets that collide with
# the hand copper or are open, reroute them with --keep (hand copper fixed)
# on the split board, join, route_unit.py once more. The extra NETs join the
# first pass (router-only nets: no hand copper on them). Result:
# pp-48-unit.kicad_pcb. Scratch: $SCRATCH (default ~/.cache/pcbkit/pp-48-unit/flow).
set -euo pipefail
[ $# -ge 1 ] || { echo "usage: route_unit.sh <placed.kicad_pcb> [NET ...]" >&2; exit 2; }
PLACED=$(realpath "$1"); shift
T=$(cd "$(dirname "$0")" && pwd)
PK=${PCBKIT:-$HOME/Electronics/pcbkit/bin/pcbkit}
W=${SCRATCH:-$HOME/.cache/pcbkit/pp-48-unit/flow}
B=pp-48-unit.kicad_pcb
export PCBKIT_CONFIG=$PWD/pcbkit.toml
mkdir -p "$W"
py() { python3 "$@" 2>&1 | grep -v "Debug\|memory leak" || true; }

# nets to reroute: from a DRC report (shorts, clearance, crossings, opens) plus the given ones
nets_from() {
    python3 - "$@" <<'PY'
import re, sys
txt = open(sys.argv[1]).read()
nets = set(sys.argv[2:])
for block in re.split(r'\n(?=\[)', txt):
    head = block.split('\n', 1)[0]
    if head.startswith(('[shorting_items]', '[clearance]', '[tracks_crossing]', '[unconnected_items]')):
        for n in re.findall(r'\[([^\]]+)\] (?:on|of)', block):
            if n not in ('GND', 'VBUS', 'VIN') and not n.startswith('/Input/USENSE'):
                nets.add(n)
print('\n'.join(sorted(nets)))
PY
}

# drop the router's (ungrouped) copper on the named nets
clear_nets() {
    python3 - "$@" <<'PY' 2>&1 | grep -v "Debug\|memory leak"
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1])
nets = set(sys.argv[3:])
t = b.Tracks()
gone = [t[i] for i in range(len(t)) if t[i].GetNetname() in nets and t[i].GetParentGroup() is None]
for it in gone:
    b.Delete(it)
b.Save(sys.argv[2])
print('cleared', len(gone), 'router items')
PY
}

# the names the nets have on the split board (tools/unitsplit.py)
split_names() {
    (cd "$T" && python3 - "$@" <<'PY'
import sys
import unitsplit as us
for n in sys.argv[1:]:
    regions = us.section(n)
    if n.startswith('/Unit/A_'):
        print(n); print(n + '_SB')
    elif regions:
        for r in regions:
            print(n + us.TAG + r)
        if n != '+3V3':
            print(n)
    else:
        print(n)
PY
    ) 2>&1 | grep -v "Debug\|memory leak"
}

py "$T/route_unit.py" "$PLACED" "$W/1-hand.kicad_pcb"
py "$T/unitsplit.py" split "$W/1-hand.kicad_pcb" "$W/2-split.kicad_pcb"
cp "$W/2-split.kicad_pcb" "$B"
"$PK" krt-route > "$W/krt-route.log" 2>&1 || true
grep -E "^\s+(Single-ended|Multi-point):" "$W/krt-route.log" | tail -2
cp "$B" "$W/3-routed.kicad_pcb"
py "$T/unitsplit.py" join "$W/3-routed.kicad_pcb" "$W/4-joined.kicad_pcb"
py "$T/route_unit.py" "$W/4-joined.kicad_pcb" "$W/5-redrawn.kicad_pcb"

CUR=$W/5-redrawn.kicad_pcb
for pass in 1 2 3; do
    cp "$CUR" "$W/p$pass-in.kicad_pcb"
    cp pp-48-unit.kicad_pro "$W/p$pass-in.kicad_pro"
    kicad-cli pcb drc --refill-zones --severity-error -o "$W/p$pass.rpt" "$W/p$pass-in.kicad_pcb" > /dev/null 2>&1 || true
    EXTRA=(); [ "$pass" -eq 1 ] && EXTRA=("$@")
    mapfile -t NETS < <(nets_from "$W/p$pass.rpt" "${EXTRA[@]}")
    echo "pass $pass rerouting: ${NETS[*]:-none}"
    [ ${#NETS[@]} -gt 0 ] || break
    clear_nets "$W/p$pass-in.kicad_pcb" "$W/p$pass-cleared.kicad_pcb" "${NETS[@]}"
    py "$T/unitsplit.py" split "$W/p$pass-cleared.kicad_pcb" "$W/p$pass-split.kicad_pcb"
    cp "$W/p$pass-split.kicad_pcb" "$B"
    mapfile -t SPLIT < <(split_names "${NETS[@]}")
    "$PK" krt-reroute --keep "${SPLIT[@]}" > "$W/p$pass-reroute.log" 2>&1 || true
    grep -E "^\s+(Single-ended|Multi-point):" "$HOME/.cache/pcbkit/pp-48-unit/reroute/sig.log" | tail -2 || true
    cp "$B" "$W/p$pass-rerouted.kicad_pcb"
    py "$T/unitsplit.py" join "$W/p$pass-rerouted.kicad_pcb" "$W/p$pass-joined.kicad_pcb"
    py "$T/route_unit.py" "$W/p$pass-joined.kicad_pcb" "$W/p$pass-out.kicad_pcb"
    CUR=$W/p$pass-out.kicad_pcb
done
cp "$CUR" "$B"
echo "done: $B (run DRC: kicad-cli pcb drc --refill-zones --schematic-parity --severity-all $B)"
