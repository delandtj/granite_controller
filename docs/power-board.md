# Power distribution board - component spec

Status: draft, 2026-10-07. Not started in hardware. Open decisions are
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
                     [ granite controller ] --(PoE/Ethernet)--> network
```

- One 8-channel board per frame.
- Two 3 kW 19 V supplies feed one common bus. Either supply alone
  carries the full load (8 x 200 W = 1.6 kW), so the pair gives
  redundancy, not extra capacity.
- The controller stays PoE-powered and independent of this board.

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
| Fault behaviour | latch off after a fault, re-enable from the controller (see open decisions) |
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

- Physical: cable to the controller's J8 expansion header (2x20, 2.54 mm),
  max 0.5 m. Used pins: I2C1 SCL/SDA (J8 5/6), EXP_INT (J8 8), GND.
  3V3 from J8 only as the I2C pull-up / level reference; not for board logic.
- I2C: 3.3 V levels, 100 or 400 kHz.
- Addresses already taken on the controller bus: 0x20 (U14 MCP23017),
  0x48 (U5 TMP1075). Add-on MCP23017 modules may use 0x21-0x27.
  If the hot-swap controllers cannot take 8 free addresses, the board
  carries an I2C mux (e.g. TCA9548A) and exposes one address.
- EXP_INT: the board pulls it low (open drain) on any channel fault
  or power-good change. Shared with other modules.
- Grounds: board GND (the 19 V bus return) connects to controller GND
  through the cable. The controller's Ethernet side stays isolated by
  its PoE module; node front-panel contacts stay floating (photoMOS).

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
- Clear a latched fault only on an explicit operator command.

## Verification

- Inrush: switch a real node on; ramp and peak within the limit.
- Short circuit on one output with both supplies on the bus: that
  channel trips, the others and the bus stay up, nothing beyond the
  channel fuse is damaged.
- Thermal: all 8 channels at 10.5 A for 1 h; MOSFET, sense R, fuse
  and connector temperatures logged.
- Controller loss: reboot the controller, pull its PoE, unplug the
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
    0x40-0x47 (pin table: datasheet Table 1). Avoid 0x48 (TMP1075 on
    the controller) and keep in mind the mass-write address 0x5F
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
  - Faults: default overcurrent / FET-bad is latch-off until FAULT_LOG
    is cleared over I2C (or VDD UVLO). Auto-retry is an EEPROM option.
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
    (current limit code, 24 V foldback). Do it from the controller
    firmware on first boot (check and program if different), then
    leave WP low, or order pre-programmed parts from ADI with WP high.

## Open decisions

1. PSU model: whether the two supplies are designed to run in parallel
   (current share or OR-ing) and whether they have PMBus.
2. Node power input connector. A 5.5 x 2.5 mm barrel jack is not rated
   for 10.5 A; this decides the output connector and cabling.
3. Fault mode: latch-off (proposed, LTC4282 default) or auto-retry
   (EEPROM option, retries at a 1:1140 duty cycle).
4. Main input fuse location and type (on board vs. in the supply cabling).
5. Board outline, mounting and cable routing in the frame.
