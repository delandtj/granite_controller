# Chained plane: one core board and identical 2-slot unit boards

**Status**: Accepted (2026-10-29, implementation started: power/pp-48-core,
power/pp-48-unit)
**Date**: 2026-10-29

**Updated**: 2026-10-29 (user: chain on JST PH 2.0, 3V3 from the core,
unit hot-swap latch-off, input strip on the far side)

Related: docs/adr/0004-plane-column-layout.md (decisions 1, 1a, 2, 6
and 9 stand; 3, 4, 5, 7 and 8 are replaced here, see "What changes"),
docs/adr/0003-plane-tile-and-module-interface.md (decision 8: the tile
level moves from the plane core to every unit), docs/adr/0001-48v-power-plane.md,
docs/pm-interface.md (unchanged). The single-board PP-48 at tag
pp48-rev1 (N = 2, 106 x 72 mm) is the starting point.

---

## Context

ADR 0004 builds one board per variant: core + N/2 units + an end strip
(N = 2/4) or an end cap with a second 30 A input (N = 6/8). Four board
sizes, each routed, ordered and stocked on its own; an 8-slot board is
~275 mm long and held at the ends only, and the input current of a
long board needs a second tile and an In2 VBUS split.

The user wants one unit design that is chained: build 2, 4, 6 or 8
slots by plugging unit boards together, each unit powered on its own
(decided 2026-10-29: short signal cable between boards, per-unit
XT60PW input, input stage in a side strip, core board with its own
small 48 V input).

What the rev1 work already gives: the unit (two 25 mm slot columns, one
I2C segment, TCA6424A, In2 bus for slot A's lines) is drawn and routed
as a block, and the rotating channel (ADR 0004 decision 5) makes every
unit's copper identical while each unit still gets its own mux
channel. Both carry over; the channel just crosses a cable instead of
a column edge.

---

## Decision

1. **Two board designs.** A core board (MCU, mux, link, isolated 12 V,
   3V3 buck) and a unit board (two slots + input stage). A plane is
   one core and 1-4 units in a row: N = 2, 4, 6, 8. No end strip, no
   end cap, no per-N board.

2. **Unit board**, ~66 x 72 mm:
   - The rev1 unit unchanged in content: two 25 mm slot columns
     (front end under each module, ADR 0004 decisions 1, 1a, 2), one
     I2C segment, TCA6424A 0x22 in slot B, In2 bus for slot A's lines,
     addresses INA238 0x40 / 0x41, ID ADC 0x51 / 0x52 (ADR 0004
     decision 9).
   - An input strip ~16 mm wide on the far side (right of slot B),
     full height: XT60PW-M at the top edge (cable entry from the
     top, like the node cables leave at the bottom), bus TVS
     (SMDJ60A, as the core today), input caps (one tall electrolytic
     + ceramics), the unit hot-swap (decision 3) and two M3 holes
     (top and bottom of the strip).
   - VBUS: hot-swap output -> In2 VBUS plane of this board only ->
     the two slots' shunts, as rev1 (ADR 0004 decision 3, now per
     board, ~50 mm wide).
   - Chain connectors at the left edge (from the core side) and in
     the strip at the right edge (to the next unit), both at the
     channel height near the bottom (decision 4).

3. **Unit hot-swap** (the ADR 0003 tile level, now per 2 slots):
   LM5069-1 latch-off, the core's tile circuit scaled to one unit:
   2 slots x ~265 W = 530 W, 14 A at 38 V. Shunt ~3 mOhm (55 mV
   limit ~18 A, e.g. 2 x 6 mOhm 2512), one or two BSC040N10NS5 (SOA
   to check at the unit power limit), UVLO / OVLO / timer as the
   slots, INA238 on the shunt at 0x44 on the unit segment. Control
   over the TCA6424A's two spare lines: P13 U_OFF (out, Q pulls UVLO
   low, 100k pull-down = ON at reset) and P27 U_PGD (in). The
   INA238 ALERT is not wired (polled; DIAG_ALRT).

4. **Chain link**: a short cable between neighbouring boards, the same
   latching header at both edges of a unit and at the core's right
   edge. 14 pins (user 2026-10-29, docs/press-deadline-line.md; was 12):
   P0..P3 SDA/SCL (8), 3V3, GND x 3, PRESS_EN, EXP_RSTN. JST PH 2.0 mm
   (user 2026-10-29: sturdier than GH in the fluid), side entry
   S14B-PH-K-S, ~30 mm along the edge. The rotating channel goes
   through the board unchanged: the left connector's P0 is this unit's
   segment, its P1..P3 are wired to the right connector's P0..P2, the
   right connector's P3 is not connected. At the core connector Pk =
   mux channel k (4 used, as rev1). PRESS_EN and EXP_RSTN are bused
   (straight through every unit). Cable: flat and straight, ~50-100 mm,
   made by the user (PHR-14 housing + SPH-002T-P0.5 crimps). No VBUS in
   the chain.
   Pinout (power/pp-48/tools/sch/blocks/chain.py): the facing headers
   are turned 180 deg to each other, so a flat cable joins pin n to pin
   15 - n. IN headers (unit left edge): 1 3V3, 2 / 3 P0 SDA / SCL, 4
   GND, 5 / 6 P1, 7 GND, 8 / 9 P2, 10 GND, 11 / 12 P3, 13 PRESS_EN, 14
   EXP_RSTN. OUT headers (core, unit right edge) are the same read
   backwards (pin n = IN pin 15 - n); on B the IN pin 1 and the OUT pin
   14 are at the bottom, by the In2 channel; pins at v 35..61 on every
   board. LCSC: S14B-PH-K-S C157938 (THT, 381 in stock 2026-10-29; the
   SMD S14B-PH-SM4-TB C5305708 had 2944).
   Press deadline (docs/press-deadline-line.md, option B, user
   2026-10-29): every unit's four photoMOS LED cathodes return through
   one 2N7002 (Q10) whose gate is PRESS_EN (100k pull-down R31): no
   PRESS_EN, no press, whatever the expander drives. The core drives
   PRESS_EN from a 74LVC1G123 retriggerable monostable (U301, R301 100k
   x C301 10u ~1 s) clocked by PB12 (PRESS_TRIG, a GPIO the
   firmware toggles, 100k pull-down): a hung, stuck or reset MCU ends every press within ~1 s.
   EXP_RSTN (option A, recovery): PD0 open drain to every unit's
   TCA6424A /RESET (R30 pull-ups).

5. **3V3 for the units' logic comes from the core** (user 2026-10-29) through the chain
   (expanders, INA238s, ID ADCs, latches, photoMOS LEDs: < 50 mA per
   unit, ~0.2 A for 4 units, 3 GND pins carry the return). A unit with
   no 3V3 (core unplugged or dead) runs on hardware defaults: slots ON,
   unit hot-swap ON, latches cleared at power-up (ADR 0001 rules).

6. **Core board**: the rev1 core without the tile input (XT90, LM5069,
   FETs, shunts, INA238 U8, D1, C1, C2 move out; TILE_* signals go).
   Own 48 V input: XT30PW-M, bus TVS, input caps, no fuse (ADR 0003
   decision 8: no fuses in the fluid; the 12 V module and the 3V3
   buck limit on their own). Mux U6 TCA9548A channels 0-3 to the chain
   connector at the right edge, 4-7 open. Size set by its parts
   (~40 x 72 mm), mounting holes in its corners.

7. **Mounting**: every board on its own holes (unit: two in the input
   strip plus two at the left edge, positions to fit around slot A's
   standoffs; core: four corners). Boards ~2-5 mm apart in a row; the
   chain cable takes the gap and any misalignment.

### What changes in ADR 0004

| 0004 decision | Now |
|---|---|
| 3 Stackup and VBUS | per unit board: In2 VBUS from the unit hot-swap |
| 4 Family | 1 core + 1-4 identical unit boards, no per-N board |
| 5 Rotating channel | same scheme, through the chain cable; 4 positions |
| 7 Input current scales with N | one 14 A input per unit; no 30 A tile, no end cap, no VBUS split |
| 8 Plane mounting | holes per board |
| 1, 1a, 2, 6, 9 | unchanged |

---

## Architecture Overview

### Component Breakdown

1. **Core board** (power/pp-48 core sheets: tools/sch/blocks/mcu.py,
   mux.py, link.py, supplies.py; the input block shrinks to the small
   core input)
   - MCU, mux (I2C2 upstream), RS-485 link with the isolators, 12 V
     isolated module, 3V3 buck.
   - Interfaces: XT30 48 V in, link connectors, 12 V out, chain
     connector (P0..P3, 3V3, GND).
2. **Unit board** (tools/sch/blocks/unit.py + a unit input block)
   - Two slots, TCA6424A, In2 bus, input strip with hot-swap and INA238.
   - Interfaces: XT60 48 V in, chain in (left), chain out (right),
     per slot the node terminal and the front-panel GH.

### Data Flow / Interaction

```
 48 V bus (breaker outside the fluid, ADR 0003 decision 8)
   |XT30          |XT60               |XT60               |XT60
 +------+  PH12 +-------------+  PH12 +-------------+  PH12 +---- ...
 | core |------>| unit 1      |------>| unit 2      |------>| unit 3
 | MCU  | P0..3 | P0 = own    | P1..3 | P0 = own    | P2..3 |
 | mux  | 3V3   | P1..3 -> out| -> P0..2 ...        |       |
 +------+       | HS 14 A     |       | HS 14 A     |       |
                | slot A  B   |       | slot A  B   |       |
                +-------------+       +-------------+       +----
                  nodes 1, 2            nodes 3, 4
```

---

## Alternatives Considered

### One board per variant (ADR 0004 as built at pp48-rev1)
- **The idea**: core + N/2 units + end strip / end cap on one PCB.
- **Optimizes for**: no inter-board connectors in the fluid, one
  mechanical part per plane.
- **Sharpest tradeoff**: four board designs, a 30 A tile and an end cap
  for N = 6/8, a long board held at the ends.
- **Bets on**: the plane sizes staying few and fixed.

### Chained with VBUS through the chain
- **The idea**: one power input on the core, VBUS passed board to board.
- **Optimizes for**: one power cable per plane.
- **Sharpest tradeoff**: ~42 A through the first joint at N = 8; needs
  bus bars or heavy connectors per joint.
- **Bets on**: N staying at 2-4.

### Direct board-to-board mating
- **The idea**: right-angle header mates a right-angle socket in-plane,
  boards butt together.
- **Optimizes for**: no loose cables.
- **Sharpest tradeoff**: alignment and flex load the contacts at every
  joint, and the boards must be mounted as one rigid row.
- **Bets on**: a precise, stiff carrier frame in the box.

### Input stage in a top strip
- **The idea**: ~14 mm strip along the top, board ~50 x 86 mm.
- **Optimizes for**: a narrower plane row, power in from the top.
- **Sharpest tradeoff**: plane height grows 72 -> ~86 mm.
- **Bets on**: box height being freer than box length (the user chose
  the side strip).

---

## Consequences

### Positive
- One unit board for every plane size; spares are interchangeable.
- Fault unit on the bus side drops from 4 slots to 2; no 30 A tile, no
  end cap, no VBUS split.
- Each board is small and held on its own holes (no 275 mm flex).
- The core board no longer carries 30 A copper.

### Negative
- Connectors in the fluid at every joint (signal only, latching).
- One hot-swap and one XT60 cable per 2 slots instead of per 4.
- The plane row gets longer: units ~66 mm instead of 50 mm, plus gaps;
  N = 8 is ~40 + 4 x 66 + gaps, ~315 mm.
- The units' logic depends on the core's 3V3 (hardware defaults hold
  when it is missing).
- The rev1 core and routed board are partly thrown away (the tile
  input moves into the unit).

### Risks
- **I2C over cables**: the worst segment (unit 4) crosses 4 cables and 3
  boards, ~0.5 m. At 100 kHz with 4.7k pull-ups per segment this is
  well inside 400 pF; keep the GND pins between the pairs.
- **PH contacts in the fluid over years**: tin-plated 2.0 mm contacts
  with a friction lock, no positive latch. Mitigation: soak test, a
  dab of retention (strain relief tie to the board hole); fallback a
  latching XH / Micro-Fit with the same pinout.
- **3V3 drop over 4 cables**: ~0.2 A x 4 joints x ~40 mOhm per contact
  pair is well under 50 mV.

---

## What an Expert Would Ask

**Q: A unit is plugged in with the core already running. Does anything break?**
A: The chain carries only 3V3, GND and I2C. Hot-plugging the chain
cable can glitch the segment of that unit and the ones behind it; the
mux isolates the other channels, and the firmware re-scans a channel
that NAKs. The slots and the unit hot-swap stay on their hardware
defaults meanwhile. Plugging the XT60 under power is what the unit
hot-swap is for.

**Q: What stops a unit from powering the core's 3V3 or the other way round?**
A: The unit has no 3V3 source of its own; its 3V3 net is the chain's.
Nothing back-feeds. The INA238 inputs tolerate 48 V on IN+/IN-/VBUS
with VS at 0 V, so an unpowered segment next to a live VBUS is safe.

**Q: Why not make each unit self-contained with its own 3V3 buck?**
A: It would remove the 3V3 pins from the chain and keep a unit's
telemetry logic alive without the core, but the MCU is on the core, so
nothing reads it anyway. One buck per unit adds cost and area in the
strip for no function. Revisit if units ever get their own MCU.

**Q: Four units is the hard limit. Is that enough?**
A: The requirement is 8 nodes per frame (memory: requirements-8-nodes);
4 units x 2 slots = 8. A ninth slot needs a second plane (its own
core), or mux channels 4-7 through a wider chain (16 lines). Not
handled now.

**Q: Unit hot-swap at 18 A with two slot hot-swaps at 15.7 A each behind it: does the coordination work?**
A: Each slot limits at 15.7 A and breaks at 30 A on its own; a single
slot fault stays below the unit limit and trips only that slot. The
unit limit only catches a fault on the unit's own bus (caps, TVS,
traces) or both slots high at once. The power limit / timer of the
unit LM5069 must ride through both slots' insertion (160 ms each):
to size with the SOA check of decision 3.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **Chain connector**: JST PH 2.0 14-pin (decided). Alternative: JST GH
  14-pin (latching, smaller). Cost to change later: one footprint per
  board edge and the cables; pinout stays.
- **Unit shunt / limit**: ~3 mOhm, ~18 A. Alternative: 2.5 mOhm, 22 A.
  Cost to change: a resistor value, if the footprint is 2512.
- **Strip side**: far side (right of slot B), next to the expander
  (decided). Alternative: core side. Cost to change: the strip layout mirrors;
  cheap before routing, expensive after.
- **Core input**: XT30PW. Alternative: a 2-pole screw terminal. Cheap.

### Known unknowns and how the plan absorbs them
- LCSC stock of the PH 14-pin side-entry header, XT60PW-M, XT30PW-M,
  the 6 mOhm 2512: checked before the schematic; pivot to a clone /
  screw terminals if missing.
- Unit hot-swap SOA with one FET at the 530 W level: default two FETs
  if the strip fits them; one if the SOA check passes.
- Left-edge mounting holes against slot A's standoffs: default M2.5 if
  M3 does not fit beside S2 / S3.

### The mechanical work
- gensch.py: two projects or two root sheets (core, unit) instead of
  one N-dependent root; unit.py keeps its body, gains the chain ports
  and the input block (from input.py, scaled).
- Unit board: place.py / route_hand.py on one unit plus the strip; no
  copycol, no netsplit per unit (one unit per board); the In2 bus,
  pad vias and keep-outs stay.
- Core board: core.py / route_hand.py core() minus the tile copper,
  plus the XT30 input and the chain connector; the In2 channel rows end
  at the chain connector instead of at unit 1.
- Docs: ADR 0004 marked partly superseded, handoff, plane requirements
  (R7-R9a, R21) checked against per-unit inputs.

Review asks: none open. Decided by the user 2026-10-29: chain on JST
PH 2.0; 3V3 from the core; unit hot-swap LM5069-1 latch-off (stays off
after a fault until the MCU resets it over U_OFF or the 48 V cycles);
input strip on the far side (right of slot B).

---

## Open Questions

**Architecture-changers**
- [ ] Two KiCad projects (power/pp-48-core, power/pp-48-unit) or one
  project with two boards? Default: two projects, shared tools.

**Behavior definers**
- [ ] Unit hot-swap power limit and timer vs two slot insertions.
- [ ] Firmware: chain topology discovery (which channel answers) and
  re-scan on a NAK.
- [ ] Unit present signal on the chain (a pin pulled low by the unit)
  or presence by I2C only. Default: I2C only.
