# PP-48 unit-fd: EMI bench test plan

**Status**: plan, nothing measured yet (2026-11-11). FD batch 1 boards
(tag `fd-batch1`) have not arrived.

**Question this answers**: can the pp-48-unit-fd boards be used as built,
or does the module ripple they pass to the 48 V bus, or its coupling into
the board's logic and the CAN link, need a change first?

Related: docs/adr/0005-chained-plane-boards.md (unit, hot-swaps),
docs/adr/0006-plane-link-can-fd.md (link, decision 12),
docs/pm-19v-250w-b.md (module, input ripple, risk 5),
docs/pm-interface.md (slot currents). Test 6 (short on one slot) is the
bench side of ADR 0005 decision 3a.

## Background: what the layout review found

A desk review of the routed board (2026-11-11) found no layout problem:
In1 is a solid GND plane, the high-current paths are plane over plane
(VBUS on In2, return on In1), C2 / C52 and the slot TVS sit between each
module's VIN and GND standoffs, and the CAN pair has unbroken copper
under it. What it left open is all about the modules' input ripple:

1. **The unit does not filter.** The paths from a module back to the XT60
   are milliohms (slot shunts + Q1, unit shunts + Q202 / Q203) and the
   unit's capacitors are small next to the module's own 11 uF. Ripple at
   the module input, on the VBUS plane and at the XT60 is expected to be
   the same to within a few percent.
2. **C201 (47 uF electrolytic) takes a large share of what leaves the
   modules** and may be over its ripple rating. The only rating found for
   the Lelon RVT2A470M1010 is 148 mA at 120 Hz (distributor listing); no
   100 kHz rating or ESR was found.
3. **The VBUS plane is the reference of everything on B.Cu**, and its only
   VBUS-to-GND ceramics are C214 / C215 in the strip. Steady-state
   coupling into the logic was estimated small (below); fast bus
   transients were not estimated.
4. **The CAN pair runs about 85 mm on B over the VBUS plane** (not link
   copper, which is what ADR 0006 decision 12 words). Coupling is equal
   on both lines, so it should land across the isolators as common mode.

### Predicted numbers

From a harmonic model (power/pp-48-fd/tools/unit_ripple.py, assumptions
in its header), not a measurement: two PM-19V-250W-B modules at
full load (252 kHz, 13.2 A out, 11 uF on each module), ceramics derated
for 48 V bias, C201 ESR assumed 0.4 ohm, 0.5 uH supply cable.

| Quantity | Predicted |
|---|---|
| Ripple at module input, VBUS plane, XT60 (252 kHz component) | about 0.4 V peak, 0.8 V p-p, the same at all three |
| Second harmonic (504 kHz) at the same points | about 60 mV peak |
| Ripple current in the supply cable | about 0.35 A rms |
| Ripple current in C201 | 0.3 to 0.7 A rms for an ESR of 1.0 to 0.4 ohm |
| 252 kHz on PRESS_EN (100k pull-down, 36 mm over VBUS) | about 0.17 V peak |
| 252 kHz on the latch inputs (10k pull-ups) | under 40 mV |

The model also says more ceramics on VBUS barely help (ten more 2.2 uF:
about 2 dB) and that only a series inductor per slot does (1 uH per slot
plus four 2.2 uF: about 15 dB). That is a design change with its own
questions (resonance with the module's 11 uF near 46 kHz, damping, unit
or module side), so it is only worth opening if the tests below say so.

## Equipment and probing rules

- Two oscilloscopes, 10x passive probes with the **ground spring** on the
  tip. Not the long ground clip: at 252 kHz with fast edges its loop
  picks up more than the signal being measured.
- **Null check at every location**: tip and ground spring on the same
  GND point. What shows there is pickup, and is the floor of that
  measurement.
- **One scope per ground domain.** The link domain (LINK_GND, CAN_P,
  CAN_N, AUX_12V) is isolated from bus GND. A scope ground on LINK_GND
  and another ground of the same scope on bus GND shorts the isolation
  through the chassis. Scope 1 stays on bus GND; scope 2 goes on
  LINK_GND when the link is measured. A mains-earthed scope on LINK_GND
  earths the link while it is connected: fine for the measurement, not a
  state to leave the row in.
- Optional: a current probe (C201, supply cable), a thermocouple or IR
  thermometer, a shorted probe loop as a near-field sniffer.
- Load: both slots with a module at full load (13.2 A at 19 V each)
  unless a step says otherwise. Note which module variant is fitted
  (-B: 252 kHz single phase; rev A: 200 kHz per phase, 400 kHz ripple).

## Test 1: ripple on the bus

Steady state, both slots at full load, scope AC coupled, 20 MHz
bandwidth limit on, then off.

| Point | Where to probe |
|---|---|
| Module input, slot A / B | across C2 / C52 (B side, between the S1 and S2 standoffs) |
| VBUS plane | across C214 or C215 (B side, strip, bottom) |
| XT60 | across C202 (B side, next to C201) |
| Supply | at the supply's output terminals |

- Record p-p and the 252 kHz amplitude (FFT or cursor) at each point,
  with and without the bandwidth limit.
- The two modules are not synchronised, so the ripple beats. Use a long
  timebase or persistence and record the **envelope maximum**.
- Repeat with one slot loaded, and at half load.

Expected: about 0.8 V p-p at the first three points with the limit on.
Much more than that, or spikes of volts with the limit off, means the
module is noisier than its document says and the rest of this plan
should be read against the measured value, not the predicted one.

## Test 2: C201

- Both slots at full load for 30 minutes **in air** (worse than in the
  fluid). Measure the can temperature against the board beside it.
- With a current probe: measure the ripple current directly and compare
  with the datasheet's 100 kHz rating (to be found; see background 2).

A rise of more than about 10 C over the board says C201 carries too much
ripple current for a general-purpose part. That is a lifetime question,
not an EMI one, and the answer is a low-impedance or polymer-hybrid part
on the same footprint.

## Test 3: logic nodes

All probed against GND right at the part, scope 1.

| Net | Where | Steady-state limit |
|---|---|---|
| PRESS_EN | Q10 pin 1 (gate), or R31 | under 0.2 V peak of 252 kHz |
| B_OVK (slot B latch /PRE) | U53 pin 7 | under 50 mV |
| B_PGD (slot B arm latch /CLR) | U60 pin 6 | under 50 mV |
| A_OVK, A_PGD for comparison | U3 pin 7, U10 pin 6 | under 50 mV |

Slot B's lines are the long ones (B_OVK: 72 mm on B over VBUS, 23 vias),
so slot B is the one to watch.

Then transients, both scopes on bus GND, single shot: scope 1 triggers
on the VBUS plane (C214), scope 2 watches one latch input. For each
event record the VBUS step, its edge time, and the excursion on the
latch input:

1. Hot-plug the XT60 with both slots off, then with one slot running.
2. Enable slot A while slot B runs at full load, and the reverse.
3. Node load step on one slot (idle to full, full to idle).
4. A forced trip of one slot (overcurrent or OV_KILL) while the other
   runs.
5. Unit hot-swap off and on (U_OFF) with both modules fitted.

A latch input must stay above 2.0 V (it is /PRE or /CLR, active low,
74LVC1G74 at 3.3 V) through every event that is not supposed to trip it.

Functional pass: repeat each event a few dozen times and confirm the
slot that was not touched never trips, and that the slot latch and the
arm latch read back unchanged through the expander.

## Test 4: CAN link

Scope 2, ground on LINK_GND (J303 / J304 pins 3 / 4, or C301 pin 2).

- CAN_P and CAN_N at JP301 (pins 1 and 3), channel math for the
  difference. Compare modules off with both slots at full load: edges,
  dominant level, noise in the recessive state.
- Separately, scope 1 grounded on bus GND, probe on LINK_GND: the 252 kHz
  common-mode voltage across the isolators. There is no limit to meet
  here beyond the ISO1044's ratings; the number is recorded to know it.
- **The test that decides**: the firmware's own counters (STATUS 0x1D:
  TEC, REC, bus-off count, reassembly drops) on a full row at full load
  for a few hours. They stay at zero.

## Test 5: what else the ripple reaches

- INA237 readings (U2, U52, U208) at a steady load: the current reading
  must not wander with load or with the beat between the two modules
  (ripple aliasing into the conversion).
- Near field, crude: a shorted probe loop held against the 19 V node
  cables and the 48 V cable, modules on against off. The 19 V cables are
  probably the largest radiated source in the system; that is a module
  question and this plan only notes the level.

## Test 6: short on one slot

Not an EMI test; it is here because it uses the same setup. Background:
docs/adr/0005-chained-plane-boards.md decision 3a. **With the FETs of FD
batch 1 (BSC070N10NS5, BSC040N10NS5) this test is expected to destroy
the slot FET and possibly the unit FETs**: run it on a board that may be
lost, or after the swap to ISC035N10NM5LF2. Supply current limit set, a
fuse or breaker in the 48 V lead, eye protection.

Scope 1 on bus GND, single shot, trigger on slot A's output falling:
VBUS (C214), A_OUT (C2), U_PGD (R238) and B_OUT (C52), or the same in
two shots.

1. Soft short: an electronic load or a resistor of about 2 ohm switched
   onto slot A's VIN standoffs with slot B at full load. Expected: slot
   A limits, folds back and switches off after about 10 ms, then
   retries about every 2 s; VBUS, B_OUT and U_PGD do not move.
2. Dead short: a relay or a FET of a few mohm across slot A's VIN
   standoffs, slot B at full load. Record the VBUS dip and its length,
   and whether U_PGD falls.
3. The same with slot B idle (no module, or module off): the bus has
   only 1.8 uF then and the unit breaker is expected to trip.
4. After each: slot A's FET still switches off (A_OUT at 0 V with the
   slot commanded off) and Q202 / Q203 still isolate (U_OFF).

Pass: slot B's node keeps running through 1 and 2, and no FET is left
conducting in 4. If U_PGD falls in 2: more VBUS capacitance or a series
inductance per slot (the fallback of tests 1 and 2), then repeat.

## Verdict

The boards are usable as built when all of these hold:

- [ ] Ripple at the XT60 is near the predicted 0.8 V p-p and the supply
      and the other units on it tolerate the sum (test 1).
- [ ] C201 stays within about 10 C of the board, or is within its
      100 kHz ripple rating (test 2).
- [ ] No latch input leaves its valid level and no untouched slot trips
      across the transient set (test 3).
- [ ] CAN error counters stay at zero over hours at full load (test 4).
- [ ] INA237 readings are steady (test 5).

Separate from the EMI verdict: test 6 decides whether a short on one
slot stays on that slot. The boards run without it; they do not protect
themselves against a short until the FETs are changed.

If test 1 or 2 fails: the choices are a different C201, or a series
inductor per slot (unit or module side), in that order of cost. If test
3 fails: VBUS-to-GND ceramics beside each slot's VBUS via ladder (B
side, x 108.5 and 133.5, y 110 to 117; fit not checked) and a look at
slot B's routing. If test 4 fails: revisit the pair's reference (ADR
0006 decision 12).

## Results

To be filled in per test: date, module variant, supply, cable length,
scope settings, numbers, screenshots.
