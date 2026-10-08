# Handoff - granite controller (2026-10-07)

Read this first, then docs/controller.md and docs/power-board.md.

## Repo state

- Branch `eight-node-expander` holds all current work (rev C). Last
  commit before this file: 4927f3e "Tidy rev C schematic sheets".
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
  outline 230 x 40 mm (180 was too dense) with 4 corner M3 NPTH
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
2. PCB is fully routed (first pass, see board state). User reviews in
   KiCad: Ethernet crossovers, power section loops, SPI transposition,
   and the F.Cu tracks KRT left under the ESP32 module body (Espressif
   advises keeping the module underside clear; B.Cu under it is fine).
   Then: JLC impedance check, fab outputs.
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
  (default 180 x 40 mm, board uses 230 x 40, 1 mm corner radius) plus H1-H4 M3 NPTH holes
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
- `tools/gndpour.py <in.pcb> <out.pcb>`: GND pours on F.Cu/B.Cu and
  0.6/0.3 stitching vias (group "gnd-stitch", re-runnable). Run after
  routing.
- `tools/krt_reroute.sh NET...`: targeted pass after moving a few parts.
  Deletes all copper of the named nets, reroutes them on F.Cu/B.Cu, runs a
  GND pass without rip, refills zones via kicad-cli and copies the board
  back. Name every non-GND net on the moved parts plus the open nets.
  Does not clean GND stubs left at old pad positions.
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
