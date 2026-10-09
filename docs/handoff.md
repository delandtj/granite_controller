# Handoff - granite controller (updated 2026-10-08 night)

Read this first, then docs/controller.md and docs/power-board.md.

## Next session: order prep

State 2026-10-09 (evening), branch eight-node-expander:
- J2 is now HRO TYPE-C-31-M-12 (C165948; the GCT USB4105-GF-A was down to
  140 at JLC). Commit c0d9b0e. Rear fan-out re-laid (D+ join behind the
  pads, CC straight back to vias, VBUS 0.5 mm on B.Cu).
- Board shortened 250 -> 199 mm (tools/compact.py): power, Ethernet and
  MCU blocks unchanged with their routing; everything right of x 134.8
  re-placed and rerouted by KRT. Top edge J6 | J7 | J9 | J11-J14, bottom
  J8 | J10 | J15-J18. U14 over U15 next to the relays; U16/U17 in the band
  between the relay halves (inputs towards their connectors); relay
  columns at 13.6 mm pitch.
- Checks: ERC 0; DRC 0 unconnected, 0 parity, violations: 2 U1 antenna
  silk (intended), J2 differs from library (silk dropped, GND pads solid),
  single-spoke thermals on THT GND pins J8.4 (as before) and J5.3 (both
  also on the In1 plane). Vias in pads: only the 4 U1 pad 29 thermal vias.
  Inner planes carry no tracks (KRT's redundant In1/In2 tracks removed).
- Silk: pin legends on all connectors except J9/J10 (no room between
  U14/U15 and the connectors; names are there).
- The previous 250 mm board with the GCT USB is tag rev-c-usb4105.
- fab/ regenerated from this board 2026-10-09.

Before ordering: look at the board in KiCad (the right region is all
KRT), then the JLC preview/impedance steps below.

Left for the user / the order:
- Review the board in KiCad: power section (shifted right for the
  short-edge rails), the U1 area (EN RC, decoupling, VIN_SENSE moved
  to the module, no F.Cu under the module body), silk at 1.0 mm.
- Order with JLC rails + mouse bites on the two SHORT edges only (U1
  and J2 overhang the long edges). Check rotations in JLC's placement
  preview, especially J2 and the THT connectors.
- Confirm ETH 0.16/0.15 and USB 0.25/0.15 in JLC's impedance
  calculator (JLC04161H-7628).
- Uploading Gerbers to JLC is the user's call: fab/ via
  tools/jlcfab.py.
- J8: stays DNP (user, 2026-10-09).

## Repo state

- Branch `eight-node-expander` holds all current work (rev C), pushed to
  the fork (2026-10-08 night, including the review fixes and this
  handoff). Push only when the user asks.
- `master` = rev A (colleague's STM32 design) + PCB setup, BOM fix,
  first expansion header. Both branches are pushed to the user's fork.
- Remotes: `origin` = github.com/delandtj/granite_controller (fork),
  `upstream` = github.com/LeeSmet/granite_controller (colleague, Lee
  Smet). No PR to upstream yet: the user wants to discuss more first.
- Untracked, keep out of git: `W5500_ds_v110e-*.pdf` (datasheet the
  user dropped in the root), KiCad lock files.

## What the board is now (rev C)

19 V-powered controller (PoE until 2026-10-07) for one frame of 8 Strix
Halo motherboards, fully submerged in dielectric fluid (single-phase now, maybe
two-phase later; materials chosen for two-phase). Details and pin
map: docs/controller.md.

- ESP32-C6-WROOM-1-N8 (8 MB, OTA headroom; N16 drop-in) + W5500 SPI Ethernet (replaced STM32H563 +
  LAN8742A). PoE dropped: J3 19 V input -> PTC/TVS/Schottky ->
  LMR51430 buck -> 5 V, HR911105A plain magjack; TLV62569 3V3 unchanged.
  All parts carry LCSC numbers for JLC assembly.
- 8 relay channels: 16 TLP176AM photoMOS (RST + PWR per node), driven
  by MCP23017 U14 @0x20 on the internal I2C bus, held in reset by
  default (all buttons released if the MCU is not driving it).
- Node power sensing: 5-wire node cable (PWR_SW, RST_SW, GND, PLED+,
  PLED-), 2x TLP290-4 AC-input optos -> U15 MCP23017 @0x21 (internal
  bus) GPA0-7. U15 GPB0-3 read 4 dry-contact inputs (J10). J9 I2C
  sensor port (Qwiic order). VIN_SENSE divider on GPIO5.
- Layout target (user): long strip ~30 mm high along the top of the
  frame: power 30x30, MCU+ETH+USB 30x40 (RJ45 on the top edge, antenna
  toward the bottom edge), probes + J8, then the 8 node connectors.
- External I2C + EXP_nRESET + EXP_INT on a protected 2x6 header J8 for
  add-on MCP23017 modules (0x21-0x27) and the future power board
  (LTC4282 x8 @0x40-0x47).
- Sheets: Power, Ethernet, MCU, Relay channels, Expansion header.
  Generated, then tidied by hand-style layout; netlist verified
  identical across the tidy. ERC 0. PCB regenerated from scratch after
  the 19 V change (162 footprints, 0 parity issues); nothing placed,
  outline 250 x 50 mm (180 x 40 was too dense) with 4 corner M3 NPTH
  holes; first grouped placement done (tools/place.py), unrouted.

## PCB setup already done

4 layers, JLCPCB JLC04161H-7628 1.6 mm stackup (0.2104 mm 7628
prepreg, 1 oz outer, 0.5 oz inner), net classes with widths, diff
pairs from a 2D field solve: ETH 0.16/0.15 = 101R in fluid (Er 2.1),
USB 0.25/0.15 = 89R in air (service port, used out of the fluid).
Confirm with JLC's impedance calculator before ordering.
granite_controller.kicad_dru
(USB-C NPTH exception; the PoE clearance rule is gone with PoE).

## Decisions made with the user

- Fab: JLCPCB for bare board and assembly (changed from PCBWay on
  2026-10-07; the colleague preferred PCBWay). 4 layers. Claude does board setup, outline,
  holes and grouped placement; the user routes critical nets (Ethernet
  pairs, USB, switcher loop, crystals).
- 8 nodes per frame. MCP23017 relay drive, fail-safe via reset.
- MCU: ESP32-C6 (user has S3/C5 experience and likes ESP-IDF).
- Power board (docs/power-board.md): separate 8-channel 19 V board fed
  by two 3 kW supplies; LTC4282 per channel with I2C monitoring;
  default-ON; auto-retry on faults (firmware disables a channel after
  repeated retries); LTC4282 EEPROM provisioned by controller firmware
  over I2C on boot, never storing FET_ON = 0.

## Open items

Controller (docs/controller.md "Open items"):
1. Soak tests (HR911105A magjack, JST housings, J8 header housing).
2. 19 V tap location and fuse on the power board side.
3. ESP32-C6 LP I2C usable from the HP core? (else software I2C stays).
4. ESP32-C6-MINI-1-N4 stock/price.
5. Layout: board outline, connector edges (8 node connectors, 4 probe
   connectors, J8, RJ45, USB-C), mounting holes, assembly side,
   surface finish (ENIG vs HASL). The user said "a lengthy board".
6. New passive MPNs follow YAGEO/Samsung naming but are unchecked
   against distributors.

Power board: PSU model (paralleling, PMBus), node 19 V input connector
(barrel jack too weak for 10.5 A), main fuse location, outline.

## Next likely steps

1. Architecture is being iterated (see "Pod architecture" below): this
   board stays as the integrated controller for small (one-frame) sites.
2. PCB: review fix pass done (see the top of this file); next is the
   independent check, then order prep.
3. Open hardware questions: motherboard front-panel pinout (PLED drive,
   ground switch returns) - user waits for the board doc/STEP; J8 as the
   power-board link (fit by default, keyed connector?); PSU remote control
   (ON/OFF via photoMOS, current/voltage setpoint via I2C DAC with a
   hardware-clamped range) - belongs on the power board.

## Pod architecture (2026-10-08, iterating)

A pod holds 4 nodes up to 9 frames x 8 plus storage, PSUs and more. Idea:
one controller per pod outside the fluid (network, Wi-Fi, bus master) and
a family of "ducks" near the equipment (frame duck, storage duck, PSU
duck, more to come) on RS-485 / Modbus RTU (I2C is board-local only).
Proposed: a common duck core as a KiCad design block (MCU, RS-485, 19 V
front end, unique ID), self-describing ducks (identity + capability
registers), generic channel kinds (contact, binary in, temperature,
analog in/out, switched out). Open: controller location and bus length,
max ducks per pod, polling vs events (CAN), ground bonding across frames,
addressing without DIP switches. Docs to write when settled:
architecture.md, duck-core.md, duck-protocol.md.

## Tools and gotchas

- `tools/mkboard.py <netlist.xml> <project_dir> <out.kicad_pcb>`:
  builds a fresh board from a KiCad XML netlist (footprints linked by
  symbol UUID, nets assigned, parked in a grid). Only for an unplaced
  board: it discards placement. Write `out` to a scratch directory and
  copy only the .kicad_pcb back: pcbnew's Save rewrites the
  .kicad_pro next to the board and strips schematic settings and net
  class patterns.
- `tools/setrules.py <pcb> <pro>`: applies the stackup, board rules
  and net class values. Run after mkboard.
- `tools/outline.py <in.pcb> <out.pcb> [L] [H]`: Edge.Cuts rectangle
  (default 180 x 40 mm, board uses 250 x 50, 1 mm corner radius) plus H1-H4 M3 NPTH holes
  4 mm in from each corner, board-only. Idempotent; re-run after
  mkboard (which parks parts at x < 0, left of the outline). Same
  scratch-dir rule as mkboard. Order: mkboard -> outline -> place -> silk -> setrules -> krt_route -> gndpour.
- `tools/place.py <in.pcb> <out.pcb>`: first grouped placement for the
  230 x 40 strip (zones along x: power 0-28, Ethernet 28-60, MCU/USB
  60-100, probes + J8/J9/J10 100-130, U14/U15/U16/U17 130-160, node
  channels 160-220 in 4 columns, ch1-4 top row, ch5-8 bottom row). RJ45
  and USB-C openings on the top edge, WROOM antenna overhanging the
  bottom edge. Anchors big parts, packs the rest per sheet. A start for
  hand placement only: re-running it discards hand moves.
- `tools/silk.py <in.pcb> <out.pcb>`: values to F.Fab, silk refs hidden
  on small passives (F.Fab keeps them), other refs 0.8 mm placed clear
  of pads, silk outlines, other courtyards and each other (connectors
  first). Run after place. Only expected silk DRC hits: U1's outline
  where the antenna overhangs the bottom edge.
- `tools/krt_route.sh`: routes with KiCadRoutingTools (drandyhaas; checkout
  and venv in ~/.cache/graver-pcb/routetest, rebuild the Rust core with
  CARGO_TARGET_DIR unset). Strips old copper, then planes (GND In1, +3V3
  In2), Ethernet and USB pairs, then everything on F.Cu/B.Cu only. Runs on
  a copy in ~/.cache/granite-pcb/route because KRT rewrites the .kicad_pro
  next to the board; only the .kicad_pcb comes back. Keeps hand placement.
  Grade with kicad-cli DRC, never with KRT's own checks.
- `tools/gndpour.py <in.pcb> <out.pcb>`: GND pours on F.Cu/B.Cu (1 mm short
  of the short edges for the rails) and
  0.6/0.3 stitching vias (group "gnd-stitch", re-runnable). Run after
  routing.
- `tools/compact.py <in> <out>`: the 199 mm re-placement of the region
  right of the MCU block (deletes its copper, places, moves the right edge,
  H2/H3, pours and keep-outs, lays the GND straps and escape vias KRT
  cannot find). Then silk.py, connlabels.py, refill, `KRT_EXTRA="--ordering
  mps" krt_reroute.sh --keep "*"`, then targeted passes on what is open.
- `tools/viaout.py <in> <out>`: moves vias out of SMD pads (KRT's plane
  welds) where a clear spot exists. KRT runs now pass
  --same-net-pad-clearance 0.15, which avoids most of them; check with a
  via-in-pad count after every KRT run.
- Prune (`tools/prune.py`) only when DRC shows 0 unconnected: on an open
  route both halves count as dangling and prune eats them.
- After KRT: delete any tracks on In1/In2 (redundant, they cut the 3V3
  plane) and any vias KRT adds inside U1 pad 29 beyond the 4 thermal ones.
- `tools/krt_reroute.sh [--keep] NET...`: targeted pass after moving a few parts.
  Deletes all copper of the named nets, reroutes them on F.Cu/B.Cu, runs a
  GND pass without rip, refills zones via kicad-cli and copies the board
  back. Name every non-GND net on the moved parts plus the open nets.
  Does not clean GND stubs left at old pad positions.
- `tools/jlcfab.py [pcb] [out]` (KiCad python): JLC outputs into fab/
  (gitignored): Gerbers + Excellon zip, BOM (Comment/Designator/Footprint/
  LCSC Part #, grouped by LCSC), CPL (absolute mm, Y flipped like the
  Gerbers, THT parts at pad centre), and a list of rotation-corrected parts
  (subset of matthewlai/JLCKicadTools' table) to check in JLC's preview.
  Skips DNP (J8) and parts without LCSC (H1-H4, TP1-TP4).
- `tools/connlabels.py <in.pcb> <out.pcb>`: connector function labels on
  F.Silkscreen ("J11 NODE 1", "J3 19V IN", ...) plus a pin legend under
  the wire-to-board connectors (node: PW RS CM L+ L-; 19 V: + -; 1-wire:
  3V DQ G; I2C: G 3V DA CL; dry in: 1 2 3 4 G). Hides those silk refs
  (F.Fab keeps them), places around existing silk and pads, group
  "conn-labels". Run after silk.py, which would show the refs again.
- `tools/pcbtool.py`: helpers for scripted hand routing (add tracks and
  vias at exact coordinates, delete a net's copper in a region) and
  render(): a region PNG with F.Cu red, B.Cu blue, highlighted nets and
  DRC opens/violations drawn in. `tools/prune.py <pcb> <drc.rpt>` deletes
  what DRC flags as dangling; repeat DRC + prune until clean.
- Board state 2026-10-08 (afternoon, superseded): placement by place.py + the user's
  hand moves (whole board shifted +25/+28.6 mm on the sheet, MCU-corner
  parts moved), routed by krt_route.sh, poured by gndpour.py, targeted KRT
  passes on the open nets. J8 area spread: RN1 (132, 68.5) and RN2
  (132, 73) rot 180 between U1 and J8, U7 (146.3, 69) beside J8 pins 9-11,
  U18 (151.3, 68.5) above J10. U7/U18 centre GND pads 3+8 strapped on F.Cu
  to a 0.6/0.3 GND via 1.2 mm right of pad 8 (KRT cannot reach them). 22
  nets rerouted (all J8/RN/DRYC/EXP nets incl. EXP_nRESET_INT, which ran
  through the RN1 spot, plus the W5500 SPI and ETH_LED_ACT).
- Board state 2026-10-08 (evening): FULLY ROUTED, first pass. DRC
  (refill zones first: KRT does not refill, a stale fill shows as
  hundreds of fake clearance errors): 0 unconnected, 0 parity, 4
  violations: 2 silk (antenna overhang, intended), 2 starved thermals on
  J8 GND pins 3/4 (THT, also tied to the In1 plane). ERC 0. Hand work
  (scripted through tools/pcbtool.py, geometry in the commit messages):
  - Ethernet MDI: J1 -> R20/R21, C28/C29, R22/R23 -> 33R row straight;
    TX/RX crossovers between the 33R row and U3 on 0.5/0.2 vias (N line
    on B.Cu). W5500 AVDD pins 4-8 and 11-15 joined under the LQFP body,
    GND pins 9/19 to inner plane vias, EXRES straight to R15.
  - Crystal: XO on F.Cu around Y2's right, XI under Y2 on B.Cu.
  - SPI/INT/RST: the W5500 edge order (CS SCLK MISO MOSI INT RST) is the
    reverse of the ESP32 west column (CS SCLK MOSI MISO INT RST), so each
    line runs east on F.Cu at its own 0.8 mm level and transposes on its
    own B.Cu column at x 96-100; RST stays on F.Cu. Pull-ups R16/R17/R18
    moved into the bundle (x 80-86) with +3V3 plane vias.
  - GPIO_IO15 to RN2 under the module on B.Cu (clear of the U1 thermal
    pad), GPIO_IO8 over to a via beside RN1; C6_EN, EXP_nRESET,
    I2C_EXT_SCL/SDA, HDR_IO15 rerouted by KRT around them.
  - USB: R34/R35 moved next to U1 pins 13/14, U4 -> R34/R35 routed as a
    0.25/0.15 pair (KRT route_diff); connector side unchanged (D+/D- pad
    joins under the receptacle, normal for USB-C).
  - Power section re-placed for tight loops (the old one had U8's input
    caps ~17 mm and its bootstrap cap ~20 mm away): U8 at (47.5, 62) with
    L3 0.7 mm off SW, C42/C41/C40 on a VIN bar under pin 3, GND via under
    the body, EN tied to VIN under the body, C43 above, R37/R38 at FB;
    C44/C45 and D2 at L3's output. U2 at (32, 59) with L1 at SW, C3 under
    VIN, C4 at the output, EN tied to VIN under the body. Input chain
    J3 -> F3 -> D8 with D1 below. C51 moved left of its old spot.
  - The rest of the board is KRT. KRT put many F.Cu tracks under the ESP32
    module body (see next steps).
  - Later the same day: connector function labels + pin legends on the
    silk (tools/connlabels.py); R15 swapped to C11692 (stock); 3D models
    for K1-K16, U16/U17, J1, U1 in 3dmodels/ (see its README); JLC
    outputs via tools/jlcfab.py. All LCSC parts were in stock for 8
    boards on 2026-10-08 (jlcsearch); 27 Basic / 33 Extended.
- Board state 2026-10-08 (night): review fix pass (docs/review-2026-10.md
  "Status"). New parts R65/C52 (VBUS bleeder, left of J2), D9 (BUCK_5V
  crowbar, below D2). Rule areas: 6.5 mm F/B.Cu keep-outs at H1-H4 and
  "U1 body: no F.Cu tracks"; DRU rules for hole clearance 0.3 mm and
  non-GND vias 0.5 mm from U1 pad 29. Zones end 1 mm from the short
  edges. Autorouter vias moved out of SMD pads (only the U1 pad 29
  thermal vias stay in a pad). tools/silk.py and tools/connlabels.py
  now write 1.0/0.15 mm text and widen footprint silk to 0.15 mm.
- Check after any change: `kicad-cli sch erc --severity-all`,
  `kicad-cli pcb drc --schematic-parity`. Expect 0 parity issues.
- kicad-cli occasionally re-serializes granite_controller.kicad_pro;
  `git diff` it and restore with `git checkout` if it changed.
- Do not use the KiCad MCP schematic tools on these files: they
  reformat whole sheets. Edit s-expressions as text.
- If KiCad has the project open, it can overwrite files edited on
  disk; ask the user to close or reload.
- Commits are GPG-signed; the pinentry times out when the user is
  away. Retry when they are back; never bypass signing.
- The original schematic generator scripts lived in a session
  scratchpad and are gone; the sheets are hand-editable now.
