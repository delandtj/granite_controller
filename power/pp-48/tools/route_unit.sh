#!/usr/bin/env bash
# Route the PP-48 unit board (docs/adr/0005): run in power/pp-48-unit.
#
#   ../pp-48/tools/route_unit.sh <placed.kicad_pcb> [NET ...]
#
# placed board (tools/place_unit.py output) -> route_unit.py (hand copper) ->
# unitsplit.py split -> krt-route -> join -> route_unit.py again (krt-route
# strips some kept copper: plain hand vias with copper on one side). Then up to
# $PASSES (default three) passes: DRC, drop the router's copper on the nets that collide with
# the hand copper or are open, reroute them with --keep (hand copper fixed)
# on the split board, join, route_unit.py once more. The extra NETs join the
# first pass (router-only nets: no hand copper on them). Result:
# pp-48-unit.kicad_pcb, zones filled. Scratch: $SCRATCH (default ~/.cache/pcbkit/pp-48-unit/flow).
# PASSES=n sets the number of reroute passes (default 3); RESUME=1 takes the given
# board as an earlier result and only runs the passes on it (no route_unit.py / krt-route first);
# HOLD="NET ..." keeps those nets out of the passes (a +3V3 problem otherwise rips all of +3V3).
# Then up to three tools/mazefix.py rounds on what is still open (IN1=1: hops on In1 allowed), then
# tools/mergetracks.py (fewer, longer segments), then the zone fill.
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
import os, re, sys
txt = open(sys.argv[1]).read()
nets = set(sys.argv[2:])
hold = set(os.environ.get('HOLD', '').split())
for block in re.split(r'\n(?=\[)', txt):
    head = block.split('\n', 1)[0]
    if head.startswith(('[shorting_items]', '[clearance]', '[tracks_crossing]', '[unconnected_items]')):
        for n in re.findall(r'\[([^\]]+)\] (?:on|of)', block):
            if n not in ('GND', 'VBUS', 'VIN') and not n.startswith('/Input/USENSE') and n not in hold:
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

if [ -n "${RESUME:-}" ]; then
    CUR=$W/0-resume.kicad_pcb
    cp "$PLACED" "$CUR"
else
    py "$T/route_unit.py" "$PLACED" "$W/1-hand.kicad_pcb"
    py "$T/unitsplit.py" split "$W/1-hand.kicad_pcb" "$W/2-split.kicad_pcb"
    cp "$W/2-split.kicad_pcb" "$B"
    "$PK" krt-route > "$W/krt-route.log" 2>&1 || true
    grep -E "^\s+(Single-ended|Multi-point):" "$W/krt-route.log" | tail -2
    cp "$B" "$W/3-routed.kicad_pcb"
    py "$T/unitsplit.py" join "$W/3-routed.kicad_pcb" "$W/4-joined.kicad_pcb"
    py "$T/route_unit.py" "$W/4-joined.kicad_pcb" "$W/5-redrawn.kicad_pcb"
    CUR=$W/5-redrawn.kicad_pcb
fi
for pass in $(seq 1 "${PASSES:-3}"); do
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
# what is still open: tools/mazefix.py (grid maze on F / B; IN1=1 also lets it hop on In1, the GND plane,
# where F / B are full), DRC between rounds
for r in 1 2 3; do
    cp "$CUR" "$W/m$r-in.kicad_pcb"
    cp pp-48-unit.kicad_pro "$W/m$r-in.kicad_pro"
    kicad-cli pcb drc --refill-zones --severity-error -o "$W/m$r.rpt" "$W/m$r-in.kicad_pcb" > /dev/null 2>&1 || true
    grep -q '^\[unconnected_items\]' "$W/m$r.rpt" || break
    py "$T/mazefix.py" "$W/m$r-in.kicad_pcb" "$W/m$r.rpt" "$W/m$r-out.kicad_pcb" ${IN1:+--in1}
    CUR=$W/m$r-out.kicad_pcb
done
# tidy: the routers' 45-degree staircases redrawn with fewer segments (tools/mergetracks.py, DRC-checked
# against its input), so a trace can be dragged or rerouted by hand in KiCad
py "$T/mergetracks.py" "$CUR" "$W/tidy.kicad_pcb" --drc "$PWD/pp-48-unit.kicad_pro"
CUR=$W/tidy.kicad_pcb
# fill the zones (KRT leaves them empty: the pours would show as ratsnest in KiCad)
# (saved in the scratch dir: pcbnew's Save rewrites the .kicad_pro beside its output)
python3 - "$CUR" "$W/final.kicad_pcb" <<'PY' 2>&1 | grep -v "Debug\|memory leak" || true
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1])
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
b.Save(sys.argv[2])
PY
cp "$W/final.kicad_pcb" "$B"
echo "done: $B (run DRC: kicad-cli pcb drc --refill-zones --schematic-parity --severity-all $B)"
