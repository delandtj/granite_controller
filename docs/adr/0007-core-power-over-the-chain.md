# Plane core powered over the chain joint (no core XT30)

**Status**: Rejected (user, 2026-11-04)
**Date**: 2026-11-04

Related: docs/adr/0005-chained-plane-boards.md (decision 3: the unit
latch judges the core's own 48 V input; decision 4: the 2x7 chain joint,
"no VBUS in the chain"; decision 6: the core's own XT30 input),
docs/adr/0006-plane-link-can-fd.md (decision 4: every core feeds
AUX_12V for the controller), docs/adr/0003-plane-tile-and-module-interface.md
(decision 8: no fuses in the fluid, bus TVS per board),
firmware/docs/adr/0002-plane-core-firmware.md (decision 7a: the latch
timing). Scope: the CAN FD generation (pp-48-core-fd, pp-48-unit-fd).
Rev D keeps its XT30.

---

## Rejection (2026-11-04)

Rejected by the user on review: the bused CORE_VIN puts every unit's
power on every unit. The diodes and limiters stop current flowing back,
but the units' + sides, deliberately separate today ("no VBUS in the
chain", ADR 0005 decision 4), would meet on one line through every
board: a fault on that line involves every unit's input, a unit taken
out and handled still has 48 V on its joint pins fed by the others, and
48 V leaves each unit's input strip to cross the whole board.

The unit-1-only variant (below, Alternatives) avoids the sharing but
makes unit 1's XT60 the plane's single point (core, management of units
2-4, its share of AUX_12V) and still puts the feeder on every unit board
for the one in position 1. Not worth one lead per plane.

Outcome: the core keeps its own XT30 input (ADR 0005 decision 6) in
the CAN FD generation too. The rest of this document is the proposal
as reviewed, kept as the record.

---

## Context

Every plane core has its own 48 V input today: J1, an XT30PW-M, with
bus TVS D1, input caps and no fuse (ADR 0005 decision 6). On it hang
the 3V3 buck U1 (LM5164, the core and, over the chain's +3V3 pin, the
logic of up to four units: ~0.2 A), the Fly-Buck U2 (LM5018) for the
isolated 12 V that feeds AUX_12V and with it the controller (ADR 0006
decision 4), and the divider R15 / R16 to VIN_SNS, the 48 V witness of
the unit latch rule (ADR 0005 decision 3).

The user wants the extra cable gone (2026-11-04): each plane needs one
XT30 lead from the bus next to the XT60 leads of its units, for a load
of a few watts.

What the input is used for, and has to keep doing:

- **Power**: about 1 W for 3V3 (core + 4 units' logic) and up to ~2.5 W
  on ISO_12V when this core alone carries the controller (2 W) plus the
  link LDOs. With converter losses ~4 W, 0.11 A at the 38 V bottom of
  the bus.
- **The witness**: the latch rule reads the raw bus on the core's own
  input. The units' INA237 sits behind the hot-swap and reads ~0 V
  exactly when the stage is open, so it cannot stand in (ADR 0005
  decision 3).
- **Independence from any one unit**: a unit's hot-swap can be latched
  off by the core; the core must not depend on a stage it controls.

The chain joint (power/pp-48-fd/tools/sch/blocks/chain.py) is a CJT
A2541 2x7: 1 EXP_RSTN, 2 PRESS_EN, 3/4 P3, 5/6 P2, 7 GND, 8 GND, 9/10
P1, 11/12 P0, 13 +3V3, 14 GND. No spare pin. The unit's raw bus (net
VIN, XT60 J201 to the LM5069 U207, with D201 SMDJ60A and C201 / C202)
lives only in the input strip at the right edge (x 146..156 of
91..169.5); the chain plug J301 is at the left edge.

---

## Decision

1. **A core-supply line in the chain, fed by every unit.** A new net
   CORE_VIN runs straight through the chain like PRESS_EN and EXP_RSTN:
   the same pin on every unit's plug J301 and socket J302, bused. Every
   unit drives it from its own raw VIN (before the hot-swap, after
   D201) through a current limiter and a series Schottky (decision 3).
   The core takes CORE_VIN from its right socket J301 as its VIN; J1
   (XT30) goes. Any one unit with its XT60 in powers the core.

2. **Joint: 2x8 instead of 2x7**, the same CJT A2541 series (plug
   A2541WR-2x8P, socket A2541HWR-2x8P; stock to check). One column
   added at the pin-15 / 16 end:
   - 15 CORE_VIN, 16 GND;
   - +3V3 moves from 13 to 14 and pin 13 becomes GND, so CORE_VIN's
     neighbours are GND on both sides (13 in its row, 16 across);
   - pins 1-12 unchanged.
   The 48 V pin then sits next to nothing it can damage if a joint
   is pushed on askew or a probe slips.

3. **Per-unit feed, in the input strip**: raw VIN -> current limiter
   (~0.3 A limit, thermal shutdown, auto-retry; a 60 V-class eFuse, part
   to pick at JLC) -> Schottky (>= 100 V, 1 A class, e.g. SS110 /
   B1100) -> CORE_VIN. The limiter is what makes a live 48 V pin on an
   open plug acceptable: a short at any plug or on the line is held at
   the limit by every unit feeding it and nothing burns. It is not a
   fuse (ADR 0003 decision 8): nothing opens for good, it limits and
   retries. The diode keeps CORE_VIN from back-feeding a unit whose
   XT60 is out.

4. **CORE_VIN across the unit**: one track from the strip to the left
   plug's pin 15 and on to the right socket's pin 15 (bused, decision
   1), on an inner layer where the unit has room, 0.3 mm wide (0.3 A
   with margin). Clearance: the Default 0.15 mm the other 48 V nets
   (VIN, VBUS) already use is inside IPC-2221 for 31-100 V on coated
   external and on inner layers; keep it, and give CORE_VIN its own net
   class only if the router needs a handle.

5. **Core input**: J1, its keying and its copper go. D1 (SMDJ60A), C1
   and the ceramics stay on VIN, now fed from J301 pin 15. VIN_SNS
   (R15 / R16) is unchanged and reads CORE_VIN: the highest unit input
   minus one Schottky drop (0.3-0.5 V at 0.1 A). Firmware subtracts a
   nominal 0.4 V before applying the latch window (decision 6).

6. **Latch rule keeps its meaning**: "the core judges the bus" now
   reads "the core judges the highest unit input". All units hang on
   the same bus behind the same breaker, so that is the bus whenever
   any unit has its XT60 in, which is the only time the core runs at
   all. A unit whose own XT60 came out while others are in still reads
   as a unit fault and is latched, as today (ADR 0005 decision 3, the
   cost noted there is unchanged). Firmware ADR 0002 decision 7a gets
   the 0.4 V offset and nothing else.

---

## Architecture Overview

### Component Breakdown

1. **Unit core feed** (power/pp-48-fd/tools/sch/blocks/uinput.py, new
   parts beside D201)
   - VIN -> limiter -> Schottky -> CORE_VIN.
   - Interfaces: VIN (raw bus), CORE_VIN (chain bus), GND.
2. **Chain joint** (power/pp-48-fd/tools/sch/blocks/chain.py)
   - 2x8, pinout of decision 2, CORE_VIN bused through like PRESS_EN.
   - Footprints: KiCad PinHeader / PinSocket_2x08_P2.54mm_Horizontal.
3. **Core input** (the core input block, cinput.py)
   - J1 removed; VIN = CORE_VIN from the right socket; TVS, caps and
     VIN_SNS unchanged.
4. **Firmware** (plane-fw, the latch in firmware ADR 0002 decision 7a)
   - VIN_SNS minus 0.4 V before the window.

### Data Flow / Interaction

```
 core              unit 1                    unit 2 (and 3, 4)
 +-----------+     +----------------------+  +----------------------+
 | VIN: TVS, |     | J301   CORE_VIN  J302|  | J301          J302   |
 | caps,     |<-15-|  15 ======+===== 15 |--|  15 ======+===== 15 |-- open
 | buck, FB, |     |           ^          |  |           ^          |
 | VIN_SNS   |     |     Schottky        |  |     Schottky        |
 +-----------+     |     limiter          |  |     limiter          |
                   |  XT60 -> VIN -> LM5069 -> VBUS (slots)        |
                   +----------------------+  +----------------------+
 GND: chain pins 7, 8, 13, 14, 16 (3V3 and CORE_VIN return)
```

---

## Alternatives Considered

### Keep the core's own XT30 (status quo)
- **The idea**: no change; one more lead per plane.
- **Optimizes for**: independence: the core's supply and witness come
  straight from the bus, no part between.
- **Sharpest tradeoff**: the cable the user wants gone, and a connector
  plus a lead per plane.
- **Bets on**: the extra lead being no real cost in assembly or service.

### Feed the core from unit 1 only
- **The idea**: unit 1's raw VIN to the core over the joint, no line
  through the other units.
- **Optimizes for**: one feed circuit per plane in use (still one per
  unit board, since the boards are identical).
- **Sharpest tradeoff**: unit 1's XT60 becomes the plane's single
  point: out, and the core, its management of units 2-4 and its share
  of AUX_12V go with it.
- **Bets on**: unit 1's lead never being the one pulled.

### Feed from unit 1's VBUS (after the hot-swap)
- **The idea**: no limiter needed, the LM5069 already limits.
- **Optimizes for**: part count.
- **Sharpest tradeoff**: circular: the core latches unit 1 off and
  loses its own supply; the witness reads ~0 V when the stage is open.
- **Bets on**: the core never latching unit 1. It will.

### Power the core from the controller over the link
- **The idea**: the controller's 12 V down the IDC ribbon and AUX_12V.
- **Optimizes for**: no 48 V on the chain at all.
- **Sharpest tradeoff**: reverses ADR 0006 decision 4 (the planes feed
  the controller), needs a supply on the controller, puts every
  plane's management behind the controller, and pushes amps through
  2.54 mm ribbon conductors; the core still needs an isolated converter
  (link domain to bus domain) and a 48 V sense input.
- **Bets on**: the controller being more available than any unit's
  XT60, which inverts the design.

### Repurpose a GND pin of the 2x7
- **The idea**: pin 7, 8 or 14 becomes CORE_VIN; no part change.
- **Optimizes for**: keeping the stocked 2x7 parts and the footprint.
- **Sharpest tradeoff**: the 48 V pin sits next to an I2C pair or
  +3V3, and the 3V3 return of four units is down to two pins.
- **Bets on**: no joint ever being pushed on askew, and no probe
  slipping. It also revives if the 2x8 is not stocked.

---

## Consequences

### Positive
- One lead and one XT30 less per plane; the core can be placed without
  a cable entry.
- The core runs while any unit of its plane has power, not only while
  its own lead is in.
- The witness still reads the bus, with a known offset.

### Negative
- Every unit carries a limiter, a Schottky and a track across the
  board; the unit (routed and ready for rev D) is re-laid for FD.
- 48 V on every unit's open plug (limited, but live).
- A core alone on the bench needs a feed: a unit, or a bench adapter
  (2x8 socket with a lab-supply lead).
- The joint grows from 2x7 to 2x8 (2.54 mm longer on every board).

### Risks
- **2x8 stock**: the A2541 2x7 parts were thin already (87 plugs on
  2026-11-02). Mitigation: check before drawing; the GND-pin
  alternative above is the fallback.
- **Limiter start-up into the core's caps**: C1 47 uF plus ceramics
  behind a 0.3 A limit charges in ~6 ms at 40 V; an eFuse with a short
  fault timer could read that as a fault. Mitigation: pick one with
  current-limit (not circuit-breaker) behaviour, or a soft-start
  capacitor; check on the bench.
- **Many feeders on one line**: four limiters in parallel share the
  load unevenly (the highest VIN wins); harmless, each one is sized for
  the whole core.

---

## What an Expert Would Ask

**Q: Is a live, current-limited 48 V pin on every open plug safe?**
A: Electrically yes: 48 V is under the 60 V DC touch limit the plane
already works with everywhere, and the limiter holds a short at ~0.3 A
indefinitely (thermal shutdown and retry, no arc). Mechanically the pin
is fenced by GND on both sides. What it does not stop is a short
between CORE_VIN and a signal pin on a mis-mated joint; the GND fence
makes that need two pins of misalignment.

**Q: What happens when the last powered unit loses its input?**
A: The core goes down with it, which is also what happens today when
the core's XT30 is out. The difference is that today the core can lose
power while all units run; with this ADR it runs as long as any unit
does. Units without a core run on hardware defaults (slots ON, hot-swap
ON), as now.

**Q: Does the Schottky offset make the latch rule wrong near the
window edges?**
A: The window is the stage's UVLO / OVLO (~35.3-38.3 V on, 64.6 V off),
set by the unit's own dividers with resistor tolerance of the same
order as the 0.3-0.5 V diode spread. Subtracting a nominal 0.4 V keeps
the core's judgement inside that tolerance. Near the edges the rule was
already fuzzy by design (it debounces 500 ms / 300 ms).

**Q: Why not keep it simple: a resistor instead of a limiter?**
A: A resistor small enough to pass 0.11 A without much drop (a few ohms)
lets ~10-20 A through into a short and burns open, which is a fuse by
another name (ADR 0003 decision 8). A resistor large enough to limit a
short to a safe current drops too much at load. The limiter is the
smallest part that does both.

**Q: Does a fault on one unit's feed take the core down?**
A: A short inside one unit's feed path on the CORE_VIN side loads the
whole line, and every limiter holds it: the core browns out until it is
cleared. A failure on the VIN side of a unit (its XT60 out, its limiter
dead open) only removes that unit as a feeder.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **Every unit feeds the line**
  - **Choice**: bused CORE_VIN, a feeder per unit.
  - **Alternative**: unit 1 only.
  - **Cost to change later**: low before the unit is re-laid; after,
    a re-spin of the unit.
- **2x8 joint**
  - **Choice**: 2x8, CORE_VIN at 15 fenced by GND, +3V3 to 14.
  - **Alternative**: repurpose GND pin 14 of the 2x7.
  - **Cost to change later**: every FD board's joint footprint.
- **Limiter**
  - **Choice**: 60 V eFuse, ~0.3 A, auto-retry.
  - **Alternative**: discrete BJT or depletion-MOSFET limiter.
  - **Cost to change later**: one block on the unit.

### Known unknowns and how the plan absorbs them
- **A2541 2x8 stock at JLC**: default 2x8; pivot to the GND-pin
  alternative if either part has less than a build's worth in stock.
- **eFuse part**: default a 60 V-class eFuse in JLC stock with
  adjustable limit down to ~0.3 A; pivot to a discrete limiter if none
  is stocked or none starts the core's caps without tripping.
- **Room for the track across the unit**: default an inner layer
  between the slot columns; pivot to routing it along the top edge on F
  if the inner layer has no gap.

### The mechanical work
- chain.py: 2x8 pinout, CORE_VIN bused; uinput.py: the feeder;
  cinput.py: J1 out, VIN from the joint.
- Unit and core boards: joint footprints (all four joint positions),
  feeder placement in the strip, the CORE_VIN track, core input copper.
  ERC / DRC / parity / isocheck as for every FD change; JLC stock.
- Firmware: VIN_SNS offset in the latch (firmware ADR 0002 decision
  7a), one constant.
- Docs: ADR 0005 decisions 4 and 6 note this ADR; the manual's wiring
  pages lose the core lead.

Review asks:
1. Every unit feeds the core (bused line), or unit 1 only?
2. 2x8 joint, or repurpose a GND pin of the 2x7?
3. eFuse limiter at ~0.3 A acceptable as "not a fuse" under ADR 0003
   decision 8?
4. A bench adapter (2x8 socket + lab lead) for a lone core: needed, or
   is bench work always with a unit?

---

## Open Questions

**Architecture-changers**
- [ ] Every unit feeds, or unit 1 only (review ask 1).
- [ ] 2x8, or the 2x7 with a GND pin given up (review ask 2).

**Behavior definers**
- [ ] The limiter part and its limit; its start-up into the core's
  input caps (bench check).
- [ ] The Schottky offset: fixed 0.4 V in firmware, or calibrated at
  commissioning.
- [ ] Does the controller-side 12 V budget (ADR 0006: one core carries
  the controller) still hold when that core's input is a 0.3 A limited
  line? At 38 V that is 11 W, so yes on paper.
