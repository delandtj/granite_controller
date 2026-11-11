# Chained plane: one core board and identical 2-slot unit boards

**Status**: Accepted (2026-10-29, implementation started: power/pp-48-core,
power/pp-48-unit)
**Date**: 2026-10-29

**Updated**: 2026-10-29 (user: chain on JST PH 2.0, 3V3 from the core,
unit hot-swap latch-off, input strip on the far side); 2026-10-30 (user:
unit hot-swap LM5069-2 auto-retry with the latch in firmware, decision 3;
INA237 for INA238); 2026-11-02 (user: no chain cable, the boards plug
into each other on a mounting bar: decisions 4 and 7, the "Direct
board-to-board mating" alternative taken); 2026-11-11 (hot-swap SOA
check done on the routed FD unit: the fitted FETs fail it on a short,
decision 3a; replacement FET in the source for the next order, FD
batch 1 unchanged)

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
(decided 2026-10-29: short signal cable between boards, 2026-11-02:
boards plugged into each other on a bar instead; per-unit
XT60PW input, input stage in a side strip, core board with its own
small 48 V input).

What the rev1 work already gives: the unit (two 25 mm slot columns, one
I2C segment, TCA6424A, In2 bus for slot A's lines) is drawn and routed
as a block, and the rotating channel (ADR 0004 decision 5) makes every
unit's copper identical while each unit still gets its own mux
channel. Both carry over; the channel just crosses a board joint
instead of a column edge.

---

## Decision

1. **Two board designs.** A core board (MCU, mux, link, isolated 12 V,
   3V3 buck) and a unit board (two slots + input stage). A plane is
   one core and 1-4 units in a row: N = 2, 4, 6, 8. No end strip, no
   end cap, no per-N board.

2. **Unit board**, 78.6 x 72.1 mm (as routed):
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
   LM5069-2 (auto-retry), the core's tile circuit scaled to one unit:
   2 slots x ~265 W = 530 W, 14 A at 38 V. Shunt ~3 mOhm (55 mV
   limit ~18 A, e.g. 2 x 6 mOhm 2512), one or two BSC040N10NS5 (SOA
   to check at the unit power limit), UVLO / OVLO / timer as the
   slots, INA237 on the shunt at 0x44 on the unit segment. Control
   over the TCA6424A's two spare lines: P13 U_OFF (out, Q pulls UVLO
   low, 100k pull-down = ON at reset) and P27 U_PGD (in). The
   INA237 ALERT is not wired (polled; DIAG_ALRT).
   **Latch in firmware** (user 2026-10-30; was the LM5069-1's own
   latch-off, which had 1 part in stock at JLC against ~8k for the -2):
   after a fault the -2 restarts on its own at ~0.5 % duty. The core
   latches the unit off: U_PGD falling while the unit input is inside
   UVLO..OVLO and U_OFF is not commanded is a unit fault; the core
   drives U_OFF high and keeps it high (power state `fault`, unit
   hot-swap latched off) until `unit_reset` or a core power loss.
   U_PGD low with the input out of range is the bus, not a fault: no
   latch. The input the core judges is **its own 48 V input**, not the
   INA237: U208's VBUS pin sits on the hot-swap output (see uinput.py),
   so the only absolute voltage that part reports reads ~0 V exactly
   when the stage is open, which is every moment the rule has to judge.
   Core and units hang on the same bus behind the same breaker, so the
   core's divider is the live witness. The cost: a unit whose own XT60
   came out while the bus is up is latched off and wants an operator's
   `unit_reset`. Timing, the boot case and the debounce are the
   firmware's: firmware/docs/adr/0002-plane-core-firmware.md
   decision 7a. With no core (or the expander in reset) U_OFF is
   low and the unit retries forever: hardware default ON (R14). The
   0.5 % duty bounds the average power only; the single 10 ms pulse of
   each retry is what the FET has to survive, and the fitted FETs do
   not (decision 3a; this sentence claimed they did until 2026-11-11).
   The retry costs one restart attempt (~10 ms fault timer) before the core
   reacts; acceptable, the slots' own hot-swaps behave the same.

3a. **Hot-swap SOA check** (2026-11-11, desk check against the
   datasheets on pp-48-unit-fd as routed; nothing measured). On a
   short the LM5069 circuit breaker opens the FET, then holds it in
   power limit at the full input voltage until the fault timer ends;
   the -2 repeats that pulse on every retry. With the values fitted
   (TI SNVS452G: P_LIM = R_PWR / (1.3e5 x R_S) + 1.18 mV x V_DS / R_S,
   t_fault = C_T x 4 V / 85 uA):

   | | Slot (U1 / U51) | Unit (U207) |
   |---|---|---|
   | R_S, R_PWR, C_T | 3.5 mOhm, 13.7k, 220 nF | 3.0 mOhm, 20.0k, 220 nF |
   | Current limit / circuit breaker (min / typ / max) | 13.9 / 15.7 / 17.6 A, 23 / 30 / 37 A | 16.2 / 18.3 / 20.5 A, 27 / 35 / 43 A |
   | Power limit at V_DS = 48 V (60 V) | 46 W, 0.96 A (50 W, 0.84 A) | 70 W, 1.46 A (75 W, 1.25 A) |
   | Sense voltage in power limit (5 mV recommended minimum) | 3.4 mV | 4.4 mV |
   | Fault time | 10.4 ms typ, about 20 ms worst case | the same |
   | FET fitted | BSC070N10NS5 | 2 x BSC040N10NS5 |
   | FET SOA at 48 V, 10 ms, T_C = 25 C | 0.15 A | 0.15 A each |
   | Asked / allowed | about 6 | about 10 on one FET, 5 if shared |

   - **Short: fails.** Both fitted types are trench switching FETs
     whose SOA falls with about V_DS^-2 in linear mode (1 ms at 48 V:
     about 0.5 A; still under the demand). Expect the slot FET to
     fail, most likely shorted, on a module input short (input
     ceramic, TVS), the unit FETs after it, and the fault to end at
     the 48 V supply's limit or breaker.
   - **Switch-on: passes at 48 V, thin.** A slot charges the module's
     about 12 uF in power limit in about 0.3 ms; BSC070N10NS5 allows
     about 1.7 A for that time at 48 V against 0.96 A asked. At 60 V
     it is slightly outside (0.84 A asked, about 0.75 A allowed).
   - **R_PWR and C_T do not fix it**: a fault time short enough for
     these FETs is shorter than the switch-on needs, and the power
     limit is already below its accurate range.
   - **Fix: a linear-mode FET on the same PG-TDSON-8 footprint**, all
     four positions (Q1, Q51, Q202, Q203): Infineon ISC035N10NM5LF2
     (OptiMOS 5 Linear FET 2, 100 V, 3.5 mOhm, datasheet rev 2.3).
     SOA at 48 V: about 7 A for 10 ms at T_C = 25 C, about 6 A at
     125 C, about 2.9 A for 100 ms; about 5 A at 60 V for 10 ms. That
     is about 5x on a slot and 4x on the unit with one FET taking
     everything. No other value changes. In the source since
     2026-11-11 (gensch blocks unit.py / uinput.py, schematic and
     board fields; copper untouched, the footprint is the same
     PG-TDSON-8): the next unit-fd order carries it. JLC lists it as
     C22408528, extended, 58 in stock on 2026-11-11 (4 per unit).
     Open: the rework of boards already built (FD batch 1, tag
     fd-batch1, carries the BSC parts). Second
     candidate, SOA not read: Nexperia PSMN4R8-100YSE (LFPAK56E).
     Read and rejected: TI CSD19531Q5A / 19532Q5B / 19533Q5A and
     BSC060N10NS3 (0.7 to 1.7 A at 48 V, 10 ms).
   - **Selectivity** (does a slot fault stay on the slot). Overload or
     soft short: yes once the FET holds; the slot folds back to about
     1 A in well under a millisecond, and the unit, which may touch
     its own current limit for that moment (slot limit plus the other
     slot's load against 16.2 A minimum), never runs out its timer.
     Dead short: a race of 1 to 2 us between the slot breaker opening
     and the unit breaker's threshold; the breaker ranges overlap and
     the unit carries the other slot's load on top. Estimated just
     under the unit's minimum threshold with the other slot running
     (its module's 11 uF holds the bus up), over it with the other
     slot idle (1.8 uF on VBUS). If the unit breaker trips, the other
     slot loses its bus for a few ms and its node goes down. Not
     settled at the desk: docs/unit-fd-emi-bench-plan.md test 6. The
     remedies would be more VBUS capacitance or a series inductance
     per slot (the same change the ripple review points at).

4. **Chain link**: the boards plug straight into each other, in the
   plane of the boards, no cable (user 2026-11-02: no cable; was a JST
   PH 2.0 14-pin flat cable, 2026-10-29). A right-angle 2x7 2.54 mm pin
   header (the plug) at a unit's left edge mates a right-angle 2x7
   socket at the core's and a unit's right edge, both on B, both CJT
   A2541 (gold over nickel; one series, so the two rows sit at the same
   heights: 5.08 / 5.00 mm bodies on the board). Plug A2541WR-2x7P
   (C5136987, 87 in stock 2026-11-02; KiCad PinHeader_2x07_P2.54mm_Horizontal):
   body flush with the edge, pins 6 mm past it. Socket A2541HWR-2x7P
   (C19100281, 430; KiCad PinSocket_2x07_P2.54mm_Horizontal, 8.5 mm deep):
   face 4 mm past the edge, so its pads clear the parts behind it on
   the core (U6, U301, L1) and in the unit's strip. Fully mated the
   boards stand 4 mm apart; 4..6 mm of gap = 6..4 mm of the pins
   inside: the bar (decision 7) sets the gap, the joint takes +-1 mm of
   it. No latch: the boards are screwed down, the joint carries no
   load. 14 pins (user 2026-10-29, docs/press-deadline-line.md): P0..P3
   SDA/SCL (8), 3V3, GND x 3, PRESS_EN, EXP_RSTN. The rotating channel
   goes through the board unchanged: the left connector's P0 is this
   unit's segment, its P1..P3 are wired to the right connector's
   P0..P2, the right connector's P3 is not connected. At the core
   connector Pk = mux channel k (4 used, as rev1). PRESS_EN and
   EXP_RSTN are bused (straight through every unit). No VBUS in the
   chain.
   Pinout (power/pp-48/tools/sch/blocks/chain.py): pin 1 at the top on
   both, the even pins in the row nearer the edge, so plug pin n meets
   socket pin n and IN and OUT share one pinout, top to bottom (odd /
   even): 1 EXP_RSTN / 2 PRESS_EN, 3 / 4 P3 SDA / SCL, 5 / 6 P2, 7 / 8
   GND, 9 / 10 P1, 11 / 12 P0, 13 3V3 / 14 GND; pins at v 42.5..57.74
   on every board (3V3 and P0 at the bottom, by the unit's In2 channel).
   Removing a middle unit means sliding the ones after it 6 mm away.
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
   (56.1 x 72.1 mm as routed), mounting holes in its corners.

7. **Mounting**: every board on its own holes (unit: two in the input
   strip plus two at the left edge, positions to fit around slot A's
   standoffs; core: four corners), all on one mounting bar (user
   2026-11-02). Boards 4-6 mm apart in a row (decision 4: nominal 5,
   5 mm of the plug's pins in the socket); the bar holds the boards'
   height and the pin rows' line within ~0.2 mm.

### What changes in ADR 0004

| 0004 decision | Now |
|---|---|
| 3 Stackup and VBUS | per unit board: In2 VBUS from the unit hot-swap |
| 4 Family | 1 core + 1-4 identical unit boards, no per-N board |
| 5 Rotating channel | same scheme, through the chain joints; 4 positions |
| 7 Input current scales with N | one 14 A input per unit; no 30 A tile, no end cap, no VBUS split |
| 8 Plane mounting | holes per board |
| 1, 1a, 2, 6, 9 | unchanged |

---

## Architecture Overview

### Component Breakdown

1. **Core board** (power/pp-48 core sheets: tools/sch/blocks/mcu.py,
   mux.py, link.py, supplies.py; the input block shrinks to the small
   core input)
   - MCU, mux (I2C2 upstream), controller link (ISO1640 I2C +
     ISO7721 EXP_INT, link.py), 12 V isolated module, 3V3 buck.
   - Interfaces: XT30 48 V in, link connectors, 12 V out, chain
     socket (P0..P3, 3V3, GND).
2. **Unit board** (tools/sch/blocks/unit.py + a unit input block)
   - Two slots, TCA6424A, In2 bus, input strip with hot-swap and INA238.
   - Interfaces: XT60 48 V in, chain plug (left), chain socket (right),
     per slot the node terminal and the front-panel GH.

### Data Flow / Interaction

```
 48 V bus (breaker outside the fluid, ADR 0003 decision 8)
   |XT30          |XT60               |XT60               |XT60
 +------+  2x7  +-------------+  2x7  +-------------+  2x7  +---- ...
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

### Short cable between the boards (the 2026-10-29 choice)
- **The idea**: JST PH 2.0 14-pin side-entry header at each edge, a flat
  ~50-100 mm cable per joint.
- **Optimizes for**: loose mounting; the cable takes the gap and any
  misalignment.
- **Sharpest tradeoff**: a hand-made cable per joint (no stock 14-pin PH
  cables; 12-pin ones exist), wired pin n to 15 - n; the user would not
  crimp them (2026-11-02).
- **Bets on**: boards held on their own holes without a common carrier.

### Bridge board across the seam
- **The idea**: vertical 2x7 headers at both board edges, a small PCB
  with two sockets plugged across the seam.
- **Optimizes for**: any board lifts out on its own.
- **Sharpest tradeoff**: an extra part per joint, ~11 mm tall on the
  board.
- **Bets on**: room above the seams.

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
- Connectors in the fluid at every joint (signal only, gold, no latch:
  the boards are screwed to the bar).
- One hot-swap and one XT60 cable per 2 slots instead of per 4.
- The plane row gets longer: units ~66 mm instead of 50 mm, plus gaps;
  N = 8 is ~40 + 4 x 66 + gaps, ~315 mm.
- The units' logic depends on the core's 3V3 (hardware defaults hold
  when it is missing).
- The rev1 core and routed board are partly thrown away (the tile
  input moves into the unit).

### Risks
- **I2C over the joints**: the worst segment (unit 4) crosses 4 joints
  and 4 boards, ~0.3 m. At 100 kHz with 4.7k pull-ups per segment this is
  well inside 400 pF; keep the GND pins between the pairs.
- **Header contacts in the fluid over years**: gold-flash 2.54 mm
  pins in a friction socket, no latch. Mitigation: soak test; the bar
  holds the boards so nothing pulls on the joint.
- **Bar tolerance**: the plug's pins bend if the bar puts the rows more
  than a few tenths of a mm apart in height or along the edge. The
  boards' holes and the bar set it; check the first frame with the
  boards screwed down and the joints unplugged.
- **3V3 drop over 4 joints**: ~0.2 A x 4 joints x ~40 mOhm per contact
  pair is well under 50 mV.

---

## What an Expert Would Ask

**Q: A unit is plugged in with the core already running. Does anything break?**
A: The chain carries only 3V3, GND and I2C. Hot-plugging a chain
joint can glitch the segment of that unit and the ones behind it; the
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
A: Each slot limits at 15.7 A and breaks at 30 A on its own. A single
slot overload trips only that slot; on a dead short the slot and unit
breakers race and the unit may trip too (decision 3a, 2026-11-11; this
answer said a slot fault always stays below the unit limit). The
unit limit only catches a fault on the unit's own bus (caps, TVS,
traces) or both slots high at once. The power limit / timer of the
unit LM5069 must ride through both slots' insertion (160 ms each):
to size with the SOA check of decision 3.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **Chain connector**: right-angle 2x7 2.54 mm plug / socket, boards
  mated directly (decided 2026-11-02). Alternative: a bridge board
  across the seam. Cost to change later: one footprint per board edge
  and the In2 band heads on the unit (tools/route_unit.py).
- **Unit shunt / limit**: ~3 mOhm, ~18 A. Alternative: 2.5 mOhm, 22 A.
  Cost to change: a resistor value, if the footprint is 2512.
- **Strip side**: far side (right of slot B), next to the expander
  (decided). Alternative: core side. Cost to change: the strip layout mirrors;
  cheap before routing, expensive after.
- **Core input**: XT30PW. Alternative: a 2-pole screw terminal. Cheap.

### Known unknowns and how the plan absorbs them
- LCSC stock of the chain plug (87 at 2026-11-02) and socket, XT60PW-M, XT30PW-M,
  the 6 mOhm 2512: checked before the schematic; pivot to a clone /
  screw terminals if missing.
- Unit hot-swap SOA with one FET at the 530 W level: default two FETs
  if the strip fits them; one if the SOA check passes. Checked
  2026-11-11: neither one nor two of the fitted type pass (decision 3a).
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
PH 2.0 (2026-11-02: boards plug into each other, 2x7 plug / socket on a
mounting bar); 3V3 from the core; unit hot-swap LM5069-2 auto-retry
with the latch in the core firmware (decision 3; 2026-10-30, was
LM5069-1 latch-off on 2026-10-29);
input strip on the far side (right of slot B).

---

## Open Questions

**Architecture-changers**
- [x] Two KiCad projects (power/pp-48-core, power/pp-48-unit), shared
  tools: built that way (the FD generation the same).

**Behavior definers**
- [ ] Unit hot-swap power limit and timer vs two slot insertions.
- [ ] Hot-swap FETs: the fitted BSC070N10NS5 / BSC040N10NS5 are outside
  their SOA on a short (decision 3a). ISC035N10NM5LF2 is in the source
  for the next order; decide the rework of FD batch 1 (JLC stock 58 on
  2026-11-11 covers the slot FETs of 24 units, not all four positions).
- [ ] Dead short on one slot: does the unit breaker trip with it
  (decision 3a, bench plan test 6).
- [x] Firmware: chain topology discovery and re-scan: plane-logic
  discovery.rs (probe 0x22 per channel, 2 s re-scan, lost after two
  NAKs).
- [x] Unit presence: I2C only (no present pin on the 2x7 chain).
