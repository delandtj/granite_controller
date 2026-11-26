#!/bin/bash
# Re-renders the layout page's pictures (sheets 8 to 11) from the CAD in mech/frame.
# Usage: tools/render_page.sh [workdir]   (workdir defaults to a temp folder)
# Run the generators first (export.py, infra_frame.py, end_frame.py, plane_carrier.py).
set -e
T=$(cd "$(dirname "$0")" && pwd)
FRAME=$(dirname "$T")
IMG=$(dirname "$FRAME")/pod-layout/img
W=${1:-$(mktemp -d)}
mkdir -p "$W"; cd "$W"
freecadcmd $T/tankview2.py 2>&1 | grep -E "Error" || true
(cd "$FRAME" && END_VIZ="$W/end_viz.step" freecadcmd end_frame.py 2>&1 | grep -E "Error" || true)
ROT_IN=end_viz.step ROT_OUT=end_tank.step freecadcmd $T/rot_step.py 2>&1 | grep -E "Error" || true
OUT="$W/carrier_half_up.step" freecadcmd $T/carrier_half_step.py 2>&1 | grep -E "Error" || true
C='--body CarrierHalfA:color=#e0b040 --body CarrierHalfB:color=#c89a30 --body Controller:color=#2e9e5b --body Ctl_RJ45:color=#9aa0a6 --body Ctl_RJ45_cable:color=#4a90d9 --body SwitchSupply_DQD50:color=#b04a4a --body Switch:color=#3b6fb5 --body Core:color=#d08a2e --body InfraFrame:color=#c9ccd1'
for n in 1 2 3 4; do C="$C --body Unit${n}_board:color=#2a7a4a --body Unit${n}_XT60_plug:color=#e8d44a --body Unit${n}_terminals:color=#d9534f"; done
I='--body InfraFrame:color=#c9ccd1 --body Switch:color=#3b6fb5 --body Core:color=#d08a2e --body Core_XT30_plug:color=#e8d44a --body Core_J3_link_plug:color=#7a5cc0 --body Core_J301_chain_plug:color=#7a5cc0 --body Core_J2_link_plug:color=#7a5cc0 --no-grid --no-axes'
snap() { /usr/bin/solidcraft-cli snapshot "$@" 2>&1 | grep -iE "error" || true; }
snap --in block_tank.step --out $IMG/block-plane.png --width 800 --height 620 --camera '{"target":[169,190,105],"yaw":3.85,"pitch":0.25,"distance":780}' $C --no-grid --no-axes
snap --in block_tank.step --out $IMG/block-side.png --width 800 --height 470 --camera '{"target":[218,205,105],"yaw":4.7124,"pitch":0,"distance":560}' $C --no-grid --no-axes
snap --in infra_tank.step --out $IMG/infra-front.png --width 470 --height 620 --camera '{"target":[108,18,105],"yaw":4.34,"pitch":0.2,"distance":450}' $I
snap --in infra_tank.step --out $IMG/infra-back.png --width 470 --height 620 --camera '{"target":[108,18,105],"yaw":-1.2,"pitch":0.2,"distance":450}' $I
snap --in end_tank.step --out $IMG/end-frame.png --width 470 --height 620 --camera '{"target":[108,25,105],"yaw":3.75,"pitch":0.45,"distance":540}' --body EndFrame:color=#e0b040 --body LastNode:color=#c9ccd1 --no-grid --no-axes
snap --in carrier_half_up.step --out $IMG/carrier-half.png --width 800 --height 620 --camera '{"target":[8,83,105],"yaw":-1.0,"pitch":0.3,"distance":330}' --body CarrierHalf:color=#e0b040 --no-grid --no-axes
snap --in carrier_half_up.step --out $IMG/carrier-clip.png --width 800 --height 560 --camera '{"target":[8,60,150],"yaw":-0.25,"pitch":0.15,"distance":95}' --body CarrierHalf:color=#e0b040 --no-grid --no-axes
ls -la $IMG | cut -c24-
