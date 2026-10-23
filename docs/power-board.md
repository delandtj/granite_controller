# Power distribution board - component spec

Status: draft, 2026-10-07, measurement and knowns/unknowns added
2026-10-21, power plane (carrier with MCU) added 2026-10-23. Not
started in hardware. Open decisions and unknowns are
listed at the end; everything else is a requirement. Since 2026-10-21
the direction is a 48-54 V bus with per-load port modules (next
section); the 19 V design below is the reference it grew from.

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

## Direction: 48-54 V bus, common port front end, per-load port modules

Raised 2026-10-21, being thought through; not decided. The rest of this
spec still describes the 19 V reference design (LTC4282 on a 19 V bus).
Where this section conflicts with it, this section is the direction of
travel; the 19 V Strix Halo channel becomes the first port module.

### Why

- Loads differ. Strix Halo nodes take 19 V at ~200 W; other loads in a
  pod (storage servers, switches, GPU boxes) need other voltages and
  far more power. A 19 V bus forces every load onto 19 V. With a
  48-54 V bus the bus is the standard and each port decides what its
  load gets: the same split as the controller (generic core, per-load
  part).
- Current: one frame at 1.6 kW is 33 A at 48 V against 85 A at 19 V; a
  9-frame pod (~14.4 kW) is ~300 A against ~760 A. For the same copper
  the conduction loss drops by about 6x.
- 48 V class is the datacenter and telecom standard: OCP Open Rack V3
  busbars, telecom -48 V (float ~54 V on a 24-cell lead-acid string),
  54 V supplies for PoE switches, 2.5-3 kW 48-54 V supplies with
  current share and PMBus.
- Still SELV (< 60 V DC), but with only ~6 V of headroom at 54 V.

### Input range

Design for 40-60 V in, not for "48 V" or "54 V", so any 48 V class
supply, or a battery-backed bus, works. Parts on the bus side are
rated 80-100 V. Transient protection (TVS) must clamp inside the gap
between 57 V and the parts' ratings.

### Port = front end + module

```
40-60 V bus --> [ port front end ]        --> [ port module ]           --> load
                same on every port:            per load class:
                - hot-swap / eFuse 40-60 V     - 54->19 V 200 W (Strix Halo)
                - V/I/P/energy metering        - 54->12 V 300 W
                - default ON, OFF on command   - 54 V pass-through
                - fault auto-retry             - 5 V low power, ...
                - on the isolated I2C bus      - its own output connector
                                               - its own output OV cut
                                               - ID: type, Vout, Imax
```

Front end (identical on every port):
- Keeps every requirement of "Control and failure behaviour": default
  ON, OFF only on an explicit command, OFF survives a controller
  reboot, logic powered from the bus.
- Metering on the bus side counts load plus module loss. An optional
  monitor on the module output (e.g. INA228, 85 V) counts the load
  alone; with both, the module efficiency per port is a free health
  check.
- The LTC4282 does not fit (2.9-33 V). Candidates in the 48 V class
  were TI LM5066I, ADI ADM1272, ADI LTC4286 / LTC4287, checked for
  default-ON behaviour, OFF via pin, pin-and-resistor configuration,
  metering accuracy and price. DECIDED 2026-10-23: none of them; the
  front end is a TI LM5069-2 hot-swap controller plus a TI INA238
  monitor on one shared shunt. See "Front end part" under "Power
  plane".

Module (the only load-specific part):
- Converts (or passes through) and carries the load's output connector,
  so the output connector is specified per module, not per board.
- Output voltage and output overvoltage cut are fixed in the module's
  hardware. Software turns the port on and off and may lower the current
  limit within the module's maximum; it never sets the output voltage.
  A config mistake or a firmware bug must not be able to put 24 V on a
  12 V load.
- Output OV cut independent of the converter: a non-isolated buck whose
  high-side MOSFET fails short puts the full bus on the load (54 V onto
  a 19 V board). This is the top entry in the FMEA.
- Identifies itself (ID EEPROM, or a resistor read by an ADC): type,
  nominal Vout, Imax. The controller compares it with its port config
  (e.g. "port 3: expects 19 V, 250 W") and raises an alarm on a
  mismatch at commissioning.
- Pass-through modules serve loads that take 48-54 V themselves (OCP
  gear, many GPU servers): no conversion, no conversion loss.

### Current and power measurement: where it lives

Written out 2026-10-22 after the question "can I read power
consumption now?". Short answer: not yet. Nothing in the hardware that
exists today (module, bench carrier) meters anything; metering is a
function of the port front end on the 48 V power board, which is still
a spec.

How current is measured:
- Shunt: a few-milliohm resistor in the wire, the voltage across it is
  amplified and digitised (Ohm's law). The monitor chip must stand the
  wire's voltage: on the 54 V bus an 85 V class part (INA228, or the
  hot-swap controllers below); on the 19 V output almost anything.
- Hall sensor: measures the field around the wire. No resistor, no
  loss, isolated, but less accurate at small currents and drifts with
  temperature. Not the fit for 3.7 A per port; a shunt is.

Three places in this design:
1. Bus side, in the port front end (54 V, 3.7 A at 200 W). The
   hot-swap chip already has a shunt for its current limit; the
   telemetry versions (LM5066I, ADM1272, LTC4286/4287) read the same
   shunt and report V, I, P and accumulated energy over I2C. One chip
   is switch, fuse and meter. A 10 mOhm shunt costs 0.14 W. This is
   the planned, required measurement; it counts load plus module loss.
2. Module output (19 V, 10.5 A). Counts what the node really eats;
   with 1. the difference is the module's efficiency, a health number,
   not a requirement. The output is only reachable on the module
   itself (the module carries its own output connector), so an output
   measurement needs the module to report it, see the J2 decision.
3. Inside the module: the two 8 mOhm phase sense resistors are not
   usable for a readout. They see the chopped inductor current and
   feed the LTC7810's protection only.

What the front end delivers: instantaneous V, I, P; min and max since
the last read (a 1 s poll still catches peaks); an energy counter the
chip integrates on its own, so a missed poll loses nothing. About 1 %
accuracy is routine with a 1 % shunt; shunt self-heating at 10 A is
the usual enemy (2512 or bigger, low tempco). See "Power measurement
and recording" for the data path and volume.

Module output reporting, the one decision to take now: J2 is a 5-pin
header with no spare pin, cheap to change at rev 0.1 and expensive
later (every carrier and front end follows it). Options:
- None: bus-side metering alone gives per-node power. Nothing changes.
- Analog: a sense amplifier on the module (INA190 class, 2 mOhm shunt,
  about 0.2 W at 10.5 A) puts out a voltage proportional to output
  current on one extra J2 pin, read by the front end's ADC the same
  way as ID and TEMP. J2 becomes 6 pins. Cheapest, no address, no
  energy counter (the controller integrates).
- Digital: an INA228 on the module and SDA/SCL on J2. More accurate,
  energy on-chip, costs two pins and one I2C address per module. J2
  becomes 7 pins.
Open, listed under "Open decisions for this direction".

Reading power today, on the bench: the lab supply's V and I readout
for the input, the electronic load's V and I readout for the output,
V x I on each side; the carrier's Kelvin test points (TP1-TP4 at the
socket pins) are where a voltmeter goes so lead drops stay out of the
efficiency number. The carrier has no shunt and no monitor IC. If
numbers over I2C are wanted before the power board exists: an INA228
breakout (85 V) with its shunt spliced into the VIN lead, a second one
in the VOUT lead, on the controller's external I2C bus. A bench hack,
not part of the design; a carrier rev could carry the same two parts
plus a Qwiic header.

### Losses

A 54->19 V buck at 200 W is 96-98 % efficient: 4-8 W per port. That
is not negligible: 60 W around the clock is ~525 kWh per year per frame,
~4.7 MWh per year for a 9-frame pod, and all of it lands in the fluid as
extra cooling load. Compare whole chains, not boards. Rough figures per
frame at 1.6 kW (typical efficiencies, to be replaced with datasheet
values):

| Loss | 19 V bus | 54 V bus + per-port buck |
|---|---|---|
| AC -> DC supply | ~93 % (3 kW 19 V supplies are niche): ~120 W | ~96 % (54 V Titanium class): ~65 W |
| Bus, 2 mOhm path | 85 A: ~15 W | 33 A: ~2 W |
| Channel switch path | 8 x 1.1 W: ~9 W | ~1 W |
| 54 -> 19 V modules | none | 8 x 4-6 W: 32-50 W |
| Total | ~145 W | ~100-120 W |

If the supply figures hold, the 54 V chain breaks even or wins, mostly
on supply efficiency; across a pod the 19 V distribution loss grows
much faster. Ways to cut module loss further: design for 98 %
(GaN or low-RDS(on) FETs, low switching frequency with large inductors,
which the fluid cools, two phases per port); a fixed-ratio converter
(3:1, ~99 %, but unregulated: 16-18 V from 48-54 V, only if the load
accepts that range); or no conversion at all for loads that take
48-54 V.

### Immersion

The board sits in the fluid. 4-8 W per port is easy for the fluid to
carry off magnetics and FETs; no heatsinks or fans. What the fluid adds:
- Material compatibility of inductors (core coatings, glue), capacitors
  (ceramic and polymer only, no wet electrolytics), connectors and fuses:
  part of the fluid soak test.
- Breaking 48-54 V DC under load arcs, and an arc in dielectric fluid
  can carbonise it. Bus and port connectors are never unplugged under
  load; the port is switched OFF before a module or load is unplugged.

### Controller supply

The rev C controller cannot sit on a 54 V bus: J3 has a 22 V TVS
(SMAJ22A) and a 0.75 A PTC. A rev C board must never be connected to
the 54 V bus.

DECIDED 2026-10-23: every plane carries an always-on 12 V auxiliary
output for the controller: its own small 100 V buck from the bus
(LM5164 class, about 1 A), its own fuse, never switched, not a slot.
J3 takes 4.5-36 V through the LMR51430, so rev C runs on it as is;
12 V keeps the SMAJ22A out of any overshoot argument a 19 V rail
would have. With more than one plane per controller, feed the
controller from two planes through an ideal-diode OR on the
controller side, so one plane losing its bus, its aux fuse or being
pulled for service does not take the controller down.

Rejected the same day: feeding the controller from slot 1's brick.
The controller must never be downstream of a switched slot: cycling
node 1 would reboot the controller, a node 1 fault would drop it and
make it reboot in step with the front end's retry, and the slot's OV
cut, current limit and retry policy are sized for a 200 W motherboard,
not for the board that has to stay up through those events. A 40-60 V
controller input is dropped too; the plane feeds the controller.

### Interface

Galvanically isolated I2C on J8, isolator on the power board (plane),
EXP_INT across the barrier, unchanged. Behind the isolator sits one
MCU per plane since 2026-10-23 (see "Power plane"), not the front ends
themselves.

### Open decisions for this direction

1. Load classes in sight (Strix Halo nodes, storage servers, switches,
   GPU boxes, ...) and their rough power. Sets the power per slot.
2. DECIDED 2026-10-23: one slot kind, the eighth-brick slot at 15 A.
   Larger loads get their own plane type later (the controller is
   plane-type agnostic), not a second slot kind on this plane.
3. DECIDED 2026-10-23: 2, 4 and 8 slot planes of one design; 6 is a
   depopulated 8.
4. Module form: the module is a brick soldered into the plane (see
   "Mounting on the carrier"); the plane carries the slot's output
   connector. A plug-in receptacle only on the bench carrier.
5. Bus voltage target and supplies: confirm 48-54 V supply models,
   efficiency at 1.6-3 kW, current share, PMBus. Same for 19 V supplies,
   to settle the loss table with datasheet numbers.
6. Input range of each load: does the Strix Halo board need exactly
   19 V or accept a range (decides whether a fixed-ratio module is an
   option).
7. DECIDED 2026-10-23: LM5069-2 + INA238 on one shunt, see "Front end
   part" under "Power plane".
8. DECIDED 2026-10-23: 12 V always-on aux output on every plane, OR-ed
   on the controller when there are several planes. See "Controller
   supply".
9. DECIDED 2026-10-23: J2 grows to 7 pins with SDA/SCL reserved and
   unpopulated on rev 0.1; module output reporting is digital, later,
   through the plane MCU. See "Power plane". The bench carrier's 5-pin
   X2 mates pins 1-5 and stays.
10. Plane address range, connector and chain topology (proposal under
    "Power plane").

### Power plane: PP-48-xS (the carrier board)

Discussed and decided 2026-10-23. The board the bricks plug into is
called the power plane. Three sizes of one design, 2, 4 and 8 slots
(a 6-slot plane is an 8-slot plane with two slots unpopulated), all
identical to the controller. 220 V / mains modules were discussed and
dropped: the plane is 48 V only, and every slot is fed from the plane's
bus, so the plane itself meters every slot. A load that does not fit a
brick slot gets its own plane type later, not a second slot kind here.

Decisions:

1. One slot kind: the DOSA eighth-brick socket of PM-19V-200W plus its
   J2 aux row, a plane-side front end rated 15 A (the brick pins 4/8
   cap a module at about 10-12 A, 200-250 W), and the slot's output
   connector (type still open, >= 15 A, keyed). 8 slots x 250 W is
   2 kW: design the 8-slot plane's bus copper for 50 A (40 V in).
2. An MCU per plane. Supersedes "no MCU on the power board"
   (2026-10-21). That decision was taken for one board per frame; it
   falls with several planes per controller and with plane types that
   do not exist yet. The MCU is a telemetry aggregator and command
   relay only; the safety chain stays in hardware (rules below).
   Candidate STM32G0B1: two I2C peripherals (slave toward the
   controller, master toward the front ends), 16 ADC channels (ID and
   TEMP of 8 slots), FDCAN for a later pod-wide link, Rust with
   embassy-stm32. Powered from the bus through a 100 V buck (LM5164
   class) to 3.3 V, never from the controller cable.
3. The controller is plane-type agnostic: it enumerates planes by
   address, reads type, slot count and map version from the register
   map, and exposes the slots. Every plane type, now and later, speaks
   the same map; type-specific fields are additions, never changes.
   The controller firmware adapts to more planes and more types, it
   never learns a plane's internals.
4. Measurement: bus-side per slot, from the shunt of the front end.
   Switch and protection in one chip (LM5069-2), metering in a second
   chip on the same shunt (INA238), see "Front end part" below. The
   MCU polls the monitors at 10-20 Hz, keeps min/max since the last
   controller read and integrates energy against its own crystal, so
   the front end needs no energy register and no external clock.
   Module output measurement stays a health number (decision 9
   below).
5. Link to the controller unchanged in principle: isolated I2C, the
   isolator on the plane, its controller side powered from J8 3V3.
   What changes is that only the MCU sits behind the isolator.
6. Node control moves to the plane (2026-10-23). Each slot carries the
   node's front-panel signals next to its output connector: PWR and
   RST contacts through two photoMOS (TLP176AM as on the controller
   rev C), one power LED sense input (same 47k network as rev C).
   Driven by MCU GPIO or an MCP23017 on the plane's private bus. The
   slot is the node's single attachment: one cable per node from the
   plane, splitting at the node end into the DC input and the
   front-panel header. The signals are on the plane at the slot, NOT
   through the brick: the module is a converter and knows nothing
   about motherboards. The plane MCU executes the primitives (press
   with duration, LED read, power-cycle sequence with delay and
   press-if-dark); policy (soft shutdown first, hard cut only when the
   node hangs, retry limits) stays on the controller. LED versus
   current cross-checks (LED lit at 0 A, LED lit while OFF) are local
   flags. Consequence: the controller sheds its node section (8
   connectors, 16 photoMOS, U14/U15) at the next rev; rev C keeps
   serving as is until then. A dead plane MCU now takes a frame's
   control, buttons and LED sense included, with it; service is not
   affected (default ON). In the FMEA.
7. Controller supply: 12 V always-on aux output on every plane, see
   "Controller supply" (decision 8 there).

Safety rules for the MCU (the FMEA checks each):

- Every front end enable pin is pulled up to ON on the plane. The MCU
  pulls it low through an open-drain output to switch a slot OFF.
  MCU in reset, unpowered, or crashed reads as ON. OFF is never set
  through a PMBus OPERATION command: the pin default must be what
  rules.
- OFF lives in MCU RAM: it survives a controller reboot and a cable
  unplug. An MCU watchdog reset returns the slot to ON (same class of
  event as the accepted LTC4282 power-loss case). Restoring OFF from
  RTC backup registers after a reset is an option, not a requirement.
- The overvoltage cut is hardware: the module's OV_KILL sets a per-slot
  latch that holds the enable pin low. The MCU reads the latch and
  clears it on command; it does not implement it.
- Start-up stagger: after boot the MCU holds all but the first slot
  low and releases them in sequence (default 0.5 s). A dead MCU holds
  nothing, so everything comes on together: the hardware default, not
  a lockout. The hold starts within 50 ms of bus power so no slot
  flickers.
- Nothing on the plane depends on the controller cable: MCU, front
  ends and the isolator's plane side run from the bus.

Slot signal interface (J2 on the module, X2 on the plane):

| Pin | Signal | Direction | Note |
|---|---|---|---|
| 1 | OV_KILL | module -> plane | open drain, low = overvoltage, sets the hardware latch |
| 2 | PGOOD | module -> plane | open drain, high = good |
| 3 | ID | module -> plane | resistor to GND, plane ADC, value table to define |
| 4 | TEMP | module -> plane | 10k NTC to GND, plane ADC |
| 5 | GND | | |
| 6 | SDA | both | reserved, unpopulated on PM-19V-200W rev 0.1 |
| 7 | SCL | both | reserved, unpopulated on PM-19V-200W rev 0.1 |

Pins 6-7 close decision 9 as "digital, later": a module may carry an
output monitor (INA228 class, address pins set per slot by the plane
or behind a per-slot I2C switch) and the plane MCU reports module
output and efficiency. Nothing in rev 0.1 uses them; adding them now
costs two holes, adding them later costs every carrier. The 5-pin
X2 on the bench carrier mates the first 5 pins and stays as is.
ON/OFF stays on J1.2 (RUN), driven by the plane front end's enable
chain, not by J2.

Slot node connector (plane side, per slot, proposal): one housing for
power and front panel, Mini-Fit Jr 2x4 class (9-13 A per pin): two
pins +VOUT, two pins GND, PWR_SW, RST_SW, PLED+, one spare. The
switch returns are the shared GND (the brick output is not isolated,
the node's GND is the plane's). Fluid compatibility of the housing is
part of the soak test; the 5-wire node cable of rev C is replaced by
this one cable.

Plane connector to the controller (controller side of the isolator):
3V3, GND, SCL, SDA, EXP_INT. EXP_nRESET is not carried (the plane
must ignore it). Two or three address strap bits on the plane set the
plane address; proposal 0x30-0x37, 8 planes per controller bus, clear
of the add-on expanders (0x21-0x27). Topology: each plane has an in
and an out connector for a daisy chain, or a small star breakout at
J8; the isolators sit in parallel on the controller-side bus, a few
mA each from J8 3V3. 0.5 m total per the existing requirement. Next
to it the 12 V aux output (2-pin, fused on the plane) for the
controller's J3.

Register map, first sketch (16-bit registers, little endian, the map
version is the first thing a controller reads):

- Plane: map version, plane type (0x0001 = PP-48-xS), slot count,
  firmware version, bus voltage, plane temperature, MCU uptime, global
  status (any fault, any PGOOD change since last read, stagger active).
- Per slot: status (present, enabled, pgood, OV latched, front end
  fault, retrying), retry count, module class and ID code, V (mV),
  I (mA), P (W x 10), min/max P since last read, module TEMP (0.1 C),
  energy (J, u64), the optional module-output V/I when pins 6-7 are
  used, node LED state (on, off, blinking with period), cross-check
  flags (LED lit at 0 A, LED lit while OFF, no LED after power-cycle),
  power-cycle sequence state (idle, off-wait, on-wait, pressing, done,
  failed).
- Control: slot enable mask (write 0 = OFF, 1 = ON), clear OV latch
  mask, retry policy (max retries per window, window length, action),
  stagger delay, clear min/max; per slot: press PWR / press RST with
  duration (ms, 50-10000, long press = forced off), start power-cycle
  (off delay, press-if-dark flag, LED timeout), abort sequence.
- Plane: aux 12 V output voltage and current (if the aux buck's
  monitor is populated), aux fuse state.
- EXP_INT: pulled low on any fault, PGOOD change, OV latch, LED state
  change or sequence completion; released when the controller reads
  the global status.

Firmware (plane): Rust, embassy-stm32, I2C slave register map toward
the controller, I2C master toward the INA238 monitors. Update over SWD
on the bench; an I2C bootloader is open. No EEPROM anywhere in the
front end: the LM5069 is configured by resistors, the INA238 gets its
configuration register (ADC range, conversion time, averaging, shunt
calibration) written by the MCU at every boot. The LTC4282 EEPROM
provisioning routine below is history.

#### Front end part

Decided 2026-10-23 after checking the datasheets and LCSC stock the
same day. Requirement to meet: current and power within 2 % of
reading at 10 A (Monitoring table), useful down to 1-4 A, default ON
at a pin, OFF via a pin, auto-retry, configuration by resistors.

| Part | Range | Protection | Metering | Price, stock (LCSC, 2026-10-23) | Verdict |
|---|---|---|---|---|---|
| ADI ADM1272 | 16-80 V | hot-swap, PMBus | 12-bit, +/-0.85 % | last-time buy at ADI, 1 piece at LCSC | out: end of life |
| ADI LTC4286 | 8.5-80 V | hot-swap, PMBus | 12-bit, +/-1 %, no energy | not at LCSC, ~$19 single, $8.84 at 1k | out: price, sourcing |
| ADI LTC4287 | 6.5-80 V | hot-swap, PMBus | with energy | 48-QFN 7x7, ~$28 single, 30 week lead | out: price, size |
| TI LM5066I | 10-80 V | hot-swap, power limit, PMBus, 27 addresses | 12-bit at 1 kHz; current +/-3.5 % at 80 % of range, +/-6 % at 19 %; power +/-4.5 to 6.5 %; READ_EIN, peaks, averages | C1848356, $8.58 at 100, 129 in stock, HTSSOP-28 | out: metering misses 2 %, and PMBus addressing is moot behind the MCU |
| TI LM5069-2 | 9-80 V | hot-swap, current limit 55 mV (48.5-61.5), circuit breaker 105 mV, power limit (PWR resistor), UVLO/EN and OVLO pins, PGD, insertion timer, auto-retry at 0.5 % duty | none | C111822, $3.70 at 100, 7219 in stock, VSSOP-10 | CHOSEN for switch and protection |
| TI INA238 | -0.3 to 85 V common mode, bus 0-85 V | none | 16-bit; offset +/-5 uV max, gain +/-0.1 % max, 5 uV/LSB at +/-163.84 mV; 16 I2C addresses (A0/A1 to GND, VS, SDA, SCL); ALERT; die temperature | not at LCSC (TI 12 week lead); Mouser ~714 in stock, ~1.5-2 EUR; VSSOP-10 | CHOSEN for metering |
| TI INA228 | as INA238 | none | 20-bit, +/-1 uV, +/-0.05 %, energy and charge accumulators | C2887910, $6.08 at 100, 0 in stock | fallback: same footprint and pinout as INA238, drop-in if stock appears; its accumulators are not needed (MCU integrates) |
| TI INA237 | as INA238 | none | 16-bit, lower accuracy class | C2864837, $0.75 at 1k, 0 in stock | fallback for cost, same footprint |

Why two chips beat one: the integrated parts meter with the 12-bit
ADC that also serves their protection, so a percent of full scale, and
at a quarter of the range that is 6 % of reading. A dedicated monitor
meters at a percent of reading down to a few hundred mA. The pair is
also cheaper than the LM5066I alone, in stock, and nothing in it has
an address count or an EEPROM to manage.

Per slot, numbers for the 15 A class:
- Shunt 3.5 mOhm, 0.5 %, low tempco, 2512 2 W (or two 7 mOhm 2512 in
  parallel). Current limit 55 mV / 3.5 mOhm = 15.7 A (13.9-17.6 A over
  the threshold tolerance), circuit breaker 30 A. Loss 0.39 W at
  10.5 A, 0.8 W at 15 A.
- INA238 on the same shunt, Kelvin-connected, +/-163.84 mV range:
  LSB 1.4 mA, full scale 47 A. At 4 A (14 mV) the offset is 0.04 %
  and the gain error 0.1 % of reading; with the 0.5 % shunt the
  reading is inside 1 % from 1 A up. Energy by MCU integration of
  V x I at 10-20 Hz, conversion time 1 ms with averaging.
- VBUS of the INA238 on the slot side of the MOSFET (reads 0 when the
  slot is OFF, reads the voltage the brick actually gets); the bus
  voltage itself is one plane-level reading.
- LM5069-2: UVLO divider for ~38 V start, OVLO divider for ~62 V stop
  (the module's own windows), RPWR for the MOSFET's SOA at 54 V,
  TIMER for insertion delay and fault timeout, PGD to the MCU. The -2
  variant retries on its own at 0.5 % duty; the MCU counts retries
  from PGD and applies the retry policy by pulling UVLO/EN low.
- Enable chain on UVLO/EN: the divider holds it above threshold (ON);
  the MCU's open-drain output and the slot's OV latch each pull it
  below threshold (OFF). UVLO/EN is rated 100 V abs max, so no level
  concern; the pull-down only sinks the divider current.
- MOSFET: 100 V N-channel in a 5x6 package, chosen with the SOA check
  for power limiting at 54 V into a shorted output (open below).
- 16 INA238 addresses cover a plane of 8 slots on one MCU I2C bus
  with no mux; A0/A1 strapped per slot.

Sourcing: LM5069-2 from LCSC for JLC assembly; INA238 consigned or
bought at Mouser for the first planes (8 per plane). Verify LCSC
stock again at order time; INA228 drops into the same footprint.

Open for the plane:

- MOSFET for the LM5069 at 15 A: 100 V, SOA for power limiting at
  54 V into a short, RPWR and TIMER values, retrying indefinitely.
- Shunt part (3.5 mOhm 0.5 % 2512, tempco) and its self-heating.
- Per-slot OV latch part (dual flip-flop or discrete bistable), and
  how the latch interacts with the front end's own retry.
- Slot output connector (>= 15 A, keyed, fluid compatible).
- Plane input: lugs or connector for 50 A, main fuse, TVS.
- Plane outline and slot pitch from the brick outline plus the output
  connector; mounting in the frame.
- ID resistor table (module class and nominal Vout).
- STM32 part, I2C slave support in embassy-stm32 for the G0's I2C
  peripheral, bootloader path.
- Controller side: plane discovery on J8, exposing N planes x M slots
  in MQTT, Modbus and the HTTPS page; node control commands routed to
  the owning plane instead of the local photoMOS.
- Node connector part (Mini-Fit Jr 2x4 or equivalent, fluid
  compatible) and the node-end split cable; motherboard front-panel
  pinout (open item 4 of controller.md moves here).
- Aux 12 V buck part and its fuse; ideal-diode OR on the controller
  (next controller rev, or a small cable-side board for rev C).

### First port module: PM-19V-200W

Concept schematic, rev 0.1, 2026-10-21: `power/pm-19v-200w/` (KiCad
10). Netlist checked; component values are first estimates, not
verified on a bench.

| Item | Value |
|---|---|
| Input | 40-60 V; UVLO ~38 V, OVLO ~62 V |
| Output | 19.0 V +/-2 %, set by fixed resistors (a 19.5 V or 20 V variant is a resistor change) |
| Current | 10.5 A continuous, designed for 13 A, current limit ~14-16 A, hiccup on a short |
| Efficiency target | 97.5-98.5 % at 200 W (3-5 W loss) |
| Size target | about 25 x 60 mm, eighth-brick outline and pinout (DOSA) |
| Environment | immersed; no fan, no heatsink, no wet electrolytics |

Topology: two-phase synchronous buck, phases 180 degrees apart.

- Controller: ADI LTC7810 (4.5-140 V, dual, two-phase single-output
  mode: VFB2 = INTVCC, RUN2 = RUN1, ITH2 and TRACK/SS2 to 1 nF).
  200 kHz per phase (FREQ = GND), so 400 kHz ripple at the output and
  input; a low frequency keeps switching loss down at 54 V, and the
  fluid cools the larger inductors that need. Forced continuous mode.
  PLLIN/SPREAD to GND: no sync and no spread spectrum in rev 0.1
  (spread spectrum is one pin change, to INTVCC, if EMI asks for it).
- Output voltage: VOUT = 1 V x (1 + RB/RA), 180k / 10k = 19.0 V.
- Gate drive 10 V (DRVSET open). DRVCC comes from the VIN LDO at
  start-up and from VOUT through EXTVCC once running (19 V is above the
  switchover point, below the 65 V EXTVCC limit, and still below it when
  a shorted top FET puts the bus on VOUT). NDRV tied to DRVCC (unused).
- Per phase: two 100 V N-MOSFETs (candidate Infineon BSC070N10NS5, 7
  mOhm, to compare against GaN), 22 uH inductor (Isat >= 12 A, DCR as
  low as fits, 15 x 15 mm class), 8 mOhm 2512 sense resistor (current
  limit 67 mV min / 8 mOhm = 8.4 A peak per phase, ~14 A DC total;
  0.34 W per phase; inductor DCR sensing is the efficiency option),
  external bootstrap diode (BAS21, 250 V) and 0.1 uF.
- Ripple at 54 V in: ~2.8 A peak-peak per phase (~43 % at 6.5 A).
  Minimum on-time is no concern: 19 / (60 x 200k) = 1.6 us >> 90 ns.
- Input: 8 x 2.2 uF 100 V X7R 1210 (ceramic loses ~60 % at 54 V); VIN
  pin through 10 ohm and 1 uF.
- Output: 4 x 22 uF 35 V X7R 1210 plus 2 x 100 uF 35 V solid polymer.
- UVLO / OVLO: one divider VIN - 976k - RUN - 12.4k - OVLO - 19.6k -
  GND: start at 38.4 V, stop above 62.7 V. RUN is also the module's
  ON/OFF pin (brick pin 2): open = on, pulled low = off (default ON).
- Soft start: 0.1 uF on TRACK/SS1, ~10 ms ramp into the load's input
  capacitance.
- Compensation (ITH1): 15k + 2.2 nF, 100 pF, plus 47 pF feed-forward
  across the top feedback resistor. First guess, to tune on the bench.

Independent output supervision (does not depend on the LTC7810 or its
supplies):
- 4.7 V rail from VIN through 47k and a zener (~1 mA).
- Overvoltage: MCP65R46 comparator (internal 1.21 V reference,
  open-drain), VOUT / (178k + 10k) trips at ~22.7 V, pulls OV_KILL low.
  The front end cuts the port input on OV_KILL in hardware and latches
  off. This covers a shorted top FET.
- Power good: second MCP65R46, VOUT / (133k + 10k), PGOOD released
  (high) above ~17.3 V.

Interface (J1 eighth-brick pins, J2 auxiliary row beside them):

| Pin | Signal |
|---|---|
| J1.1 | +VIN |
| J1.2 | ON/OFF (RUN): open = on, low = off |
| J1.3 | -VIN (GND) |
| J1.4 | -VOUT (GND, non-isolated) |
| J1.5-7 | Sense-, Trim, Sense+: not connected (voltage is fixed in hardware) |
| J1.8 | +VOUT |
| J2.1 | OV_KILL, open drain, low = overvoltage |
| J2.2 | PGOOD, open drain, high = good |
| J2.3 | ID: resistor to GND, read by the carrier ADC (value table to define) |
| J2.4 | TEMP: 10k NTC to GND |
| J2.5 | GND |
| J2.6-7 | SDA, SCL: reserved (decided 2026-10-23), not on the rev 0.1 board yet; the next module rev grows J2 to 7 pins |

#### Mounting on the carrier (eighth-brick, DOSA)

The module is a brick: it carries pins, the carrier carries holes. The
module PCB sits parallel to the carrier on 8 shouldered pins pressed
or soldered into its own J1 holes; the pins protrude below the module
(B side, where U1, the FETs and the 1210 caps are) and go through the
carrier. Numbers below are from the SynQor NQ90W90EPx10 (rev D,
2026) and Artesyn eighth-brick drawings; both agree.

- Pin pattern, top view (component side, inductors up): pin 1 top
  left, 2, 3 down the left short edge at 0.300" (7.62) pitch, pins 8
  (top) to 4 (bottom) down the right edge at 0.150" (3.81) pitch,
  rows 2.000" (50.8) apart, 0.145" (3.68) from each short edge. The
  `pm.pretty/Brick_Eighth_DOSA` footprint matches this (checked
  2026-10-22, the "pin 1 chirality" note is closed); the module's
  F side is the brick's top, as it must be.
- Pins: 1-3 and 5-7 are 0.040" (1.02 mm) dia with a 0.080" (2.03)
  shoulder; 4 and 8 (the 10 A pins) 0.062" (1.57) with a 0.100"
  (2.54) shoulder. Copper alloy, tin over nickel. Length below the
  shoulder 0.145" (3.68) standard, 0.180" (4.57) long option (SynQor);
  Artesyn uses 0.188" (4.78). The shoulder sets the standoff: the
  module's bottom clearance is 0.020-0.070" (0.5-1.8 mm) plus
  whatever pin length is left above the carrier; with the 1.6 mm
  1210 caps on the B side take the long pins or add a 2-3 mm spacer
  under the shoulders. Candidate pins: Mill-Max press-fit PCB pins
  for plated through-holes (0.040" to 0.080" polygon press-fit end,
  solder or plug-in pin end), or any "power module pin" in these two
  diameters; pressed/soldered after JLC assembly. Module holes: J1
  drills 1.3 / 1.9 mm with 2.1 / 2.7 mm pads suit solder-in pins of
  these sizes (IPC-2222 level B); press-fit pins need the hole the pin
  maker specifies.
- Carrier footprint: the same 8 holes. Artesyn's recommended pattern
  is 0.058" +/-0.008 (1.47 +/-0.2 mm) for the 1.02 mm pins and
  0.080" +/-0.008 (2.03 +/-0.2 mm) for the 1.57 mm pins, IPC-2222
  level B, same 2.000" x 0.600" / 0.300" grid. The module sits on the
  carrier component side up, so both are viewed from the same side:
  the carrier's top-view hole pattern is the module's J1 pattern as
  is, NOT mirrored (an earlier note here said to put the footprint on
  the carrier's B side; that was wrong and is corrected 2026-10-22;
  `carrier.pretty/Brick_Eighth_DOSA_Socket` in the bench carrier is
  the host pattern, same pin coordinates as the module's J1). Pins 4
  and 8 carry 10.5 A: wide rings, via fields into the carrier's planes.
- J2 (the 5-pin 1.27 mm aux row) is ours, not DOSA: 5 more small
  pins and holes. An off-the-shelf fallback brick leaves them empty,
  so the carrier reads "ID open" as "no aux row" and then ignores
  PGOOD and OV_KILL (both open-drain with pull-ups, so absent reads
  "good") and relies on its own output sensing.
- Soldered, not socketed: immersed, 10 A, vibration, and an eighth
  brick has no mounting holes to hold it otherwise. Receptacles
  (Mill-Max makes ones for the 0.061" brick power pins) only on the
  bench carrier, for swapping modules.

#### Bench carrier: PM-BENCH-CARRIER

KiCad project `power/pm-bench-carrier/` (rev 0.1, 2026-10-22): the
board the module plugs into on the bench, until the 48 V power board
exists. 130 x 55 mm, 2 layers, bench only (wet electrolytics, not for
immersion). Schematic ERC clean, board routed, DRC clean (only the two
"silk clipped by edge" warnings from the terminal blocks that overhang
the edges by design). Not yet ordered. `pcbkit jlcfab` run 2026-10-22:
gerbers, BOM (15 lines, 20 SMD parts with LCSC numbers in the
schematic and board) and CPL in `fab/` (gitignored, regenerate). Hand
assembly: the receptacles, X2, J1, J2, JP1, C1, C2, C4, the test loops.
LCSC: fuse C44479 (0451010.MRL, 10 A: the 8 A is not stocked), TVS
C353345, zener C27131, 2N7002 C8545, C3 C55151, C5 C2918502, C6 C15849,
R1 C17943, R2 C17414, R3 C270956, 100k C25803, 3.3k C22978, 10k C25804,
green LEDs C2297, red LED C84256. The CPL rotations of D1, D2, Q1, F1
and the four LEDs are written uncorrected (no entry in pcbkit's
KiCad-to-EasyEDA table for these footprint/LCSC pairs): check them in
the JLC placement preview before ordering.

- X1: the eight DOSA holes with solder-mount Mill-Max receptacles so
  modules can be swapped: 0331 (accepts 0.94-1.09 mm pins, 1.85 mm
  hole, 12 A) on pins 1-3 and 5-7, 9372 (accepts 1.14-1.65 mm pins,
  2.64 mm hole, 18 A) on the 10 A pins 4 and 8. Pads 2.7 / 3.6 mm.
  Pin 1 top left as on the module. X2: 1.27 mm 5-pin socket at the
  module's J2 position (22.2 mm left of the brick centre, pin 1 at the
  brick's centre line), inside the brick outline.
- J1: 5.08 mm screw terminal (Phoenix MKDS 3/2, 24 A) for the lab
  supply, 40-60 V; nano2 fuse 8 A 125 V (F1), SMBJ60A TVS, 2 x 100 uF
  100 V electrolytic + 2.2 uF 1210 at pin 1, "VIN" LED (27k from VIN).
  J2: same terminal for the electronic load, 100 uF 35 V + 10 uF, "VOUT"
  LED (10k from VOUT). Input current limit on the supply ~6 A, load OVP
  ~22 V.
- Kelvin test points at the socket pins for efficiency measurement:
  TP1 VIN_K / TP2 GNDIN_K at pins 1 / 3, TP3 VOUT_K / TP4 GNDOUT_K at
  pins 8 / 4 (all 2.5 mm loops; the GND loops sit below the brick's
  bottom edge, the live ones above its top edge).
- Aux 5 V rail: 10k 1 W (2512) from VIN into a 5.1 V zener, ~5 mA at
  54 V: pull-ups and LEDs only. PWR and OV_KILL / PGOOD are open-drain
  on the module and must not be pulled above ~5 V.
- Module signals (X2): OV_KILL with 100k pull-up, "OV" LED lit when
  low; PGOOD with 100k pull-up, 2N7002 so the "PGOOD" LED is lit when
  high; ID and TEMP with 10k pull-ups to 5 V (a voltmeter at TP9 / TP10
  reads the module's ID resistor / NTC as a divider, the same way the
  carrier ADC will). Test points on every signal, TP5 a spare GND loop,
  TP11 the 5 V rail.
- JP1 (2-pin header, "OFF"): jumper on pulls RUN (pin 2) to GND = module
  off; open = on. TP6 RUN.
- JP2 / JP3: open solder jumpers from Sense+ / Sense- (pins 7 / 5) to
  VOUT / GND, for an off-the-shelf brick that wants its sense pins
  connected; Trim (pin 6) unconnected. Leave open for PM-19V-200W.
- No input cut-off on OV_KILL: the module is tested without a front
  end, so for the shorted-top-FET test the electronic load's OVP and the
  supply's current limit are the protection.
- Copper: VIN (2.5 mm) and VOUT (3 mm) trunks on both layers drawn by
  tools/power.py (group "power"; KRT only adds the small taps); GND is
  the pour on both layers (pcbkit gndpour) with solid zone connection
  on the 10 A pads (tools/place.py). The pour is made before routing
  and its zones go into the kept group, so KRT treats GND as a plane
  net and draws no GND tracks (the first pass had 58 of them, all
  redundant on a two-layer board with pours on both sides). Silk: pcbkit silk + connlabels +
  tools/legend.py (net names at the test points, LED names, +/- at the
  terminals).
- Open: the module's J2 is a plain 1.27 mm pin header whose tails
  (~3 mm) may not reach X2 once the module stands on the receptacle
  flanges plus pin shoulders (2-3 mm): take long-tail 1.27 mm pins on
  the module, or a taller socket here. Check with the chosen DOSA pins.
  Receptacle flange sizes (0331, 9372) not verified against the pad
  pitch (3.81 mm between pins 7 and 8) beyond the hole spec.

Open for this module:
- MOSFET choice, silicon versus GaN (switching loss at 54 V dominates).
- Inductor part, and resistor versus DCR current sensing.
- Compensation, checked with a load-step test.
- Pin part number and length (see mounting above), and whether the
  B-side parts need a spacer under the shoulders.
- Whether an off-the-shelf brick in the same footprint (e.g. SynQor
  NQ90W90EPx10, 10 A at 19 V = 190 W) is an acceptable fallback for
  loads under 190 W.
- J2 to 7 pins (decision 9, taken 2026-10-23): move the header and
  the carrier's X2 footprint at the next rev.

Marking: a large bold "19 V" legend on both silk layers (2026-10-22;
board-level text, kept by `pcbkit silk`, lost by a fresh `mkboard`), so
the rating is readable on a loose module. A voltage variant changes it.

Variants: the output voltage is a resistor change, by design never a
trim or a software setting (a 19 V build could not reach 24 V through
the DOSA +/-10 % trim convention anyway, and a runtime-adjustable Vout
would decouple the independent OV cut from the output). A 24 V build
(PM-24V-200W) on the same PCB, recipe 2026-10-22, not started:

| Ref | 19 V | 24 V | Note |
|---|---|---|---|
| R6/R7 (VFB, 1 V x (1 + R6/R7)) | 180k/10k | 232k/10k | 24.2 V; 230k for 24.0 V |
| C12 (feed-forward across R6) | 47p | 39p | zero stays near 9 us |
| R12/R13 (OV cut, 1.21 V x (1 + R12/R13)) | 178k/10k, 22.7 V | 226k/10k | 28.6 V; forgetting it kills a healthy module |
| R14/R15 (PGOOD) | 133k/10k, 17.3 V | 169k/10k | 21.7 V, 90 % |
| R16 (ID) | 10k | other | ID table to define |
| C20-C23 output ceramics | 35 V X7R 1210 | 50 V | DC bias at 24 V, must hold 28.6 V until OV_KILL |
| C24/C25 polymers | 35 V | 35 V (50 V if it fits) | 69 % of rating |
| Silk, title | 19 V | 24 V | |

Checked and unchanged: duty 40 % at 60 V in, 63 % at the 38 V UVLO;
EXTVCC at 24 V inside the 65 V limit, but its LDO drops 14 V instead
of 9 V at the gate-drive current (about 0.5 W instead of 0.3 W in the
LTC7810, bench thermal check); inductor ripple about 3 A pk-pk per
phase at 54 V with 4.2 A per phase at 200 W (22 uH fine, 33 uH for the
same ripple ratio); current limit stays about 14 A DC = 340 W, so
either R9/R10 8 -> 12 mOhm (about 9.5 A DC) for a 200 W class module
or let the front end's power limit do it; compensation is a bench
item either way. The bench carrier needs no change (load OVP set for
24 V).

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
3. The board logic is powered from the 19 V bus (local 3.3 V / 5 V
   regulators), not from the controller, so requirements 1-3 hold with
   the controller disconnected. Only the controller side of the I2C
   isolator takes power from J8.
4. EXP_nRESET from the controller must NOT reset this board's channel
   state. On the controller it resets the relay-drive expander on every
   MCU reset; here that would violate requirement 3. The board does not
   use EXP_nRESET.

### Interface to the controller

- Physical: cable to the controller's J8 expansion header (2x6, 2.54 mm),
  max 0.5 m. Used pins: external I2C SCL/SDA (J8 5/6), EXP_INT (J8 8),
  3V3 and GND.
- Galvanically isolated I2C (decided 2026-10-21). The isolator sits on
  the power board. Its controller side is powered from J8 3V3 (a few
  mA) and referenced to the controller GND in the cable; its board side
  runs from the power board's own 3.3 V / 5 V, made from the 19 V bus.
  The cable carries no bus current and the board's 19 V return never
  touches it. EXP_INT crosses the barrier too (a third, one-way
  channel: open drain on the controller side). Candidate parts: ISO1640
  or ISO1540 for SDA/SCL plus a one-channel isolator for EXP_INT, or a
  three-channel part if one exists in stock; to be chosen.
- With the controller absent or unpowered, the controller side of the
  isolator is dead, I2C is idle and every channel stays ON (default ON
  holds).
- I2C: 3.3 V levels, 100 or 400 kHz.
- Rev C splits I2C in two buses. The on-board parts (U14 0x20, U15
  0x21, U5 TMP1075 0x48) are on the internal bus and do not collide.
  The external bus (J8, J9) carries add-on MCP23017 modules at
  0x21-0x27 and the LTC4282s at 0x40-0x47, plus whatever Qwiic sensor
  is plugged into J9.
- EXP_INT: the board pulls it low (open drain) on any channel fault
  or power-good change. Shared with other modules.
- Grounds: board GND (the 19 V bus return) does NOT connect to the
  cable; the cable GND is the controller's, and only reaches the
  controller side of the isolator. The two grounds still meet through
  the 19 V bus, which also feeds the controller's J3, so no signal
  current shares the bus return. The controller's Ethernet side stays isolated by
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

For the 19 V reference design. The 48 V plane uses LM5069-2 + INA238,
see "Front end part" above (2026-10-23).

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

19 V reference design only; the 48 V plane front end has no EEPROM
(2026-10-23). Kept for the record.

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

Not on the controller (decided 2026-10-21): no logging to flash, on
the 8 MB module or on the 16 MB module planned for production. The
controller measures and publishes; it keeps only a short RAM window to
bridge a broker outage. So:

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
  LTC4282 and survives a controller reboot. An outage longer than the
  RAM window loses resolution, not energy.

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

Options that were considered:

1. Galvanically isolated I2C on J8, isolator on the power board, board
   side powered locally. CHOSEN 2026-10-21: fixes the ground problem.
   Chosen then with no MCU on the board; since 2026-10-23 one MCU per
   plane sits behind the isolator (see "Power plane"), the link itself
   is unchanged.
2. Differential I2C (e.g. PCA9615 over twisted pair) for a longer,
   noisier cable. Not needed at 0.5 m once isolated.
3. Power duck: an MCU on the power board, RS-485 / Modbus RTU to the
   controller. Reaches a whole pod, but costs an MCU, firmware and an
   address scheme. Not pursued in 2026-10-21; the plane MCU of
   2026-10-23 is this option's MCU without the RS-485 link (I2C
   stays), and its FDCAN keeps a pod-wide link open for later.

Still open with option 1: the shared bus. The isolated segment is on
the same controller bus as J9, so a stuck Qwiic sensor still blocks
monitoring (never power). Either accept it, or give J9 and J8 separate
segments (an I2C switch on the controller, next board revision).

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
- Link: galvanically isolated I2C on J8, power board side powered from
  its own 3.3 V / 5 V. Since 2026-10-23 one MCU per plane behind the
  isolator: retry counting, start-up stagger, energy integration and
  any front end provisioning run there; the power-cycle sequence and
  policy stay on the controller. (Before: no MCU, everything on the
  controller.)
- No power logging on the controller: it measures and publishes, a
  backend stores.

### Known unknowns

Interface and architecture:
- Isolator part numbers (two I2C channels plus one for EXP_INT).
- J9 and J8 on one bus segment or separate ones.
- Frames per controller (isolated I2C over J8 is one frame each).
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
- Ground: the I2C cable would share the 85 A return. Solved by the
  isolated link.
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

Decide next: in or out of the fluid, then the node's power input
(connector, current profile, BIOS setting).

## Open decisions

1. PSU model: whether the two supplies are designed to run in parallel
   (current share or OR-ing) and whether they have PMBus.
2. Node power input connector. A 5.5 x 2.5 mm barrel jack is not rated
   for 10.5 A; this decides the output connector and cabling.
3. Main input fuse location and type (on board vs. in the supply cabling).
4. Board outline, mounting and cable routing in the frame.
