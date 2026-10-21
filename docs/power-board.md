# Power distribution board - component spec

Status: draft, 2026-10-07, measurement and knowns/unknowns added
2026-10-21. Not started in hardware. Open decisions and unknowns are
listed at the end; everything else is a requirement.

## Purpose

Switch and meter the 19 V supply of the 8 motherboards (nodes) in one
frame, under control of the granite controller. This gives a real
remote power-cycle (cut and restore DC), on top of the front-panel
RESET/PWR contacts the controller already has, plus per-node power
measurement.

## Context

```
 PSU A 3 kW 19V --+                        +--> node 1 (19 V, 10.5 A)
                  +==> [ power board ] ====+--> ...
 PSU B 3 kW 19V --+     8 x hot-swap       +--> node 8
                        + monitoring
                             ^
                             | I2C1, EXP_INT, 3V3 ref, GND (from J8)
                     [ granite controller ] --(Ethernet)--> network
```

- One 8-channel board per frame.
- Two 3 kW 19 V supplies feed one common bus. Either supply alone
  carries the full load (8 x 200 W = 1.6 kW), so the pair gives
  redundancy, not extra capacity.
- The controller runs from the same 19 V bus (its J3), tapped upstream of
  the hot-swap channels with its own fuse, so no channel fault or
  channel disable can take the controller down.

## Requirements

### Input

| Item | Requirement |
|---|---|
| Nominal voltage | 19 V DC |
| Operating range | 17-24 V (covers 19-20 V supplies and droop) |
| Abs max / transients | survive 30 V transients on the bus (TVS at input) |
| Continuous current | 85 A (8 x 10.5 A), design for 100 A |
| Available fault current | up to ~320 A from both supplies |
| Connection | bolted lugs (M6 or larger) or busbar, sized for 100 A |
| Main input fuse | 100-125 A, DC rated, on or next to the board |
| Reverse polarity | prevent by mechanics (keyed / asymmetric lugs) |

### Per output channel (x8)

| Item | Requirement |
|---|---|
| Switched rail | +19 V high side; GND is common and never switched |
| Continuous current | 10.5 A (200 W), design for 15 A |
| Current limit | adjustable, default ~14 A |
| Short-circuit response | primary: hot-swap controller, cut-off within tens of microseconds |
| Backup protection | ~20 A fuse per channel (covers a MOSFET that fails short) |
| Inrush | controlled ramp (soft start); no relay contacts |
| Path resistance | <= 10 mOhm total (sense R + MOSFET + fuse + copper), <= 1.1 W loss per channel at 10.5 A |
| Fault behaviour | auto-retry after an overcurrent / FET fault (decided); the controller logs every fault and may disable a channel that keeps retrying |
| Output connector | rated >= 15 A, keyed; type depends on the node input (open decision) |

### Monitoring

| Item | Requirement |
|---|---|
| Per channel | voltage, current, power; energy accumulation preferred |
| Accuracy | current and power within 2 % of reading at 10 A |
| Update rate | readable at >= 1 Hz per channel |
| Status | power-good, fault (overcurrent, short, overtemperature) per channel |
| PSUs | if the supplies speak PMBus, expose their status on the same bus |

### Control and failure behaviour

These are the critical requirements.

1. Default ON. A channel is ON unless the controller actively turns it
   OFF. Every undriven state counts as ON: board logic in reset, I2C dead,
   controller absent, rebooting or unpowered, or the cable unplugged.
1. A channel only goes OFF on an explicit command (register write or
   driven signal), never because a signal disappeared.
2. Channel OFF state survives a controller reboot. Releasing it requires
   a new command, so a power-cycle in progress is not cut short or
   repeated by a reboot.
3. The board logic is powered from the 19 V bus (local 3.3 V regulator),
   not from the controller, so requirements 1-3 hold with the
   controller disconnected.
4. EXP_nRESET from the controller must NOT reset this board's channel
   state. On the controller it resets the relay-drive expander on every
   MCU reset; here that would violate requirement 3. The board does not
   use EXP_nRESET.

### Interface to the controller

- Physical: cable to the controller's J8 expansion header (2x6, 2.54 mm),
  max 0.5 m. Used pins: external I2C SCL/SDA (J8 5/6), EXP_INT (J8 8),
  GND. 3V3 from J8 only as the I2C pull-up / level reference; not for
  board logic. Whether J8 I2C is the right link at all is open (see
  "Is I2C sufficient?" below).
- I2C: 3.3 V levels, 100 or 400 kHz.
- Rev C splits I2C in two buses. The on-board parts (U14 0x20, U15
  0x21, U5 TMP1075 0x48) are on the internal bus and do not collide.
  The external bus (J8, J9) carries add-on MCP23017 modules at
  0x21-0x27 and the LTC4282s at 0x40-0x47, plus whatever Qwiic sensor
  is plugged into J9.
- EXP_INT: the board pulls it low (open drain) on any channel fault
  or power-good change. Shared with other modules.
- Grounds: board GND (the 19 V bus return) connects to controller GND
  through the cable. The controller's Ethernet side stays isolated by
  its magjack; node front-panel contacts stay floating (photoMOS).

### PCB and mechanics

- Input to channel taps: busbar bolted to the board, or >= 3 oz copper
  with enough cross-section for 100 A at <= 20 C rise.
- Per-channel paths: >= 2 oz copper, widths for 15 A at <= 20 C rise.
- Losses: ~1 W per channel plus input path, about 10-12 W total.
  Natural convection with the frame's airflow; MOSFET and sense
  resistor on copper pours. No fan on the board.
- No creepage/clearance requirements beyond normal practice (19 V is SELV).
- Mounting holes and outline: to be fixed with the frame layout.
- Fab: PCBWay, heavy copper option.

### Controller firmware behaviour (requirements on the other side)

- Power-cycle: OFF, wait (default 10 s), ON, then press PWR via the
  existing photoMOS if the node does not auto-start (BIOS "power on
  after AC loss" not set).
- Power-up after a frame power loss: stagger channel ON with a delay
  (default 0.5 s) only if the board supports a startup hold; otherwise
  hardware default-ON applies.
- Report per-node V/I/P/energy and fault state through the controller's
  API; log faults.
- Count retries per channel; after a configurable number of retries in
  a time window, turn the channel OFF and raise an alarm (operator
  re-enables).

## Verification

- Inrush: switch a real node on; ramp and peak within the limit.
- Short circuit on one output with both supplies on the bus: that
  channel trips, the others and the bus stay up, nothing beyond the
  channel fuse is damaged.
- Thermal: all 8 channels at 10.5 A for 1 h; MOSFET, sense R, fuse
  and connector temperatures logged.
- Controller loss: reboot the controller, pull its 19 V feed, unplug the
  cable; no node loses power. Repeat during a power-cycle in progress.
- Monitoring accuracy against a bench meter at 1, 5 and 10 A.
- I2C robustness over the cable with all channels loaded.

## Selected parts

- Hot-swap controller: Analog Devices LTC4282, one per channel
  (decided 2026-10-07). 2.9-33 V, 16-bit V/I/P plus energy
  accumulation (+/-0.7 % total error), MOSFET power foldback for SOA,
  fault log, configuration in internal EEPROM, 5x5 QFN-32.
  Fallback if a check below fails: TI LM5066I (10-80 V, V/I/P/energy,
  SOA power limit, lower accuracy, HTSSOP-28).
  Order code LTC4282AUH#PBF (-40 to 85 C), about $10-12 each
  (distributor snippet, unconfirmed). Datasheet Rev. C checked:
  - Addresses: ADR0-2 are 3-state, 27 addresses in 0x40-0x5A. Use
    0x40-0x47 (pin table: datasheet Table 1). Keep in mind the
    mass-write address 0x5F
    (disable via CONTROL bit 4) and SMBus alert response 0x0C.
  - Default ON: FET_ON is loaded from EEPROM at power-up (default 1),
    so the FET turns on with no I2C traffic once UV/OV are valid and
    ON is high. Tie ON to VDD through a divider. Host OFF = clear
    CONTROL bit 3 (RAM only): survives a controller reboot, and an
    LTC4282 power loss reloads EEPROM and turns the channel back ON.
    Meets the failure-behaviour requirements. Never program EEPROM
    FET_ON = 0.
  - UV/OV: external dividers required (internal windows do not fit
    17-24 V). UV trips below ~16 V, OV above ~27 V (to be computed).
  - Faults: factory default is latch-off for overcurrent / FET-bad.
    This design uses auto-retry (EEPROM CONTROL bit 2 = 1): retry after
    256 TIMER cycles at a 1:1140 duty cycle. MOSFET SOA must survive
    retrying into a hard short indefinitely.
  - Abs max: VDD 45 V, operating 2.9-33 V; 30 V transients are fine.
    Input TVS per datasheet suggestion (SMCJ). MOSFETs rated >= 60 V.
  - Current limit: 12.5-34.4 mV in 8 steps. 2.0 mOhm sense resistor
    with ILIM code 101 (28.1 mV) gives ~14 A; 0.22 W in the resistor
    at 10.5 A. Foldback mode set for 24 V. Single MOSFET with SENSE2-
    grounded; SOA via TIMER capacitor (64 ms/uF) and power foldback.
    The datasheet only shows 12 V designs: MOSFET SOA at 24 V still
    to be checked when choosing the MOSFET.
  - Monitoring: 12/16-bit ADC, V/I +/-0.9 %, power +/-1.0 %; energy
    +/-5.1 % on the internal clock, +/-1.0 % with an external crystal
    or clock - use an external clock for usable kWh figures. 48-bit
    energy accumulator. Must be set to 24 V range (VIN_MODE = 11),
    the 12 V default clips at 16.6 V.
  - Production: factory EEPROM defaults are wrong for this design.
    Each part needs a one-time I2C write before first use:
    EE_CONTROL (24 V VIN_MODE, chosen fault mode) and EE_ILIM_ADJUST
    (current limit code, 24 V foldback). Decided: the controller
    firmware does this over I2C (see "EEPROM provisioning" below).

## EEPROM provisioning (controller firmware)

On every controller boot, for each LTC4282 at 0x40-0x47:

1. Read the EEPROM registers (EE_CONTROL, EE_ILIM_ADJUST).
2. Compare with the expected image: VIN_MODE 24 V, ILIM code,
   FOLDBACK_MODE 24 V, overcurrent auto-retry = 1, FET_ON = 1.
3. Match: do nothing (normal case).
4. Mismatch: write the image, wait >= 2 ms per write, read back.
5. Apply the same settings to the RAM registers so they take effect
   now, preserving the channel's current FET_ON state (never switch a
   node off as a side effect).
6. Log the result (ok / programmed / verify failed); a verify failure
   raises an alarm and marks the channel untrusted.

Rules:
- The write routine hard-codes EEPROM FET_ON = 1 and checks it on
  readback; a stored 0 would break default-ON.
- Disable mass-write (0x5F) at boot.
- WP pin: pulled low by a resistor, with a jumper (or expander GPIO)
  to lock the EEPROM after commissioning.
- Only write on mismatch; EEPROM endurance is 10k cycles.

## Power measurement and recording

### What is measured

Per channel, from the LTC4282 (register map to be checked against the
datasheet when the firmware is written):

- Source voltage, sense current, power (16-bit ADC).
- Min / max of voltage, current and power since the last clear. Read
  and clear on every poll, so a 1 s poll still catches inrush peaks and
  short spikes.
- Energy: 48-bit accumulator plus a tick counter, integrated in the
  chip continuously. The poll rate does not affect energy accuracy and a
  missed poll loses no energy; only the time resolution of the record
  suffers.
- Fault and status bits (power good, overcurrent, FET bad, UV/OV).

Energy accuracy is +/-5.1 % on the internal clock and +/-1.0 % with an
external clock (see "Selected parts"). Whether one shared oscillator
may drive all 8 CLKIN pins is to be checked.

The energy counter restarts when the LTC4282 loses power or is
cleared. Consumers treat it as a counter (monotonic, with resets), never
as an absolute meter reading.

### Data volume

Per channel per poll about 30 bytes of registers, about 40 bytes on the
wire with addressing and repeated starts.

| Link | Per frame (8 ch) | At 1 Hz |
|---|---|---|
| I2C 100 kHz | ~30 ms | ~3 % of the bus |
| I2C, software on GPIO6/7 (~50 kHz) | ~60 ms | ~6 % |
| RS-485 Modbus RTU 115200, ~16 regs per ch | ~25 ms | 9 frames on one bus: ~25 % |

Bandwidth is not the constraint on any of these links.

### Where the data lives

On the controller: only a short window. The ESP32-C6 has no PSRAM, the
8 MB flash is fully partitioned (two 2.5 MB OTA slots, ~190 KB unused),
and a time series written to flash every few seconds wears it out. So:

- Poll every channel at 1 Hz.
- Every publish interval (default 10 s, a config value) publish one
  message per frame on MQTT `.../power`: per channel V, I, P average
  over the window, P min/max of the window, the energy counter (as
  joules, u64), the tick count, status bits. Timestamps are wall clock
  from SNTP (already working, from DHCP option 42).
- Faults go out immediately as events, not in the 10 s message.
- Keep the last ~10 minutes in a RAM ring (60 x 8 x ~16 B, ~8 KB), so
  a short broker outage is bridged like the held events in mqtt.rs, and
  the HTTPS page can draw a short graph.
- Expose the latest values as Modbus input registers too, for a BMS or
  SCADA that wants to poll.
- Energy needs no on-device persistence: the counter lives in the
  LTC4282 and survives a controller reboot.

With a 16 MB module (planned for production, after testing) about 8 MB
is free for a data partition, and a local history becomes possible: a
fixed-size ring, written as a log with wear levelling. 1 min averages
of 8 channels for 30 days is 43200 x 8 x ~16 B, about 5.5 MB. At
~1 MB written per day the partition is rewritten about once a week, far
inside the flash's ~100k erase cycles. It would let a frame without a
backend show and export a month of history, and refill the backend
after a long outage. It is a buffer, not the record: the backend store
stays the source of truth.

Off the controller: a collector subscribes to MQTT and writes a store.
Two candidates, to be decided with the fleet backend ADR:

- RRD (rrdtool, fed by collectd's MQTT plugin or a small script).
  Fixed size, consolidation built in, energy as a DERIVE data source
  (rrdtool turns the counter into watts and handles resets). One file per
  node, DS = V, I, P, E; RRAs with AVERAGE, MIN and MAX at 10 s for
  2 days, 1 min for 30 days, 1 h for 2 years, 1 day for 10 years:
  about 8 MB per node, 63 MB per frame, 560 MB for a 9-frame pod.
  Good for one frame or a bench, no daemon, nothing to grow. Weak for a
  fleet: no labels, one file per series, queries across nodes and pods
  are awkward, and a late or out-of-order sample is dropped.
- A time-series database with labels (VictoriaMetrics, Prometheus,
  InfluxDB), fed from MQTT by Telegraf or the backend itself. Labels
  pod / frame / node, queries across the fleet, about 1 byte per
  sample: 72 nodes x 5 series at 10 s is about 1.1 GB per year without
  downsampling. Needs a running service and a retention policy.

Leaning: TSDB as the store for the fleet; RRD only if a site runs one
frame without a backend. Both work from the same MQTT message, so the
controller side does not depend on this choice.

### Is I2C sufficient?

For bandwidth, yes, by a wide margin (table above). The doubts are
elsewhere:

- Ground: up to 85 A returns on the common GND. The I2C cable's ground
  shares that path, so the ground offset between the power board and
  the controller moves with load. I2C is single-ended with a ~0.3 V
  noise margin at 3.3 V.
- Shared bus: J9 carries whatever Qwiic sensor is plugged in. One
  sensor holding SDA low stops all power monitoring (not the power
  itself: channels stay ON).
- Reach: 0.5 m and one frame. It does not reach the other frames of a
  pod.
- Software I2C on the controller (GPIO6/7) shares the CPU with
  everything else; EEPROM writes and bus recovery are untested.

Options, cheapest first:

1. Plain I2C on J8, with an I2C isolator (e.g. ISO1540) on the power
   board, and the power board alone on its bus segment (a switch or a
   second bus, not shared with J9). One frame only.
2. Differential I2C (e.g. PCA9615 over twisted pair) for a longer,
   noisier cable. Still one controller per frame.
3. Power duck: an MCU on the power board talks I2C to the LTC4282s
   over a few cm of trace, keeps the retry counting and EEPROM checks
   local, and talks RS-485 / Modbus RTU to the controller. Isolated,
   multi-drop, metres of cable, one controller for a pod. Fits the pod
   architecture; costs an MCU, firmware and an address scheme.

## Knowns and unknowns

Snapshot 2026-10-21, for an 8-node power expansion. The interface and
connectors are not fixed.

### Known knowns

- Load: 8 Strix Halo nodes at 19 V, ~200 W (10.5 A) each, ~1.6 kW per
  frame. Design for 15 A per channel and 100 A total.
- Supply: two 3 kW 19 V supplies on one bus; either carries the frame.
- Purpose: real remote power-cycle (cut DC) plus per-node V/I/P/energy.
- Default ON: OFF only on an explicit command, survives a controller
  reboot, board logic powered from the 19 V bus.
- The controller is never downstream of a channel.
- LTC4282 meets the default-ON requirement (FET_ON from EEPROM, OFF
  in RAM only, auto-retry, 8 free addresses at 0x40-0x47).
- The controller already has J8 (2x6: external I2C, EXP_INT on GPIO0,
  EXP_nRESET, 3V3, 5V), photoMOS PWR/RST per node, power LED sense per
  node, MQTT, Modbus TCP and HTTPS, SNTP wall clock.
- The boards are immersed; 19 V is SELV.

### Known unknowns

Interface and architecture:
- I2C on J8, power duck on RS-485, or both (see "Is I2C sufficient?").
- Frames per controller.
- Where the power-cycle sequence, retry counting, start-up stagger
  and EEPROM checks run: controller or board.
- Connector and cable to the controller; hot-plug safe or not.

Power path:
- PSU model: current sharing or OR-ing needed (at 100 A an ideal-diode
  OR-ing stage is real heat), PMBus or not.
- Node power input connector and cable (a barrel jack cannot carry
  10.5 A).
- Node current profile: inrush, input capacitance, peak versus average,
  boost spikes. Sets the current limit and the MOSFET SOA.
- MOSFET SOA retrying into a hard short at 24 V, indefinitely (the
  datasheet only shows 12 V designs).
- Main fuse location and type; per-channel fuse type.
- Frame cold start: all 8 nodes start together (the LTC4282 has no
  start-up hold). Can the supplies take the combined inrush?

Environment:
- Power board in the fluid or outside it: cooling, material
  compatibility, fuses, connectors.
- Board outline, mounting, cable and busbar routing.

Requirements:
- Energy figures for observation or for billing (decides the external
  clock).
- Cost target per channel (the LTC4282 alone is ~$10-12).
- Data retention: how long, at what resolution, who reads it.

### Unknown knowns (assumed, never written down)

- Nodes power on by themselves after a DC cut (BIOS "power on after AC
  loss"). The photoMOS PWR press is the fallback; not yet checked on a
  real node.
- Nodes take 19 V DC directly, with no internal brick. To check on the
  actual board.
- Ground: the I2C cable shares the 85 A return (see above).
- Power LED sense becomes a cross-check: channel ON at 0 A with the LED
  lit, or OFF with the LED lit, means something is wrong. Not designed
  yet.
- A hard cut is acceptable. Repeated cuts risk the SSD and the file
  system; the policy should be soft shutdown via PWR first, cut only
  when the node hangs.
- Software I2C on GPIO6/7 is good enough. Fine for 8 x 1 Hz; EEPROM
  writes and bus recovery under load are untested.
- The auto-retry duty cycle (1:1140) is harmless per channel. All 8
  retrying together into a bus fault gives the supplies a hiccup
  pattern nobody has looked at.

### Unknown unknowns: how to surface them

1. One-channel prototype on a real node: inrush, current profile,
   short, retry, thermal, a cut during boot.
2. Fluid soak: solder mask, plastics, fuse elements, connectors,
   electrolytics, at temperature for weeks.
3. Full frame: 8 channels, both supplies, pull one supply at full
   load, short one channel with the others running.
4. EMI: 100 A switching next to the W5500 and the I2C or RS-485 bus.
5. Node behaviour after hundreds of hard cycles: BIOS, SSD, RTC.
6. A per-part FMEA: what happens when each part fails open, short or
   stuck.

Decide first: I2C on J8 versus a power duck, and in or out of the
fluid. Then the node's power input (connector, current profile, BIOS
setting).

## Open decisions

1. PSU model: whether the two supplies are designed to run in parallel
   (current share or OR-ing) and whether they have PMBus.
2. Node power input connector. A 5.5 x 2.5 mm barrel jack is not rated
   for 10.5 A; this decides the output connector and cabling.
3. Main input fuse location and type (on board vs. in the supply cabling).
4. Board outline, mounting and cable routing in the frame.
