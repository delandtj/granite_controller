#!/usr/bin/env bash
# PM-19V-250W-B board flow, end to end (run from the project root):
#   tools/build.sh [scratch dir] [--route]
# schematic -> netlist -> mkboard -> place -> setrules -> power copper;
# --route then runs what made the board (2026-10-30): krt-route, two
# krt-reroute passes on what it left open, gndpour, silk, GND pours solid
# (no starved thermals on J1's via rings / U1's thermal vias), prune, DRC.
# KRT is not guaranteed to repeat itself: check the DRC line at the end.
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
if false; then   # hand routes live in tools/power.py (group "hand")
    python3 tools/route_hand.py "$S/power.kicad_pcb" "$S/hand.kicad_pcb"
    cp "$S/hand.kicad_pcb" pm-19v-250w-b.kicad_pcb
else
    cp "$S/power.kicad_pcb" pm-19v-250w-b.kicad_pcb
fi
if [ "$ROUTE" = "--route" ]; then
    $PK krt-route > "$S/krt.log" 2>&1 || true
    grep -E "Single-ended|Multi-point" "$S/krt.log" | tail -2
    $PK krt-reroute /COMP /RUN /VAUX > "$S/rr1.log" 2>&1 || true
    $PK krt-reroute --keep /RUN /FB /OVS > "$S/rr2.log" 2>&1 || true
    $PK gndpour pm-19v-250w-b.kicad_pcb "$S/gnd.kicad_pcb"
    $PK silk "$S/gnd.kicad_pcb" "$S/silk.kicad_pcb"
    cp "$S/silk.kicad_pcb" pm-19v-250w-b.kicad_pcb
    python3 - <<'PY'
import pcbnew
b = pcbnew.LoadBoard("pm-19v-250w-b.kicad_pcb")
for z in b.Zones():
    if z.GetNetname() == "GND" and z.GetAssignedPriority() == 1:     # gndpour's outer pours
        z.SetPadConnection(pcbnew.ZONE_CONNECTION_FULL)
pcbnew.ZONE_FILLER(b).Fill(b.Zones())
b.Save("pm-19v-250w-b.kicad_pcb")
PY
    for i in 1 2; do
        kicad-cli pcb drc --schematic-parity --severity-all --refill-zones -o "$S/drc.rpt" pm-19v-250w-b.kicad_pcb >/dev/null 2>&1 || true
        $PK prune pm-19v-250w-b.kicad_pcb "$S/drc.rpt"
    done
fi
kicad-cli pcb drc --schematic-parity --severity-all --refill-zones -o "$S/drc.rpt" pm-19v-250w-b.kicad_pcb 2>/dev/null | grep Found
