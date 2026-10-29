# Plane core firmware

**Status**: Proposed
**Date**: 2026-10-29
**Updated**: 2026-10-29 (user answers: image integrity, stagger, latches decided; unit glitch state open; press deadline line asked from the hardware, docs/press-deadline-line.md)

Related: docs/adr/0001-48v-power-plane.md (safety rules, TLV message
format, data flow), docs/adr/0005-chained-plane-boards.md (core + unit
boards, chain, unit hot-swap), docs/plane-requirements.md (R14-R20a),
firmware/docs/adr/0001-firmware-architecture.md (controller firmware,
OTA probation ladder). Hardware: power/pp-48/tools/sch/blocks mcu.py,
mux.py, link.py, unit.py, uinput.py.

---

## Context

The chained plane (ADR 0005) is one core board and 1-4 unit boards of
two slots each. Only the core has an MCU, an STM32G070CBT6 (Cortex-M0+,
64 MHz, 128 KB flash in 2 KB pages, 36 KB RAM, 8 MHz HSE crystal). No
firmware exists for it yet. Everything it touches:

| Core pin | Net | Use |
|---|---|---|
| PB6 / PB7 | SCL1 / SDA1 | I2C slave to the controller, through the ISO1640 |
| PA8 | INT_OUT | high = assert EXP_INT (ISO7721, open drain on the controller side) |
| PB10 / PB11 | SCL2 / SDA2 | I2C master: TCA9548A 0x70, then one unit segment per channel |
| PB12 | MUX_RST | low = mux reset, all channels off |
| PC13-PC15 | ADR0-2 | straps, pulled up, closed = 0; slave address 0x30 + ADR[2:0] |
| PB0 | VIN_SNS | core 48 V input, 220k / 10k divider |
| PB8 | LED_RUN | heartbeat LED |
| PA13 / PA14, NRST | SWD | bench flashing and debug |

Per unit, on mux channel k (unit k + 1), every unit with the same
addresses:
- TCA6424A 0x22, 24 lines, all inputs after reset (/RESET only on a
  pull-up, not driven by the core). Per slot (A on P00-P07 / P10-P12,
  B on P14-P17 / P20-P26), outputs OFF, LCLR, RUNOFF, DISARM, PWRD,
  RSTD and inputs PGD, PG, LQ, LED, OVK. Unit: P13 U_OFF (out), P27
  U_PGD (in).
- INA238 0x40 / 0x41 (slot A / B shunt, ALERT sets the slot latch on a
  power limit the firmware programs), INA238 0x44 (unit input, polled).
- ADC121C027 0x51 / 0x52: module ID resistor per slot (open = no
  module).

The decisions this firmware lives under are already taken:
- No firmware in any safety chain. Every slot and the unit hot-swap
  default ON; the OV cut is a hardware latch; current limits are
  analog (ADR 0001 safety rules 1-6, R14-R15).
- The plane core owns node sequencing: press timing, power-cycle,
  retry and latch policy, stagger. The controller does policy, rules
  and the network (user, 2026-10-29).
- The plane is updated from the controller over the link with its own
  A/B bootloader that keeps the strapped address; ST's ROM bootloader
  is bench recovery only (user, 2026-10-29). Reason: every G070 in the
  ROM bootloader answers the same fixed I2C address, so two planes in
  it at once collide, and a plane stuck there blocks the others.
- Several planes per controller on one isolated I2C link (0x30-0x37).

Two things in the hardware shape the firmware more than the ADRs say:
1. **Expanders keep their outputs across a core reset.** Nothing resets
   a TCA6424A except loss of the unit's 3V3. A core crash in the middle
   of a PWR press leaves the photoMOS on. The rev C controller solved
   this with a hardware deadline (expander held in reset when no press
   runs); the plane has no such line.
2. **A unit can lose and regain 3V3 on its own** (chain cable, core
   input glitch). Its expander then comes back as all inputs: every
   slot ON, the unit hot-swap ON, latches as the hardware left them.

---

## Decision

1. **Rust, embassy, two images.** `plane-boot` (bootloader, embassy-boot)
   and `plane-fw` (application, embassy-stm32 executor), target
   thumbv6m-none-eabi, in the granite-plane firmware workspace. The
   plane's behaviour (models, sequencer, scan, TLV handling) lives in
   a hardware-free `no_std` crate, `plane-logic`, behind small device
   traits, so the same code runs on the host: unit tests, and the fake
   plane in granite-sim is the real plane logic over simulated devices.
   The wire format is the shared `granite-proto` crate (own spec).

2. **Flash layout** (2 KB pages, sizes to confirm after the first
   build):

   | Region | Size | Content |
   |---|---|---|
   | BOOT | 16 KB | plane-boot, flashed over SWD only, never in the field |
   | STATE | 2 KB | embassy-boot swap state |
   | CONFIG | 2 x 2 KB | plane config, two copies with a sequence number and CRC |
   | ACTIVE | 52 KB | running application |
   | DFU | 54 KB | update image (ACTIVE + 1 page, embassy-boot swap) |

3. **Start-up order** (the expander rule). Before anything else the
   application:
   1. starts the IWDG (~500 ms) and reads the reset cause;
   2. on every channel that has a unit, writes the expander's photoMOS
      outputs (PWRD, RSTD) released, before any read;
   3. then restores the intended state. A slot that was OFF goes
      back OFF (user 2026-10-29: "back off when off was set"):
      - The app keeps the intended per-slot OFF / RUNOFF / DISARM and
        U_OFF state in the TAMP backup registers (magic + CRC),
        rewritten on every change, not only before a commanded reset.
      - **Backup registers valid** (commanded reset, watchdog, any
        reset with VDD kept): adopt. The expanders held their outputs
        through the reset, so an OFF slot never came on; the app
        checks them against the backup registers and keeps them.
        Nothing switches. This replaces ADR 0001 rule 2 ("a watchdog
        reset returns the slot to ON") for slots that were OFF.
      - **Backup registers invalid** (core power loss): hardware
        defaults, every slot ON, and the flag "intended state lost"
        in STATUS. The controller then re-applies the OFF it last
        commanded (controller ADR 0004).

   A crash in a press therefore holds the button for at most the IWDG
   period plus the boot time to step 2, ~0.6 s, under the ~4 s a
   motherboard needs for a forced off.

4. **Discovery and the chain.** At boot and then every 2 s for absent
   channels: release MUX_RST, select channel k (0-3), probe 0x22. A
   unit is present when the expander ACKs; the INA238s and ADCs are
   then probed and their absence is a unit fault, not "no unit". A
   present unit that NAKs twice in a row is marked lost (event), its
   channel goes back on the 2 s re-scan. A unit that appears or comes
   back is configured from scratch (expander direction, INA238
   configuration and power limits) and then put back in its intended
   state: a slot that was OFF goes back OFF (user 2026-10-29), and an
   event "unit reset seen" is raised. While the unit had no 3V3 its
   slots were ON (hardware default); that cannot be avoided. A bus that stays
   stuck (SDA low) gets nine SCL clocks, then a MUX_RST pulse, then
   the channel is skipped until the next re-scan.

5. **Polling.** I2C2 at 100 kHz (cable lengths, ADR 0005 risks). One
   cycle every 50 ms (20 Hz) over every present unit: expander inputs
   (3 bytes), each slot's INA238 bus voltage, current, power and die
   temperature, DIAG_ALRT on change of ALERT state; the unit INA238
   at 2 Hz; ID ADCs at 1 Hz and on a PG / presence change. Budget at
   4 units ~25 ms of the 50 ms. The plane model keeps per slot V, I,
   P, min / max since last read, energy (integrated from P against the
   HSE time base; reported with a boot counter so a consumer sees the
   reset), temperature, module ID and class, status bits and the LED
   state (on / off / blinking with period, from the 20 Hz samples).

6. **Slot actions** (all on the plane, the controller sends one CTRL
   record and reads the outcome):

   | Action | Lines | Rules |
   |---|---|---|
   | enable / disable | OFF | OFF kept in RAM and in the backup registers; a CTRL record is the only way to set it |
   | press PWR / RST | PWRD / RSTD | 50-10000 ms, one press per slot at a time, released by a timer task and again at every boot |
   | clear OV latch | LCLR pulse 10 ms | command only; never automatic (OV is the fault that kills a node) |
   | module run off / on | DISARM, then RUNOFF; release RUNOFF, then DISARM | ordering enforced, never RUNOFF without DISARM |
   | power-cycle | OFF, wait, ON, optional PWR press if the LED stays dark | the ADR 0001 0x47 sequence, with abort |
   | unit reset | U_OFF pulse 100 ms | re-arms the latched-off LM5069-1 |
   | stagger on | OFF on every slot, release one by one | plane-level, delay from config |

   Every action is refused (ack code) when it conflicts with a
   running sequence on the same slot, and recorded as an event when it
   ends.

7. **Retry and latch policy, plane side.** Slot hot-swaps (LM5069-2)
   retry by themselves; the firmware counts PGD drops per slot in a
   window and, past the configured maximum, sets the slot OFF and
   raises an event (ADR 0001 0x42: max, window, action). The slot OV
   latch and the unit hot-swap latch are never cleared automatically
   in the first release; the controller decides.

8. **Controller link.** I2C1 slave at 0x30 + straps, ADR 0001
   transactions in granite-proto's TLV format. A request (block id,
   index) is answered from the plane model encoded on the spot (a few
   microseconds, clock stretching only as a fallback); a block is a
   consistent snapshot. Control writes are queued, executed by the
   sequencer, and acknowledged in STATUS. EXP_INT (INT_OUT high) is
   asserted while the EVENTS ring is non-empty; reading EVENTS clears
   what was read; an overflow leaves one overflow event. The plane
   does nothing when the controller goes away: intended state stays.
   Slot index on the wire: 2 x (unit - 1) + (A = 0, B = 1), 0-7.

9. **Firmware update over the link.**
   - Transfer: FW_BEGIN (size, CRC-32, version) erases DFU, FW_DATA
     records (offset, up to 128 bytes, per-record CRC-8 as every
     write) are buffered in RAM and written by the main task; STATUS
     reports busy / next offset, so the controller paces itself and
     never needs a long clock stretch. FW_COMMIT checks the CRC-32
     over DFU and marks the swap; a following CTRL reboot does it.
   - Swap: embassy-boot swaps ACTIVE and DFU with the IWDG fed.
     Expanders hold their outputs; the app saved its intended state
     first (decision 3, commanded reset).
   - Probation: the new image runs unconfirmed. The controller reads
     INFO (version), checks the plane works (units seen as before),
     and sends FW_CONFIRM, the app then marks the image booted. No
     confirm within 120 s, or a watchdog reset before it: the app
     resets and the bootloader swaps back. The same ladder as the
     controller's own OTA (firmware ADR 0001).
   - Integrity: the controller verifies the plane image's signature
     (shipped inside or beside its own signed OTA); the plane checks
     the CRC-32. The link is inside the tank and only the controller
     can write it.
   - Time: ~55 KB at 100 kHz with the TLV overhead, ~10 s per plane,
     planes one after another.

10. **Plane config** in the CONFIG pages: retry policy per slot,
    stagger delay, INA238 power limits per slot, the unit LED
    thresholds. The plane runs on it without a controller; the
    controller holds the desired config and writes it when the
    config hash in INFO differs (compare, write on mismatch, as the
    controller firmware ADR planned for the LTC4282).

11. **Diagnostics.** defmt over RTT on the bench (SWD). In the field:
    a LOG block (last N short records from a RAM ring) and counters
    in STATUS (I2C errors per channel, resets by cause, boot count).
    LED_RUN: 1 Hz heartbeat, fast blink while updating, double blink
    while unconfirmed.

---

## Architecture Overview

### Component Breakdown

1. **plane-logic** (`firmware/plane-logic`, no_std, host-testable)
   - Plane model (units, slots, events), sequencer (per-slot state
     machines and timers), discovery, poll scheduler, TLV request
     handler, config.
   - Device traits: `SegmentBus` (select channel, read / write a
     register at an address), `Clock`, `BackupRegs`, `ConfigStore`,
     `Int`. Drivers for TCA6424A, INA238, ADC121C027, TCA9548A are
     written against `SegmentBus` and live here too.
   - Depends on granite-proto.
2. **plane-fw** (`firmware/plane-fw`, thumbv6m)
   - embassy-stm32 glue: I2C2 master as `SegmentBus`, I2C1 slave link
     task, IWDG, TAMP backup registers, flash config pages, ADC for
     VIN_SNS, LED, INT_OUT, embassy-boot FirmwareUpdater for DFU.
   - Tasks: link (slave), bus (poll cycle and actions on the segment
     bus, the only owner of I2C2), sequencer (timers), watchdog / LED.
3. **plane-boot** (`firmware/plane-boot`, thumbv6m)
   - embassy-boot for STM32 with the IWDG fed during the swap. No
     link, no I2C. SWD only.
4. **granite-proto** (`firmware/granite-proto`, no_std; own spec)
   - TLV codec, tags, blocks, CRC-8, the update records.
5. **granite-sim fake plane** (in `firmware/granite-sim`)
   - plane-logic over simulated devices (scenario TOML: units present,
     modules, loads, faults) behind a simulated I2C slave, so the
     controller firmware is developed against the real plane logic.

### Data Flow / Interaction

```
 controller (ESP32)        plane core (STM32G070)                     units
 ---------------         -----------------------------------         -------------
 I2C master  --iso-->  I2C1 slave --> link task                         
 EXP_INT    <--iso--   INT_OUT  <---- event ring                         
                          |  read: encode block from model               
                          |  write: queue CTRL / FW records              
                          v                                              
                       plane-logic: model, sequencer, discovery          
                          ^                    |                         
                          | samples, acks      | expander writes         
                       bus task (I2C2, 20 Hz) --> TCA9548A ch 0..3 --> TCA6424A 0x22
                                                                     INA238 0x40/41/44
                                                                     ADC 0x51/52
```

---

## Alternatives Considered

### Controller drives the units directly, the core MCU as a thin bridge
- **The idea**: the core forwards raw register reads and writes; the
  controller's firmware runs sequences and policy.
- **Optimizes for**: one firmware with all the logic in it.
- **Sharpest tradeoff**: every press deadline and stagger timing then
  crosses the isolated link and the controller, so a controller crash
  or link loss mid-press holds a button. The rev C deadline problem
  moves onto a bus.
- **Bets on**: the link and controller being as reliable as the plane.
  The user decided the other way (plane owns sequencing).

### C with ST HAL / LL instead of Rust
- **The idea**: CubeMX project, ST HAL I2C slave, ST's bootloader
  examples.
- **Optimizes for**: the most worn path for an STM32 I2C slave.
- **Sharpest tradeoff**: no code shared with the controller (protocol,
  model, sim); the TLV codec gets written twice and drifts.
- **Bets on**: the protocol staying small and frozen.

### ST ROM bootloader over I2C for updates
- **The idea**: jump to system memory on command, flash with AN4221.
- **Optimizes for**: no bootloader of our own.
- **Sharpest tradeoff**: a fixed shared address on every plane (one
  plane at a time, a plane stuck there blocks the others) and no
  rollback. Decided against by the user.
- **Bets on**: one plane per link.

### Plane verifies a signature (Ed25519 in plane-boot)
- **The idea**: the bootloader refuses an unsigned image.
- **Optimizes for**: a compromised controller cannot flash a plane.
- **Sharpest tradeoff**: ~6-8 KB of bootloader and a key that can never
  change in the field (BOOT is SWD-only).
- **Bets on**: the controller being a weaker point than physical
  access to the tank. Kept as the nearest alternative (see review
  asks).

---

## Consequences

### Positive
- The plane keeps sequencing and policy through a controller reboot,
  a link loss and a controller update.
- Plane updates in the field without pulling a plane; a bad image rolls
  back on its own.
- The controller and the simulator speak to the same plane logic that
  runs on the board.

### Negative
- A third binary to build, sign, version and ship (inside the
  controller's update).
- 128 KB halves to ~52 KB for the application. embassy on an M0+ must
  stay lean (no defmt in release, no float formatting).
- A crash mid-press is bounded by the watchdog, not by hardware.

### Risks
- **embassy-stm32's I2C slave on the G0 (i2c v2)**: newer and less used
  than its master. Mitigation: a spike first (G070 Nucleo or the core
  board: a slave answering a fixed block to the ESP32 through an
  ISO1640); fallback a small interrupt-driven slave on the PAC.
- **Code size**: if the app passes ~48 KB, drop to a single 'copy'
  update (DFU written, bootloader copies, no swap back), which
  costs the rollback.
- **TAMP backup registers lost with VDD**: they are; a core power loss
  is a power-on reset and the hardware defaults are the right answer
  there.
- **I2C on the chain**: 4 cables to unit 4. The 2 s re-scan and the
  stuck-bus recovery keep one bad cable to one unit.

---

## What an Expert Would Ask

**Q: The core reboots during a firmware update while the operator had
slot 3 OFF. Does slot 3 come on?**
A: No. The update path is a commanded reset: the app writes the
intended state to the backup registers, the expanders hold their
outputs through the swap, and the new image adopts both after checking
them. The same holds for a watchdog reset and for a rollback (the
old image adopts from the backup registers like the new one). Only a
core power loss puts slots ON, and then the controller re-applies OFF.

**Q: What holds a PWR press if the firmware hangs with the IWDG still
fed (a stuck task, the watchdog fed from another)?**
A: The watchdog is fed only from the bus task, after a full poll cycle
in which the sequencer also ran. A hung sequencer stalls the cycle. A
press is also released by the bus task when its deadline has passed,
not only by the timer that set it. A hardware deadline through the chain
(PRESS_EN, docs/press-deadline-line.md) is asked from the hardware;
until it exists this is the only bound.

**Q: Unit 2's chain cable is pulled and replugged. What do the slots
on units 2-4 see?**
A: Units 2-4 lose 3V3: their expanders reset to inputs, so every slot
there goes ON (and a slot that was OFF powers up its node) for as long
as the cable is out. When the units come back the firmware turns the
OFF slots back OFF (decision 4). The plane cannot prevent the ON while the cable is out; that is
the hardware default by design. Events record it.

**Q: How long does it take to see a node's LED blink pattern, and can
a sleeping node (slow blink) be told from off?**
A: 20 Hz samples of the LED line; blinking is recognised after two
edges and reported with its period. A slow 0.5 Hz breathing LED needs
up to 4 s before it is called blinking rather than off; the LED
timeout of the power-cycle sequence must be longer than that.

**Q: Several planes on one link: what stops the controller from
updating two at once?**
A: Nothing on the plane needs to; each plane keeps its own address
throughout. The controller updates one at a time anyway to bound the
link load and to keep one failure at a time.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **State after a reset or a unit glitch.** Decided (user
  2026-10-29): a slot that was OFF goes back OFF; the plane restores
  it from the backup registers or its RAM, the controller after a
  core power loss. Alternative: leave the hardware defaults and
  report. Cost to change: one branch in start-up and discovery.
- **Image integrity.** Decided (user 2026-10-29): controller verifies
  the signature, plane checks CRC-32. Alternative: Ed25519 in
  plane-boot. Cost to change later: high, plane-boot is SWD-only.
- **Latches never cleared automatically.** Decided (user 2026-10-29):
  OV latch and unit hot-swap latch need a controller command.
  Alternative: N retries for the unit hot-swap. Cost: config fields.
- **Power-up stagger.** Decided (user 2026-10-29): dropped as a boot
  behaviour; each unit has its own input and its slots start before
  the core can act. The stagger stays as a command (stagger on).
- **Flash split.** 16 / 52 / 54 KB. Cost to change: a re-flash over SWD
  of every plane (bootloader and partition table move together).

### Known unknowns and how the plan absorbs them
- embassy-stm32 I2C slave on the G070: default embassy; pivot to a PAC
  slave if the spike shows lost bytes or no clock stretching.
- Application size: default A/B swap; pivot to copy-only above ~48 KB.
- ISO1640 with clock stretching at 100 kHz across several planes:
  default the response is ready before the read starts; measure
  stretch on the bench.
- Chain I2C with 4 cables: default 100 kHz; drop to 50 kHz per channel
  if unit 4 shows errors.

### The mechanical work
- Workspace members plane-logic, plane-boot, plane-fw (own target dir
  and memory.x per image), granite-proto.
- Drivers in plane-logic: TCA9548A, TCA6424A, INA238 (configuration,
  power limit with ALATCH, DIAG_ALRT), ADC121C027.
- Host tests: sequencer timing, start-up adoption, discovery and
  re-scan, TLV round trips.
- granite-sim: fake plane on plane-logic.
- Bench: SWD flash of plane-boot + plane-fw on the first core board,
  controller-side tool to push an update (reuse of the controller
  OTA host tool).

Review asks: answered 2026-10-29 (integrity, stagger, latches as
proposed); the state after a unit glitch stays open.

---

## Open Questions

**Architecture-changers**
- [x] Image integrity: controller-verified, CRC-32 on the plane.
- [ ] Press deadline line in the chain: the hardware session is
  moving the chain to 14 pins with both PRESS_EN (gates every unit's
  photoMOS LED return, deadline circuit on the core) and EXP_RSTN
  (bused expander reset, open drain from the core), 2026-10-29, not
  committed yet. Once in: the press path keeps PRESS_EN alive only
  while a press runs, the capability bit "press deadline hardware" is
  set, and EXP_RSTN becomes the CTRL 0x4D recovery action (ADR 0003).
  Core pins to take from mcu.py when it lands.

**Behavior definers**
- [x] State after a reset or a unit glitch: OFF slots go back OFF
  (user 2026-10-29).
- [x] Unit hot-swap and OV latches: controller command only.
- [x] Power-up stagger: dropped, command only.
- [x] After a rollback: adopt from the backup registers like any
  reset.
