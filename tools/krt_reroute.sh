#!/usr/bin/env bash
# Targeted KRT pass: clear and reroute only the named nets, keep everything else.
#
#   tools/krt_reroute.sh [--keep] NET [NET ...]
#
# --keep: do not clear the nets first; KRT treats existing copper as fixed and
# routes only what is still open (use after laying escapes by script).
#
# Use after moving a few parts: list every non-GND net on the moved parts plus
# the nets DRC reports open. All copper of those nets is deleted first (KRT's
# --rip-existing-nets leaves stubs at old pad positions), then:
#   1. route.py on the named nets, F.Cu/B.Cu only
#   2. route.py GND without rip-up (closes GND pads the move opened)
#   3. kicad-cli DRC with --refill-zones --save-board (KRT does not refill)
# The refilled board is copied back. GND stubs left at moved pads are not
# cleaned here; DRC shows them as track_dangling.
#
# Env: same as krt_route.sh (KRT_DIR, KRT_PY); KRT_WORK defaults to
#      ~/.cache/granite-pcb/reroute and must be outside the repo.
set -euo pipefail
KEEP=0
[ "${1:-}" = --keep ] && { KEEP=1; shift; }
[ $# -gt 0 ] || { echo "usage: $0 [--keep] NET [NET ...]" >&2; exit 2; }

REPO=$(cd "$(dirname "$0")/.." && pwd)
BASE=${HOME}/.cache/graver-pcb/routetest
KRT_DIR=${KRT_DIR:-$BASE/KiCadRoutingTools}
KRT_PY=${KRT_PY:-$BASE/venv/bin/python}
WORK=${KRT_WORK:-${HOME}/.cache/granite-pcb/reroute}
case "$WORK" in "$REPO"*) echo "KRT_WORK must be outside the repo" >&2; exit 1 ;; esac

rm -rf "${WORK:?}" && mkdir -p "$WORK/drc"
for d in "$WORK" "$WORK/drc"; do
    cp "$REPO/granite_controller.kicad_pro" "$REPO/granite_controller.kicad_dru" "$d/"
done
B=$WORK/granite_controller.kicad_pcb
"$KRT_PY" - "$REPO/granite_controller.kicad_pcb" "$B" "$KEEP" "$@" <<'PY'
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1])
rip = set() if sys.argv[3] == "1" else set(sys.argv[4:])
t = b.Tracks()   # index: SWIG iterators break on Python 3.14
gone = [t[i] for i in range(len(t)) if t[i].GetNetname() in rip]
for it in gone:
    b.Delete(it)
b.Save(sys.argv[2])
print("cleared", len(gone), "tracks/vias")
PY
COMMON=(--layers F.Cu B.Cu --track-width 0.2 --clearance 0.2 --via-size 0.5 --via-drill 0.2
        --strict-sizes --escalation off --keep-input-copper)
POWER=(--power-nets GND +3V3 +5V "*BUCK_5V" "*VIN_19V" "*VIN_RAW" VBUS "*SW_5V" "*SW_3V3"
        --power-nets-widths 0.4 0.4 0.5 0.5 0.4 0.4 0.5 0.5 0.5)

cd "$KRT_DIR"
rc=0
"$KRT_PY" py_router/route.py "$B" "$WORK/1-sig.kicad_pcb" --nets "$@" "${COMMON[@]}" "${POWER[@]}" \
    --json-out "$WORK/sig.json" > "$WORK/sig.log" 2>&1 || rc=$?
[ "$rc" -eq 0 ] || [ "$rc" -eq 3 ] || { echo "signal pass failed ($rc), see $WORK/sig.log" >&2; exit "$rc"; }
rc=0
"$KRT_PY" py_router/route.py "$WORK/1-sig.kicad_pcb" "$WORK/2-gnd.kicad_pcb" --nets GND \
    --power-nets GND --power-nets-widths 0.4 "${COMMON[@]}" \
    --json-out "$WORK/gnd.json" > "$WORK/gnd.log" 2>&1 || rc=$?
[ "$rc" -eq 0 ] || [ "$rc" -eq 3 ] || { echo "GND pass failed ($rc), see $WORK/gnd.log" >&2; exit "$rc"; }

cp "$WORK/2-gnd.kicad_pcb" "$WORK/drc/granite_controller.kicad_pcb"
cd "$WORK/drc"
kicad-cli pcb drc --severity-all --refill-zones --save-board -o drc.rpt granite_controller.kicad_pcb \
    2>&1 | grep Found || true
cp "$WORK/drc/granite_controller.kicad_pcb" "$REPO/granite_controller.kicad_pcb"
echo "board copied back; now run: kicad-cli pcb drc --schematic-parity --severity-all granite_controller.kicad_pcb"
