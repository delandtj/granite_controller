#!/usr/bin/env bash
# Route the board with KiCadRoutingTools (drandyhaas), as a first pass for hand
# routing.
#
#   tools/krt_route.sh            # route a copy, then copy the .kicad_pcb back
#
# Starts from the board as it is (hand placement kept). Existing tracks, vias
# and zones are stripped first; nothing else is touched.
#
# Chain (KRT README, #562): planes first, then diff pairs, then everything.
# KRT rewrites the .kicad_pro next to the board it routes ("FAB FLOOR
# RELAXED") and grades itself against that, so it runs on a copy outside the
# repo with --escalation off --strict-sizes, and only the .kicad_pcb comes
# back. Grade with `kicad-cli pcb drc --schematic-parity` in the repo, never
# with KRT's own checks.
#
# Env: KRT_DIR (checkout), KRT_PY (venv python with pcbnew visible),
#      KRT_WORK (scratch dir, must be outside the repo).
set -euo pipefail

REPO=$(cd "$(dirname "$0")/.." && pwd)
BASE=${HOME}/.cache/graver-pcb/routetest
KRT_DIR=${KRT_DIR:-$BASE/KiCadRoutingTools}
KRT_PY=${KRT_PY:-$BASE/venv/bin/python}
WORK=${KRT_WORK:-${HOME}/.cache/granite-pcb/route}
case "$WORK" in "$REPO"*) echo "KRT_WORK must be outside the repo" >&2; exit 1 ;; esac

rm -rf "$WORK" && mkdir -p "$WORK"
for f in granite_controller.kicad_pcb granite_controller.kicad_pro granite_controller.kicad_dru; do
    cp "$REPO/$f" "$WORK/"
done
B=$WORK/granite_controller.kicad_pcb
"$KRT_PY" - "$B" <<'PY'
import sys, pcbnew
b = pcbnew.LoadBoard(sys.argv[1])
t = b.Tracks(); z = b.Zones()   # index: SWIG iterators break on Python 3.14
for it in [t[i] for i in range(len(t))] + [z[i] for i in range(len(z))]:
    b.Delete(it)
b.Save(sys.argv[1])
print("stripped old copper")
PY
COMMON=(--clearance 0.2 --via-size 0.5 --via-drill 0.2 --strict-sizes
        --same-net-pad-clearance 0.15)   # no vias in SMD pads

cd "$KRT_DIR"
echo "== planes: GND on In1.Cu, +3V3 on In2.Cu"
"$KRT_PY" py_router/route_planes.py "$B" "$WORK/1-planes.kicad_pcb" \
    --nets GND +3V3 --plane-layers In1.Cu In2.Cu --via-size 0.6 --via-drill 0.3

echo "== diff pairs: Ethernet MDI 0.16/0.15 (101R in fluid)"
"$KRT_PY" py_router/route_diff.py "$WORK/1-planes.kicad_pcb" --output "$WORK/2-eth.kicad_pcb" \
    --nets "*ETH_TX*" "*ETH_RX*" --track-width 0.16 --diff-pair-gap 0.15 "${COMMON[@]}"

echo "== diff pairs: USB 0.25/0.15 (89R in air)"
"$KRT_PY" py_router/route_diff.py "$WORK/2-eth.kicad_pcb" --output "$WORK/3-usb.kicad_pcb" \
    --nets "*USB*D+" "*USB*D-" --track-width 0.25 --diff-pair-gap 0.15 "${COMMON[@]}"

echo "== everything else (signals on F.Cu/B.Cu only: In1/In2 stay solid planes)"
rc=0
"$KRT_PY" py_router/route.py "$WORK/3-usb.kicad_pcb" "$WORK/4-routed.kicad_pcb" --nets "*" \
    --layers F.Cu B.Cu \
    --track-width 0.2 "${COMMON[@]}" --escalation off --ordering mps --keep-input-copper \
    --power-nets GND +3V3 +5V "*BUCK_5V" "*VIN_19V" "*VIN_RAW" VBUS "*SW_5V" "*SW_3V3" \
    --power-nets-widths 0.4 0.4 0.5 0.5 0.4 0.4 0.5 0.5 0.5 \
    --json-out "$WORK/route.json" || rc=$?
# exit 3 = finished but incomplete or below a requested size; still worth importing
[ "$rc" -eq 0 ] || [ "$rc" -eq 3 ] || { echo "route.py failed ($rc)" >&2; exit "$rc"; }
echo "route.py exit $rc"

cp "$WORK/4-routed.kicad_pcb" "$REPO/granite_controller.kicad_pcb"
echo "routed board copied back; now run: kicad-cli pcb drc --schematic-parity --severity-all granite_controller.kicad_pcb"
