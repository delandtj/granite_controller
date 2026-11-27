# Handoff: PRO9 immersion pod, frames and layout

Written 2026-11-19. Production date: 2026-12-16. About 3 weeks to order frames for 12 blocks
(16 will be asked for). All prints so far are PLA test prints; production prints are PETG by a
contact. PETG in the fluid (ThermaSafe R, PAO) is untested; no time for a long soak.

## What this is

3D-printed frames that hold AXB35-03 (Strix Halo) motherboards upright in a DCX PRO9 immersion
tank. A block = 1 infra frame + 8 node frames + 1 end frame, joined by dovetails, with a plane
carrier hooked on the bus bar side.

- CAD and generators: `mech/frame/` (the vendor board STEP, MoboRef.FCStd and the superseded FCStd saves are on disk but gitignored)
- Layout page (the canonical overview): https://claude.ai/artifact/EHnSezyqRZ5oaNb7SHNWJ6
  (version 23). Source copy: `mech/pod-layout/pod-layout.html` with `img/` and `smoke.js`.
- Moved into this repo (granite) on 2026-11-26; `~/Granite/Hardware` is a symlink to `mech/`.
- Tank spec sheet: `~/Granite/Documents/DCX_Specsheet_PRO9_V1.pdf`

## The one thing that bites: frame z points DOWN in the tank

A frame seated on its neighbours' dovetails has frame z = 0 at the TOP. Checked on printed parts.
The motherboard reference inside Frame.FCStd suggests the opposite; do not use it to decide "up".
Describe parts to the user the tank way up, and render them rotated 180 degrees about y.
Boards cannot be mirrored, so flipping "up" swaps which side their edge plugs point to.

Frame axes: x across (0 = bus bar side), y = stacking direction (0 = back = print bed side),
z = frame height axis.

## Current design (all regenerated 2026-11-19, all clash checks 0)

| Part | Generator | Print file | State |
|---|---|---|---|
| Node frame | `Frame.FCStd` (parametric, sheet `Params`) + `tools/export.py` | `frame.3mf` | 216 x 210, pitch 41.6 |
| Infra frame | `infra_frame.py` | `infra_frame.3mf` | 36 deep, switch on a ledge + plane core below |
| End frame | `end_frame.py` | `end_frame.3mf` | 9 mm flat frame, closes the block |
| Plane carrier | `plane_carrier.py` | `plane_carrier_half.3mf` (print twice) | screwed to the frame legs; boards on pin standoffs + 30 degree clips |

Run a generator with `freecadcmd <script>.py` from anywhere (paths are relative to the script); each prints `CHK` lines
(validity, clashes). `infra_frame.py` and `end_frame.py` read `Frame.FCStd`, so after changing
the Params sheet regenerate all of them, then `plane_carrier.py` to re-check clashes.

Key numbers:

- Frame 216 wide = board 180 + 18 on the 10-pin power side (`widen_l`) + 18 on the front panel
  header side (`widen_r`, added 2026-11-19). The old 198 mm frame is `Frame.w198.FCStd`.
- Pitch 41.6 = rail 12 + standoff 11 + board 1.6 + 17 on the solder side. `cooler_h` = 23 is the
  cooler height above the PCB that ends flush with the rail back; `standoff_h = cooler_h - 12`.
- Standoffs cannot go below 6 mm (board parts hit the band); checked with `tools/lowso.py`.
- Block length 36 + 8 x 41.6 + 9 = 377.8 mm against a tank half of 379 mm (not measured).
- A row is 246 mm (frame 216 + plane 30.2). Two rows use 492 of the tank's 664.5 mm depth (spec
  sheet, not measured). The strip behind them holds the 48 V supplies (Changbao CB3000,
  287 x 185 x 70, covers off): 8 places = 2 halves x 2 high x 2 deep, with 0.5 mm left over when
  two sit side by side. 64 nodes need 5 supplies, 48 or 56 need 4 (then one deep is enough).
  Decided 2026-11-21: six supplies in every pod (two per mains phase), so two may fail or a
  phase may drop: the other four run at 64% (48 nodes) or 85% (64 nodes) at 150 W per node.
  Six always needs the two-deep fit. Nodes idle at about 20 W (6 to 8% per supply).
- Decided 2026-11-21: the 48 V bars run along the tank's centre bar in a printed holder (to
  design; needs the bar's real section and height). Infra frames face the centre bar in both
  halves. A block cannot be mirrored, so the right half's blocks are turned half a turn and
  carry their planes on the other side of the row.
- The main 48 V bars are copper 15 x 15 x 350 mm (the user makes them, tapped for every lug).
  The block bars are Al 10 x 10 through the frames (Params bar_x 7.7, bar_z 130, bar_pitch 18).
  Open: copper bars lying along the centre bar or upright. Turned right half: block bars end
  at depth 44, 214, 296, 466 mm (span 423 > 350); layers are 212 mm apart in height.
- The block controllers have no RS485 port. The supplies' Modbus RTU goes through a Modbus TCP
  to RTU gateway on the management switch, outside the tank (to buy). The balancing software
  is not written; first pod: a script that sets 48.0 V and logs the currents.
- Plane: the core is in the infra frame (two ribbons of about 10 cm to unit 1), the controller on
  the carrier's top strip (link ribbon around the block corner, 30 to 35 cm), the four units
  (326 mm) along the 8 frames. Chain connector = 3V3, GND, I2C only; link = CAN FD.

## Board screws: self-tapping, no inserts (2026-11-27)

- The four standoff holes are pilot holes for 3 mm self-tapping screws (3 x 10 pan head,
  ST2.9 works too), no heat-set inserts: 2.5 mm x 10 deep (Params insert_d 2.5,
  insert_depth 10; set by `tools/screw_pilot.py`, the same pilot as the carrier screws).
  3 x 10 through the 1.6 mm board leaves 8.4 mm of thread in the standoff. Do not
  overtighten. Backup with the insert holes: `Frame.pre-screw-pilot.FCStd`.
- The infra frame's four core standoffs (infra_frame.py PILOT_D, PILOT_DEPTH) are the same
  2.5 x 10 pilot, same screws: 1 mm of plastic left under the hole.
- Flood openings: infra_frame.py and end_frame.py now also write `infra_frame_flood.3mf` and
  `end_frame_flood.3mf`, the frame plus the node frame's six FloodModifiers boxes (set walls 0
  on them in the slicer, as on frame_flood.3mf). Infra: same place (y 8..11). End frame (9 mm
  thick): moved to y 5.5..8.5.
- The drop-in claw mount (2149b23) was tried and reverted: it would not fit.

## Plane carrier fixing (changed 2026-11-21)

- The hooks into the frame windows are gone (they needed supports). Each half has four slots
  (3.4 x 5.2) at the ends of its struts for 3 x 12 self-tapping screws into pilot holes in the
  node frame legs: `CarrierPilotHoles` in Frame.FCStd, 2.5 mm x 10 deep in the x = 0 face at
  local y 32.7, z 5 and z 205 (Params car_pilot_*; added by `tools/carrier_pilot.py`).
  Backup without them: `Frame.pre-carrier-pilot.FCStd`. Old carrier:
  `tools/plane_carrier.hooks.superseded.py`.
- Boards: standoffs with a 2.9 mm pin (board holes assumed 3.2 mm) and clips whose barb faces
  are 30 degrees off the post, the user's rule for printing without supports. Prints frame
  side down. Clips are untested and bend across the layers.
- `tools/render_page.sh` re-renders all page pictures into `pod-layout/img`.

## Locking the block for lifting (2026-11-21)

- The dovetails slide along z and only stop one way: a frame can lift up out of the frame
  behind it (higher y). A block cannot be lifted by one frame.
- The user chose zip ties over tie rods (rejected) and over a plug in the bottom entry pocket.
  `TieSlots` and `TieRecess` in Frame.FCStd (added by `tools/tie_slots.py`): a 4.4 x 2.2 slot
  through each top leg along x at y 0.9..5.3, z 11.3..13.5, and a 1.6 deep recess across the
  groove walls and the tenon top. 3.6 mm ties, head on the inside of the leg, 18 per block.
- The end frame inherits both. `infra_frame.py` cuts the tenon recess itself (its tenons are
  at y 36..42). Backup before this: `Frame.pre-tie.FCStd`. Picture: `tools/tie_view.py`.
- Not printed. Open: the 1.3 mm floor between groove and slot, and a lift test.

## Coolers

- The bridge was designed for a cooler that no longer exists. The board has four cooler posts
  (3.8 mm high) on 75 x 75 mm, the LGA 115x pattern, so such a cooler may mount directly.
- A 20 mm cooler the user found fits as is, but stock is short.
- Dynatron K129 (90 x 90 x 26.5, copper): 2 pieces arrive Tue 2026-11-24. It ends 28.2 mm above
  the PCB and stands 5.2 mm proud of the rail back, into the free space behind the next board,
  where 3.5 mm stays free (`tools/bleed.py`). So it fits the frames as they are, 8 per block.
  The infra frame got a pocket in its ledge for the first node's cooler; the fins end 0.3 mm from
  the switch at its assumed size. `frame_k129.*` (a 47.6 mm deep variant) is obsolete.
- Other candidate: Lori LGA115X-1U-P11, 88 x 88 x 23, full copper, 3 to 15 days lead time.

## Open checks (also on the page under "Bench checks")

Mon 2026-11-23, controller and plane boards arrive (a carrier half is printed):
- units on the carrier: the chain connectors on the back (5.2 mm in the board model) pass
  0.8 mm over the rib between unit 1 and 2 and over the end strut of each half (standoffs
  6 mm). Check they clear; measure the mated pair above the back of the board. If they
  touch: cut a 2 mm relief in rib and strut (plane_carrier.py), do not raise the standoffs
  (that makes the row deeper)
- controller link plug (9.0 mm) has 1.0 mm over the ladder
- the 2.9 mm pins in the board holes; the clips snapping over the edge without breaking
- front panel cable: the unit connector is 4 pins (JST GH: 1 PWR_SW, 2 RST_SW, 3 PLED+,
  4 PLED-), by design one less than controller rev C (JST PH 5p with node GND on pin 3).
  Presses return through the 19 V cable, so they need it plugged in. With a meter on the
  AXB35 header before making cables: which pin of each button is ground (wire the other,
  leave ground open), and polarity and voltage of the power LED pins. The pinout is
  unconfirmed in the electronics repo (cabling/node.yml, site/src/reference/pinouts.md)
- to order for the front panel cables (urgent, asked 2026-11-21): JST GH series, 1.25 mm,
  4 positions, latching. Housing GHR-04V-S, contacts SSHL-002T-P0.2 (26 to 30 AWG); shops
  list it as "JST GH 1.25 mm 4-pin" ("GHR" is only the housing's part-number prefix).
  Pre-crimped single-ended leads of 30 cm or more avoid the crimp tool. Not PicoBlade, SH
  or PH. 8 per block: 96 for 12 blocks, 128 for 16, plus spares. Unit header: SM04B-GHS-TB.
  Motherboard end: plain 2.54 mm
- controller on four 10 mm M3 spacers; 2.5 mm between unit 2's 48 V plug and the ESP32 module
- link ribbon length around the corner; core in the infra frame, plugs reaching the carrier side
- first-batch units switch on at about 41.3 V, off at 38.3 V (fixed to 38.0 / 35.0 for the next
  rev in the electronics repo); watch for a false power-good trip at slot start

Tue 2026-11-24, K129:
- its contact pad must stand more than 2.1 mm proud of the base (posts 3.8, chip 1.7), else a
  copper shim; pad size about 35 x 35 scaled off the drawing
- whether its screws suit the board's posts

Whenever possible:
- mains draw of one CB3000 with no load (decides whether idle staging is ever worth it), and
  whether it keeps its setpoint and current limit through a power cycle
- real switch size (assumed 160 x 90 x 25, Sodola SL902-SWTGW218AS)
- tank inside depth (664.5) and half length beside the centre bar (379)
- which board-edge header is the front panel one (`JFP1` in the STEP; four header-like parts
  were cleared without knowing which it is)
- the 2.8 mm wall in front of the end frame's dovetail grooves
- barrel plugs for the 19 V node leads (9 to 13 A), still to order

## Tried and dropped (do not redo)

- Lowering the switch shelf or rotating the core: the core's plug lands on other parts.
- Units on top of the carrier with the controller below: needs a 40 cm I2C ribbon. Rejected by
  the user. Kept as `tools/plane_carrier.units-top.rejected.py`.
- A notch in the rail for the front panel plug: the plug is as long as the rail is deep.
- A ninth node per block: needs a pitch of 37.1, and the plane is built for 8.

## Layout page: how to update

- Edit `mech/pod-layout/pod-layout.html`, run `node smoke.js pod-layout.html` (5 presets,
  expect "bad tokens 0" on each), publish to the artifact URL above with `root` = that folder
  and the `img/*.png` files in `files`.
- Pictures (sheets 8 to 11): `tools/tankview2.py` and `tools/rot_step.py` build STEP files
  rotated to tank-up, rendered with `/usr/bin/solidcraft-cli snapshot`. The scripts write into
  the current directory. Camera yaws that work: 3.85 (carrier side), 4.7124 pitch 0 (square on
  the carrier), 4.34 (infra front), -1.2 (infra back), 3.75 pitch 0.45 (end frame).
- Stale on the page: Sheet 6 (block wiring) and its cable schedule still draw the controller in
  the infra frame; Sheet 3 draws the node frame with the model's "up". The wiring sheets and
  `docs/block-cabling.md` in the electronics repo have the same stale controller location.
- The page has never been checked in a browser by the assistant, only by the smoke test.

## Working rules the user set

- Plain ASCII in files. No Claude signatures in commits. Push only on explicit request.
- Under time pressure: act, keep reports short, and say plainly what was not verified.
