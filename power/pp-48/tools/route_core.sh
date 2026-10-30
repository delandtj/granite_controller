#!/usr/bin/env bash
# Route the PP-48 core board (docs/adr/0005): run in power/pp-48-core.
#
#   ../pp-48/tools/route_core.sh <placed.kicad_pcb> [NET ...]
#
# placed board (tools/place_core.py output) -> route_core.py (hand copper: island, pocket, bands, GND
# pours) -> krt-route (VIN plane on In2, the rest on F / B) -> route_core.py again. Then up to $PASSES
# (default three) passes: DRC, drop the router's copper on the nets that collide or are open, reroute
# them with --keep, route_core.py once more. The extra NETs join the first pass. Then up to three
# tools/mazefix.py rounds on what is still open (IN1=1: hops on In1 allowed), pcbkit prune (dangling
# stubs), tools/mergetracks.py (fewer, longer segments) and the zone fill. Result: pp-48-core.kicad_pcb. Scratch: $SCRATCH (default
# ~/.cache/pcbkit/pp-48-core/flow). HOLD="NET ..." keeps those nets out of the passes.
set -euo pipefail
[ $# -ge 1 ] || { echo "usage: route_core.sh <placed.kicad_pcb> [NET ...]" >&2; exit 2; }
PLACED=$(realpath "$1"); shift
T=$(cd "$(dirname "$0")" && pwd)
PK=${PCBKIT:-$HOME/Electronics/pcbkit/bin/pcbkit}
W=${SCRATCH:-$HOME/.cache/pcbkit/pp-48-core/flow}
B=pp-48-core.kicad_pcb
export PCBKIT_CONFIG=$PWD/pcbkit.toml
mkdir -p "$W"
py() { python3 "$@" 2>&1 | grep -v "Debug\|memory leak" || true; }

# nets to reroute: from a DRC report (shorts, clearance, crossings, opens) plus the given ones
nets_from() {
    python3 - "$@" <<'PY'
import os, re, sys
txt = open(sys.argv[1]).read()
nets = set(sys.argv[2:])
hold = set(os.environ.get('HOLD', '').split())
for block in re.split(r'\n(?=\[)', txt):
    head = block.split('\n', 1)[0]
    if head.startswith(('[shorting_items]', '[clearance]', '[tracks_crossing]', '[unconnected_items]')):
        for n in re.findall(r'\[([^\]]+)\] (?:on|of)', block):
            if n not in ('GND', 'VIN', 'CTRL_GND') and n not in hold:
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

py "$T/route_core.py" "$PLACED" "$W/1-hand.kicad_pcb"
cp "$W/1-hand.kicad_pcb" "$B"
"$PK" krt-route > "$W/krt-route.log" 2>&1 || true
grep -E "^\s+(Single-ended|Multi-point):" "$W/krt-route.log" | tail -2 || true
cp "$B" "$W/2-routed.kicad_pcb"
py "$T/route_core.py" "$W/2-routed.kicad_pcb" "$W/3-redrawn.kicad_pcb"
CUR=$W/3-redrawn.kicad_pcb
for pass in $(seq 1 "${PASSES:-3}"); do
    cp "$CUR" "$W/p$pass-in.kicad_pcb"
    cp pp-48-core.kicad_pro "$W/p$pass-in.kicad_pro"
    kicad-cli pcb drc --refill-zones --severity-error -o "$W/p$pass.rpt" "$W/p$pass-in.kicad_pcb" > /dev/null 2>&1 || true
    EXTRA=(); [ "$pass" -eq 1 ] && EXTRA=("$@")
    mapfile -t NETS < <(nets_from "$W/p$pass.rpt" "${EXTRA[@]}")
    echo "pass $pass rerouting: ${NETS[*]:-none}"
    [ ${#NETS[@]} -gt 0 ] || break
    clear_nets "$W/p$pass-in.kicad_pcb" "$W/p$pass-cleared.kicad_pcb" "${NETS[@]}"
    cp "$W/p$pass-cleared.kicad_pcb" "$B"
    "$PK" krt-reroute --keep "${NETS[@]}" > "$W/p$pass-reroute.log" 2>&1 || true
    grep -E "^\s+(Single-ended|Multi-point):" "$HOME/.cache/pcbkit/pp-48-core/reroute/sig.log" | tail -2 || true
    cp "$B" "$W/p$pass-rerouted.kicad_pcb"
    py "$T/route_core.py" "$W/p$pass-rerouted.kicad_pcb" "$W/p$pass-out.kicad_pcb"
    CUR=$W/p$pass-out.kicad_pcb
done
# what is still open: tools/mazefix.py (grid maze on F / B; IN1=1 also lets it hop on In1), DRC between rounds
for r in 1 2 3; do
    cp "$CUR" "$W/m$r-in.kicad_pcb"
    cp pp-48-core.kicad_pro "$W/m$r-in.kicad_pro"
    kicad-cli pcb drc --refill-zones --severity-error -o "$W/m$r.rpt" "$W/m$r-in.kicad_pcb" > /dev/null 2>&1 || true
    grep -q '^\[unconnected_items\]' "$W/m$r.rpt" || break
    py "$T/mazefix.py" "$W/m$r-in.kicad_pcb" "$W/m$r.rpt" "$W/m$r-out.kicad_pcb" ${IN1:+--in1}
    CUR=$W/m$r-out.kicad_pcb
done
# dangling stubs (only when nothing is open: on an open route prune eats both halves)
cp "$CUR" "$W/prune.kicad_pcb"
cp pp-48-core.kicad_pro "$W/prune.kicad_pro"
kicad-cli pcb drc --refill-zones --severity-all -o "$W/prune.rpt" "$W/prune.kicad_pcb" > /dev/null 2>&1 || true
if ! grep -q '^\[unconnected_items\]' "$W/prune.rpt"; then
    "$PK" prune "$W/prune.kicad_pcb" "$W/prune.rpt" 2>&1 | tail -1 || true
    CUR=$W/prune.kicad_pcb
fi
# tidy: the routers' 45-degree staircases redrawn with fewer segments (DRC-checked against its input)
py "$T/mergetracks.py" "$CUR" "$W/tidy.kicad_pcb" --drc "$PWD/pp-48-core.kicad_pro"
CUR=$W/tidy.kicad_pcb
# fill the zones (saved in the scratch dir: pcbnew's Save rewrites the .kicad_pro beside its output)
python3 - "$CUR" "$W/final.kicad_pcb" <<'PY' 2>&1 | grep -v "Debug\|memory leak" || true
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1])
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
b.Save(sys.argv[2])
PY
cp "$W/final.kicad_pcb" "$B"
echo "done: $B (run DRC: kicad-cli pcb drc --refill-zones --schematic-parity --severity-all $B)"
