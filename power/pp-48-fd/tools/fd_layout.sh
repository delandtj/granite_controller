#!/usr/bin/env bash
# CAN FD plane boards (docs/adr/0006-plane-link-can-fd.md): the rev D layouts brought onto the CAN FD
# schematics, keeping the rev D placement and copper of everything that did not change. This is the
# sequence the boards of 2026-11-04 were made with; run from power/.
#
#   pp-48-fd/tools/fd_layout.sh core|unit <scratch dir>
#
# core: rev D board -> fdsync.py (parts in / out / swapped, nets) -> place_core.py (CAN FD tables: link
#   connectors at v 30, link domain parts, Fly-Buck primary turned under the strip, MCP2518FD) ->
#   fdoutline.py (68 x 72 mm, x 43.5-111.5) -> fdstrip.py fd/core-regions.json (router copper of moved parts
#   and in the link domain / bands; the VIN plane outline) -> route_core.py (hand copper: LINK_GND pours,
#   isolation band, pocket, GND pours) -> krt-reroute --keep (max-ripup 0) on the listed nets -> krt-reroute
#   VIN (the Fly-Buck's VIN pads) -> solid U4.7 / C23.2 (place_core.py SOLID) -> gndstitch.py (LINK_GND /
#   GND islands) -> route_core.py (adds the pocket) -> krt-reroute /Supplies/SW2 (out of the pocket) ->
#   prune (3 rounds) -> pcbkit silk -> nosilk J2 / J3 / J301. Checks: isocheck.py <filled board> 2.0.
#   Termination header (2026-11-04, ADR 0006 decision 8: 2x2 jumper-cap header JP1 for the solder jumpers
#   JP1 / JP2) and J2's pin 1 silk mark: on the routed board, fdsync.py -> fd/core_term.py (places D9 / JP1 /
#   R25 / R26 / C32 from place_core.py, redraws the channel's CAN_P / CAN_N / TERM_* / AUX_12V copper and the
#   mark); a fresh run of the sequence above places them by place_core.py and lets KRT route them instead.
# unit: rev D board -> fdsync.py -> place_unit.py (J303 / J304 at v 30, C201 turned, C202 / H3 / Q10 / R31 /
#   R58 moved, termination in the margin) -> fdstrip.py fd/unit-regions.json -> route_unit.py (link() band
#   on B, link_term(), margin_press(), EXP_RSTN jog, strip vias) -> solid C201.2. Then by hand, with the
#   scripts kept in fd/unit_*: pass.sh (keepouts of keep.py round the link copper while KRT runs, unitsplit
#   split / krt-reroute --keep / join, route_unit.py) on the fdstrip net list with KRT_EXTRA="--max-ripup 0";
#   vbus.py (three slot VBUS vias lifted out of the band); clear.py + pass.sh on what DRC still lists
#   (A_OVLO, B_OVLO, SEG_SCL / SDA, B_DISARM, PRESS_EN, B_UVLO, B_ARMS_N, B_CLRN, B_OVK, UVLO); mz.sh
#   (gndstitch, mazefix --in1 inside the keepouts); patch1.py (A_OVLO In1 hop at R5); padtap.py (group
#   "fd-taps": GND vias for Q204.2 / R214.2, U51.5's GND track); prune2.py (dangling stubs; the SEG_SCL
#   junction via at 139.7 / 145.7 replaced by a track), pcbkit silk. Decision 12's 0.5 mm: rule "link to
#   bus" in pp-48-unit-fd.kicad_dru (net class LINK of the .kicad_pro), checked by DRC; KRT does not read
#   it, hence the keepouts while it runs. Termination header (2026-11-04): JP301 / JP302 -> the 2x2 jumper-cap
#   header JP301, on the routed board by fdsync.py -> fd/unit_term.py (place_unit.py TERM + route_unit.py
#   link_term(), old term copper deleted first); a fresh run gets both from the steps above.
# The KRT steps are not bit-for-bit repeatable; the scripted steps are.
set -euo pipefail
B=${1:?core|unit}; W=${2:?scratch dir}
T=$(cd "$(dirname "$0")" && pwd)
PK=${PCBKIT:-$HOME/Electronics/pcbkit/bin/pcbkit}
P=$(cd "$T/../.." && pwd)
mkdir -p "$W"
py() { python3 "$@" 2>&1 | grep -v "Debug\|memory leak" || true; }
D=$P/pp-48-$B-fd; N=pp-48-$B-fd
kicad-cli sch export netlist --format kicadxml -o "$W/net.xml" "$D/$N.kicad_sch" > /dev/null
py "$T/fdsync.py" "$P/../attic/power/pp-48-$B/pp-48-$B.kicad_pcb" "$W/net.xml" "$D" "$W/1-sync.kicad_pcb"
if [ "$B" = core ]; then
    py "$T/place_core.py" "$W/1-sync.kicad_pcb" "$W/2-placed.kicad_pcb"
    py "$T/fdoutline.py" "$W/2-placed.kicad_pcb" "$W/3-outline.kicad_pcb" 43.5 100 111.5 172
    python3 "$T/fdstrip.py" "$P/../attic/power/pp-48-core/pp-48-core.kicad_pcb" "$W/3-outline.kicad_pcb" "$W/4-strip.kicad_pcb" \
        "$T/fd/core-regions.json" --zone VIN 43.8,100.3,111.2,171.7 2> /dev/null > "$W/nets.txt"
    py "$T/route_core.py" "$W/4-strip.kicad_pcb" "$W/5-hand.kicad_pcb"
    cp "$W/5-hand.kicad_pcb" "$D/$N.kicad_pcb"
    (cd "$D" && KRT_EXTRA="--max-ripup 0" "$PK" krt-reroute --keep $(cat "$W/nets.txt") && \
        KRT_EXTRA="--max-ripup 0" "$PK" krt-reroute --keep VIN)
    py "$T/gndstitch.py" "$D/$N.kicad_pcb" "$W/6-stitch.kicad_pcb"
    py "$T/route_core.py" "$W/6-stitch.kicad_pcb" "$W/7-hand.kicad_pcb"
    cp "$W/7-hand.kicad_pcb" "$D/$N.kicad_pcb"
    (cd "$D" && KRT_EXTRA="--max-ripup 0" "$PK" krt-reroute /Supplies/SW2)
else
    py "$T/place_unit.py" "$W/1-sync.kicad_pcb" "$W/2-placed.kicad_pcb"
    python3 "$T/fdstrip.py" "$P/../attic/power/pp-48-unit/pp-48-unit.kicad_pcb" "$W/2-placed.kicad_pcb" "$W/4-strip.kicad_pcb" \
        "$T/fd/unit-regions.json" 2> /dev/null > "$W/nets.txt"
    py "$T/route_unit.py" "$W/4-strip.kicad_pcb" "$W/5-hand.kicad_pcb"
    (cd "$D" && RESUME=1 PASSES=4 SCRATCH="$W/flow" "$T/route_unit.sh" "$W/5-hand.kicad_pcb")
fi
echo "then: DRC, pcbkit prune until no dangling items, pcbkit silk, DRC (see the header)"
