#!/usr/bin/env bash
# mz.sh <in> <out>: gndstitch, DRC, keepouts on, mazefix --in1, keepouts off, DRC
set -e
S=$(dirname "$2"); T=/home/delandtj/Granite/Electronics/granite/power/pp-48-fd/tools; D=/home/delandtj/Granite/Electronics/granite/power/pp-48-unit-fd
py() { python3 "$@" 2>&1 | grep -v "Debug\|memory leak" || true; }
py $T/gndstitch.py "$1" "$S/_m1.kicad_pcb" | tail -1
cp $D/pp-48-unit-fd.kicad_pro $S/_m1.kicad_pro; cp $D/pp-48-unit-fd.kicad_dru $S/_m1.kicad_dru
kicad-cli pcb drc --severity-error --refill-zones -o $S/_m1.rpt $S/_m1.kicad_pcb >/dev/null 2>&1 || true
py $S/keep.py add $S/_m1.kicad_pcb $S/_m2.kicad_pcb
py $T/mazefix.py $S/_m2.kicad_pcb $S/_m1.rpt $S/_m3.kicad_pcb --in1 | tail -8
py $S/keep.py del $S/_m3.kicad_pcb "$2"
cp $D/pp-48-unit-fd.kicad_pro "${2%.kicad_pcb}.kicad_pro"; cp $D/pp-48-unit-fd.kicad_dru "${2%.kicad_pcb}.kicad_dru"
kicad-cli pcb drc --severity-error --refill-zones -o "$2.rpt" "$2" 2>&1 | grep Found
grep "^\[" "$2.rpt" | sed 's/(.*//' | sort | uniq -c
