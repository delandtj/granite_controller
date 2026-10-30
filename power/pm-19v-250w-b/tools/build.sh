#!/usr/bin/env bash
# PM-19V-250W-B board flow, end to end (run from the project root):
#   tools/build.sh [scratch dir] [--route]
# schematic -> netlist -> mkboard -> place -> setrules -> power copper;
# --route also runs krt-route (2026-10-30: KRT leaves 9-11 nets open round
# U1, see docs/pm-19v-250w-b.md section 8; the committed board stops before it).
set -euo pipefail
cd "$(dirname "$0")/.."
S=${1:-${TMPDIR:-/tmp}/pm-19v-250w-b}
ROUTE=${2:-}
mkdir -p "$S"
PK=~/Electronics/pcbkit/bin/pcbkit
export PCBKIT_CONFIG=$PWD/pcbkit.toml
python3 tools/gensch.py
kicad-cli sch export netlist --format kicadxml -o "$S/net.xml" pm-19v-250w-b.kicad_sch >/dev/null
$PK mkboard "$S/net.xml" . "$S/raw.kicad_pcb"
python3 tools/place.py "$S/raw.kicad_pcb" "$S/placed.kicad_pcb"
cp "$S/placed.kicad_pcb" pm-19v-250w-b.kicad_pcb
$PK setrules pm-19v-250w-b.kicad_pcb pm-19v-250w-b.kicad_pro
python3 tools/power.py pm-19v-250w-b.kicad_pcb "$S/power.kicad_pcb"
if [ -f tools/route_hand.py ]; then
    python3 tools/route_hand.py "$S/power.kicad_pcb" "$S/hand.kicad_pcb"
    cp "$S/hand.kicad_pcb" pm-19v-250w-b.kicad_pcb
else
    cp "$S/power.kicad_pcb" pm-19v-250w-b.kicad_pcb
fi
if [ "$ROUTE" = "--route" ]; then
    $PK krt-route > "$S/krt.log" 2>&1 || true
    grep -E "Single-ended|Multi-point|JSON_SUMMARY_MIN" "$S/krt.log" | tail -3
fi
kicad-cli pcb drc --schematic-parity --severity-error --refill-zones -o "$S/drc.rpt" pm-19v-250w-b.kicad_pcb | grep Found
