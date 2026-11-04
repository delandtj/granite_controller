#!/usr/bin/env bash
# Rebuild controller-fd/granite-controller-fd.kicad_pcb from the routed rev D
# board (controller/granite-controller.kicad_pcb) and the FD schematic.
#
#   tools/fd_build.sh            (from anywhere; writes the board in place)
#
# Steps: netlist -> fd_board.py (drop the I2C link, place J15 / CAN parts,
# move J9) -> pcbkit relink -> fd_hand.py (plane welds) -> pcbkit viaout ->
# krt-reroute (AUX_12V steered by fd_keepout.py, DRY_IN3, then the new / ripped
# nets, retries on what stays open) -> fd_post.py + viaout -> prune -> silk -> connlabels -> DRC.
# FD_RESUME=1 redoes only the steps after routing. Scratch: $FD_SCRATCH
# (default ~/.cache/pcbkit/granite-controller-fd/build): pcbnew rewrites the
# .kicad_pro next to every board it saves, so nothing is saved in the project
# but the final copy.
set -euo pipefail
PRJ=$(cd "$(dirname "$0")/.." && pwd)
NAME=granite-controller-fd
S=${FD_SCRATCH:-$HOME/.cache/pcbkit/$NAME/build}
P=${PCBKIT:-$HOME/Electronics/pcbkit/bin/pcbkit}
cd "$PRJ"
if [ -n "${FD_RESUME:-}" ]; then        # finish from the last routed board
    mkdir -p "$S"/{d,e,f}
    cp "$S/routed.kicad_pcb" $NAME.kicad_pcb
else
rm -rf "$S" && mkdir -p "$S"/{a,b,c,d,e}

kicad-cli sch export netlist --format kicadxml -o "$S/net.xml" $NAME.kicad_sch
python3 tools/fd_board.py ../controller/granite-controller.kicad_pcb "$S/net.xml" "$S/a/$NAME.kicad_pcb" 2>&1 | grep -v swig/
$P relink "$S/net.xml" "$S/a/$NAME.kicad_pcb" --save | tail -3
python3 tools/fd_hand.py "$S/a/$NAME.kicad_pcb"
$P viaout "$S/a/$NAME.kicad_pcb" "$S/b/$NAME.kicad_pcb" 2>&1 | grep -v swig/
# AUX_12V first, steered below U1 (fd_keepout.py), then the rest
python3 tools/fd_keepout.py add "$S/b/$NAME.kicad_pcb"
cp "$S/b/$NAME.kicad_pcb" $NAME.kicad_pcb
KRT_EXTRA="--max-ripup 0" $P krt-reroute --keep AUX_12V | tail -1
cp $NAME.kicad_pcb "$S/c/$NAME.kicad_pcb"
python3 tools/fd_keepout.py del "$S/c/$NAME.kicad_pcb"
cp "$S/c/$NAME.kicad_pcb" $NAME.kicad_pcb
# DRY_IN3 (U1 pin 4, below the hand I2C escapes) next, before the rest
# boxes it in
KRT_EXTRA="--max-ripup 0" $P krt-reroute --keep DRY_IN3 | tail -1

# new nets, nets ripped by fd_board.py, the moved J9 group's nets; --keep:
# the W5500 SPI hand bus and +5V stay, only the open connections are routed
NETS=("/CAN link/CANH" "/CAN link/CANL" "/CAN link/CAN_RXD" "/CAN link/CAN_TXD"
      "/CAN link/CAN_SPLIT" "/CAN link/MCP_OSC1" "/CAN link/MCP_OSC2" "/CAN link/MCP_SDO"
      CAN_STB CAN_nCS CAN_nINT ETH_MISO ETH_MOSI ETH_SCLK +5V
      "/Sensors and dry contacts/HDR_3V3" "/Sensors and dry contacts/HDR_SDA"
      "/Sensors and dry contacts/HDR_SCL" I2C_LOC_SDA I2C_LOC_SCL
      /MCU/LED_HB /MCU/USB_D+ /MCU/USB_D- DRY_IN1 DRY_IN3 VIN_SENSE
      /Power/VIN_12V)         # D10 -> D12 went through the removed D11
cp $NAME.kicad_pcb "$S/pre-rest.kicad_pcb"
KRT_EXTRA="--max-ripup 0" $P krt-reroute --keep "${NETS[@]}" | tail -3

# what KRT left open: clear it with the nets that boxed it in (of NETS only:
# the rev D copper stays) and route that set again, up to four times
open_nets() {
    python3 - "$HOME/.cache/pcbkit/$NAME/reroute/sig.json" "${NETS[@]}" "$@" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
mine = set(sys.argv[2:]) - {"I2C_LOC_SCL", "I2C_LOC_SDA", "DRY_IN3"}   # hand escapes (fd_hand.py) stay
bad = [p["net"] for p in d.get("pad_pairs_open", [])]
blk = {b["net"] for x in d.get("blockers", []) if x["net"] in bad for b in x.get("blocked_by", [])}
keep = "--all" not in sys.argv
out = list(dict.fromkeys([n for n in bad if n in mine or not keep] + sorted(blk & mine)))
if out:
    print("\n".join(out))
PY
}
for i in 1 2 3 4; do
    mapfile -t R < <(open_nets)
    mapfile -t A < <(open_nets --all)
    [ ${#A[@]} -eq 0 ] && break
    if [ ${#R[@]} -gt 0 ]; then
        echo "retry $i: ${R[*]}"
        KRT_EXTRA="--max-ripup 0" $P krt-reroute "${R[@]}" | tail -1
    else
        echo "retry $i (keep): ${A[*]}"
        KRT_EXTRA="--max-ripup 0" $P krt-reroute --keep "${A[@]}" | tail -1
    fi
done
mapfile -t R < <(open_nets --all)
[ ${#R[@]} -eq 0 ] || { echo "still open: ${R[*]}"; exit 1; }
cp $NAME.kicad_pcb "$S/routed.kicad_pcb"
fi

mkdir -p "$S/g"
cp $NAME.kicad_pcb "$S/g/in.kicad_pcb"
python3 tools/fd_post.py "$S/g/in.kicad_pcb"
$P viaout "$S/g/in.kicad_pcb" "$S/g/$NAME.kicad_pcb" 2>&1 | grep -v swig/
cp "$S/g/$NAME.kicad_pcb" $NAME.kicad_pcb

# leftovers: rev D copper islands at removed / moved pads (all nets routed
# now, so dangling items are islands, not open routes)
for i in 1 2 3 4 5 6; do
    kicad-cli pcb drc --severity-all --refill-zones -o "$S/prune.rpt" $NAME.kicad_pcb >/dev/null
    grep -q '_dangling' "$S/prune.rpt" || break
    cp $NAME.kicad_pcb "$S/d/$NAME.kicad_pcb"
    $P prune "$S/d/$NAME.kicad_pcb" "$S/prune.rpt" | tail -1
    cp "$S/d/$NAME.kicad_pcb" $NAME.kicad_pcb
done
# rev D's GND pours and stitching stay (a gndpour re-run starves J10 pin 5's
# thermal and cuts J7 pin 1 off the +3V3 plane); stitching vias that hit new
# parts went in fd_board.py
cp $NAME.kicad_pcb "$S/e/$NAME.kicad_pcb"
$P silk "$S/e/$NAME.kicad_pcb" "$S/e/silk.kicad_pcb" | tail -2
mkdir -p "$S/f"
$P connlabels "$S/e/silk.kicad_pcb" "$S/f/$NAME.kicad_pcb" | tail -2
cp "$S/f/$NAME.kicad_pcb" $NAME.kicad_pcb
kicad-cli pcb drc --schematic-parity --severity-all --refill-zones --format json -o "$S/final.json" $NAME.kicad_pcb | tail -2
