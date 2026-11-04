#!/usr/bin/env bash
# pass.sh <in.kicad_pcb> <out.kicad_pcb> NET...   split -> krt-reroute --keep (KRT_EXTRA from env) -> join -> route_unit.py
set -euo pipefail
T=/home/delandtj/Granite/Electronics/granite/power/pp-48-fd/tools
D=/home/delandtj/Granite/Electronics/granite/power/pp-48-unit-fd
IN=$1; OUT=$2; shift 2
W=$(dirname "$OUT")
py() { python3 "$@" 2>&1 | grep -v "Debug\|memory leak" || true; }
py $W/keep.py add "$IN" "$W/_keep.kicad_pcb"
py $T/unitsplit.py split "$W/_keep.kicad_pcb" "$W/_split.kicad_pcb" >/dev/null
cp "$W/_split.kicad_pcb" "$D/pp-48-unit-fd.kicad_pcb"
mapfile -t SPLIT < <(cd $T && python3 - "$@" <<'PY' 2>&1 | grep -v "Debug\|memory leak"
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
)
(cd $D && ~/Electronics/pcbkit/bin/pcbkit krt-reroute --keep "${SPLIT[@]}" > "$W/_krt.log" 2>&1 || true)
grep -E "^\s+(Single-ended|Multi-point):" ~/.cache/pcbkit/pp-48-unit-fd/reroute/sig.log | tail -2 || true
cp "$D/pp-48-unit-fd.kicad_pcb" "$W/_rerouted.kicad_pcb"
py $T/unitsplit.py join "$W/_rerouted.kicad_pcb" "$W/_joined.kicad_pcb" >/dev/null
py $W/keep.py del "$W/_joined.kicad_pcb" "$W/_unkeep.kicad_pcb"
py $T/route_unit.py "$W/_unkeep.kicad_pcb" "$OUT"
cp "$OUT" "$D/pp-48-unit-fd.kicad_pcb"
cd $D && kicad-cli pcb drc --severity-error --refill-zones -o "$OUT.rpt" pp-48-unit-fd.kicad_pcb 2>&1 | grep Found
