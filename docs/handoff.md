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

1. User review of rev C with the colleague.
2. PCB: outline + mounting holes + grouped placement (ESP32 antenna
   keep-out at a board edge), then user routes critical nets.
3. Power board schematic once its open items are answered.

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
  scratch-dir rule as mkboard. Order: mkboard -> outline -> place -> setrules.
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
