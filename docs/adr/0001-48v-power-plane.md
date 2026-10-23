# 48 V power plane and slimmed controller

**Status**: Accepted
**Date**: 2026-10-23
**Updated**: 2026-10-23 (accepted; review asks answered: TLV message
format over I2C, one node housing, STM32G0B1, policy on the controller
and primitives on the plane)

Related: docs/power-board.md (the running spec, sections "Direction:
48-54 V bus", "Power plane: PP-48-xS", "Front end part"),
docs/controller.md (rev C, open items 2 and 4), docs/handoff.md
(session 2026-10-23).

---

## Context

The granite controller rev C controls 8 Strix Halo nodes in one
immersion frame: front-panel PWR/RST through photoMOS, power LED
sense, 1-wire probes, Ethernet, MQTT/Modbus/HTTPS. It cannot cut DC
to a node. The power distribution spec of 2026-10-07 added an
8-channel 19 V hot-swap board (LTC4282) on an isolated I2C link from
J8, with no MCU on it, to give a real remote power cycle plus per-node
V/I/P/energy.

Since 2026-10-21 the direction is a 48-54 V bus (design 40-60 V in),
with each load behind a per-port DC/DC module. The first module,
PM-19V-200W, is drawn and routed as a DOSA eighth brick; a bench
carrier for it is drawn. The 19 V board design is the reference the
48 V design grew from; its parts do not fit at 48 V.

What this ADR settles, from the 2026-10-23 discussion:

- The carrier the bricks plug into exists in three sizes and more
  than one may hang off one controller. The old "one board per
  frame, no MCU" assumption no longer holds.
- Loads other than the 200 W node will appear. They get their own
  plane types later; the controller must not care.
- The controller duplicates per-node hardware that the plane slot
  can carry better, and it needs a supply that is not the 54 V bus.

Constraints: boards sit in dielectric fluid (no wet electrolytics, no
fans, soak-tested connectors, no unplugging under load). 48-54 V is
SELV. Default ON is a hard requirement: a node never loses power
because the controller, a cable or a firmware is absent. Fab and
assembly at JLC/LCSC where possible. Rust firmware on both MCUs.

---

## Decision

Two board families and one interface:

1. **Power plane PP-48-xS**: one design in 2, 4 and 8 slot sizes
   (6 = 8 with two slots unpopulated), 48 V only, one slot kind. Each
   slot feeds a DOSA eighth brick through a plane-side front end that
   switches, protects and meters the slot, carries the node's
   front-panel signals, and offers the slot's output connector. One
   STM32 per plane aggregates telemetry and relays commands; the
   safety chain is hardware. Every plane carries an always-on 12 V
   aux output for the controller.
2. **Controller rev D**: the rev C minus its node section. MCU,
   Ethernet, USB-C service, probes, J9, J8 as the only control
   interface, 12 V in from a plane (two inputs OR-ed). Rev C keeps
   serving until rev D exists; it talks to a plane as is.
3. **One message format** over isolated I2C, one address per plane,
   identical for every plane type now and later: tag-length-value
   records, so a plane type adds tags and an old controller skips
   what it does not know. The controller enumerates planes, reads
   type and slot count, exposes slots. It never learns a plane's
   internals.

Decisions folded in, each dated in docs/power-board.md:
- Front end per slot: TI LM5069-2 (switch, current limit, power
  limit, auto-retry) plus TI INA238 (16-bit monitor) on one 3.5 mOhm
  shunt. ADM1272, LTC4286/4287 and LM5066I rejected.
- Node control (PWR, RST, LED sense) moves from the controller to the
  plane slot. The module (brick) stays a pure converter.
- Controller supply: 12 V aux from the plane, never a slot, never the
  bus directly.
- J2 (module aux row) grows from 5 to 7 pins, SDA/SCL reserved.
- Mains / 220 V modules and inductive current sensing: discussed,
  dropped. The plane is 48 V only and meters from the shunt.

---

## Architecture Overview

```
 48-54 V bus (40-60 V) ---+-----------------------+----------------
                          |                       |
                   [ plane A, 8 slots ]    [ plane B, 2 slots ]
                   core: STM32, iso-I2C,   core: same
                         3V3 buck, 12 V aux
                   slot x8: LM5069-2 + INA238 + OV latch
                            + brick socket + J2
                            + photoMOS PWR/RST + LED sense
                            + node connector (Mini-Fit Jr 2x4)
                          |  ^  12 V aux          |  ^  12 V aux
          isolated I2C    |  |                    |  |
          + EXP_INT       v  |                    v  |
                   [ controller rev D ] <-- 12 V OR-ed from A and B
                     ESP32-C6, W5500, J8 master, probes, J9
                          |
                       Ethernet -> MQTT / Modbus TCP / HTTPS
```

### Component Breakdown

1. **Plane core** (one per plane; KiCad project `power/pp-48/`, to be
   created; schematic block "core")
   - 48 V input: lugs or a 50 A connector, main fuse, TVS clamping
     between 57 V and the 100 V parts. Bus copper for 50 A on the
     8-slot board, scaled down on the 4 and 2.
   - MCU: STM32G0B1 (candidate). I2C1 slave to the controller behind
     the isolator; I2C2 master to the INA238s and the plane's
     expanders; ADC for ID and TEMP of up to 8 slots; GPIO for
     enable, latch clear, PGD, photoMOS drive and LED sense (or an
     MCP23017 on I2C2 for the slot GPIO); FDCAN unused in rev 0.1,
     pads kept for a pod-wide link later; crystal for the energy
     time base; SWD header.
   - Supplies: 100 V buck (LM5164 class) 54 -> 3.3 V for the plane;
     a second 100 V buck 54 -> 12 V at about 1 A, always on, own
     fuse, for the controller. Nothing on the plane is fed from the
     controller cable.
   - Controller connector: controller side of the isolator (ISO1640
     class for SDA/SCL plus a one-channel isolator for EXP_INT),
     pins 3V3, GND, SCL, SDA, EXP_INT; powered from J8 3V3. In and
     out connectors for a daisy chain, or a star breakout at J8.
     Address straps, 2-3 bits, proposal 0x30-0x37. EXP_nRESET not
     carried. Beside it the 12 V aux pair.
   - Slot count and plane type readable from the register map; slot
     count from a strap or from the populated ID inputs.

2. **Slot block** (repeated N times; schematic block "slot")
   - LM5069-2, VSSOP-10: current limit 55 mV / 3.5 mOhm = 15.7 A
     (13.9-17.6 A), circuit breaker 30 A, power limit by RPWR for the
     MOSFET's SOA at 54 V, UVLO divider ~38 V, OVLO divider ~62 V,
     TIMER for insertion delay and fault timeout, PGD to the MCU.
     Auto-retry at 0.5 % duty on its own; the MCU counts retries
     from PGD and enforces the policy through UVLO/EN.
   - MOSFET: 100 V N-channel, 5x6, SOA for retrying into a short
     indefinitely (open).
   - Shunt: 3.5 mOhm 0.5 % low tempco 2512 2 W (or 2 x 7 mOhm).
     0.39 W at 10.5 A, 0.8 W at 15 A.
   - INA238, VSSOP-10, Kelvin on the same shunt, +/-163.84 mV range
     (LSB 1.4 mA, 47 A full scale), VBUS on the slot side of the
     MOSFET. A0/A1 strapped per slot: 16 addresses, 8 slots on one
     bus, no mux. Config written by the MCU at every boot. INA228 is
     the pin-compatible upgrade; INA237 the pin-compatible cheaper
     fallback.
   - Enable chain on UVLO/EN: divider holds ON; the MCU's open-drain
     output and the OV latch each pull it low for OFF. UVLO/EN is
     100 V abs max; the pull-downs sink only the divider current.
   - OV latch: set by the module's OV_KILL (open drain, low =
     overvoltage), holds UVLO/EN low, cleared only by an MCU command.
     Part open (dual flip-flop or discrete bistable).
   - Brick socket: the 8 DOSA holes of `pm.pretty/Brick_Eighth_DOSA`
     (soldered pins, no receptacles in the fluid) plus X2, the 7-pin
     1.27 mm J2 mate (OV_KILL, PGOOD, ID, TEMP, GND, SDA, SCL). ID and
     TEMP to the MCU ADC through 10k pull-ups.
   - Node control: two TLP176AM photoMOS (PWR, RST) with 270R LED
     resistors, one LED sense input (47k network), as on rev C.
   - Node connector: Mini-Fit Jr 2x4 class, two pins +VOUT, two GND,
     PWR_SW, RST_SW, PLED+, spare. One cable per node, splitting at
     the node end into DC input and front-panel header. Housing in
     the soak test.

3. **Module** (PM-19V-200W, `power/pm-19v-200w/`, exists)
   - Unchanged except J2 to 7 pins at its next rev. Pure converter:
     54 -> 19 V, hardware-fixed Vout and OV cut, ID resistor, NTC.
     Knows nothing about nodes or the plane.

4. **Controller rev D** (`granite_controller.kicad_*`, next rev)
   - Keeps: ESP32-C6, W5500 + magjack, USB-C service, 1-wire probes,
     J9 Qwiic, J8 with isolated-I2C master and EXP_INT, TMP1075,
     status LEDs.
   - Drops: 8 node connectors, 16 TLP176AM, U14/U15 MCP23017, LED
     sense networks, the 19 V input protection (SMAJ22A, 0.75 A PTC).
   - Adds: two 12 V inputs through an ideal-diode OR (LM66100 /
     LM74700 class) into the existing LMR51430 buck; TVS sized for
     12 V.
   - Firmware: plane discovery on J8 (scan 0x30-0x37, read map
     version, type, slot count), N planes x M slots exposed on MQTT,
     Modbus and the HTTPS page; node commands routed to the owning
     plane; retry and power-cycle policy; the LTC4282 provisioning
     code retired.

5. **Bench carrier** (`power/pm-bench-carrier/`, exists)
   - Unchanged. Its 5-pin X2 mates J2 pins 1-5.

### Message format (plane, I2C slave, TLV)

Decided 2026-10-23 (TLV preferred over fixed register offsets).

Transaction: the controller writes a request of 2 bytes, block id and
slot index (0xFF = plane level), then reads: byte 0 = payload length
L (0-254), bytes 1..L = TLV records, byte L+1 = CRC-8 over bytes
0..L. A control write is the same 2-byte request followed by TLV
records and a CRC-8; the plane answers the next read of block
STATUS with an ack record. A record is tag (u8), length (u8), value
(length bytes, little endian integers). Unknown tags are skipped by
length; a reader never assumes order. Tags are global, not per block,
and a tag's meaning never changes; new plane types add tags.

Blocks: INFO (plane), STATUS (plane), SLOT (per slot telemetry and
state), CTRL (per slot or plane control, write), EVENTS (what raised
EXP_INT, read clears).

Tags, first set (value type in brackets):
- Plane INFO: 0x01 format version [u8], 0x02 plane type [u16, 0x0001
  = PP-48-xS], 0x03 slot count [u8], 0x04 firmware version [u8 x3],
  0x05 serial [u32], 0x06 capabilities bitmap [u32: node control,
  aux monitor, module output monitor, FDCAN present, ...].
- Plane STATUS: 0x10 bus voltage [u16 mV x10 = 0.01 V], 0x11 plane
  temperature [i16 0.1 C], 0x12 uptime [u32 s], 0x13 global flags
  [u16: any fault, any PGOOD change, any LED change, stagger active,
  aux fuse open], 0x14 aux 12 V [u16 mV], 0x15 aux current [u16 mA],
  0x16 ack of last CTRL [u8 ok/err code].
- SLOT: 0x20 slot status [u16: present, enabled, pgood, OV latched,
  front end fault, retrying, sequence running], 0x21 retry count
  [u16], 0x22 module class [u8], 0x23 module ID code [u16], 0x24 V
  [u16 mV x10], 0x25 I [u16 mA], 0x26 P [u16 0.1 W], 0x27 P min/max
  since last read [u16, u16], 0x28 module TEMP [i16 0.1 C], 0x29
  energy [u64 J], 0x2A module output V/I [u16, u16, if present],
  0x2B node LED [u8 state: off/on/blinking, u16 period ms], 0x2C
  cross-check flags [u8: LED lit at 0 A, LED lit while OFF, no LED
  after power-cycle], 0x2D sequence state [u8: idle, off-wait,
  on-wait, pressing, done, failed].
- CTRL (write): 0x40 enable [u8 slot, u8 0/1], 0x41 clear OV latch
  [u8 slot], 0x42 retry policy [u8 max, u16 window s, u8 action],
  0x43 stagger delay [u16 ms], 0x44 clear min/max [u8 slot or 0xFF],
  0x45 press PWR [u8 slot, u16 ms 50-10000], 0x46 press RST [u8 slot,
  u16 ms], 0x47 power-cycle [u8 slot, u16 off delay s, u8
  press-if-dark, u16 LED timeout s], 0x48 abort sequence [u8 slot].
- EVENTS: 0x60 event [u8 slot, u8 kind, u32 uptime s] repeated.

EXP_INT: low while EVENTS is non-empty; a read of EVENTS clears it.
Size: one SLOT block is about 60 bytes; a plane of 8 slots is read in
9 transactions, about 600 bytes, well under 3 % of a 100 kHz bus at
1 Hz for three planes.

### Safety rules (hardware default ON; the MCU is never in the chain)

1. Every UVLO/EN is pulled to ON by its divider. The MCU pulls low
   through open drain for OFF. MCU in reset, unpowered or crashed
   reads as ON.
2. OFF lives in MCU RAM: survives a controller reboot and a cable
   unplug. An MCU watchdog reset returns the slot to ON (same class
   as the accepted LTC4282 power-loss case). Restoring OFF from RTC
   backup registers is an option.
3. The OV cut is a hardware latch from OV_KILL into UVLO/EN. The MCU
   reads and clears it, never implements it.
4. Start-up stagger: within 50 ms of bus power the MCU holds all but
   the first slot low and releases them in sequence (default 0.5 s).
   A dead MCU holds nothing: everything comes on together, the
   hardware default, not a lockout.
5. Nothing on the plane is fed from the controller cable. The
   controller side of the isolator is the only thing on J8 3V3.
6. OFF is never set through a monitor or hot-swap register; the pin
   default rules.

### Data flow

Plane MCU polls each INA238 at 10-20 Hz (1 ms conversion, averaged),
keeps min/max, integrates energy against its crystal, samples ID and
TEMP, reads PGD and LED sense, runs sequences. Controller polls each
plane at 1 Hz (one block read per plane), publishes per frame every
10 s on MQTT, raises events on EXP_INT, exposes Modbus input
registers. No logging on the controller; a backend stores (unchanged
from the 2026-10-21 decision).

---

## Alternatives Considered

### No MCU: front ends on the controller's bus, address translator or hub per plane
- **The idea**: keep the 2026-10-21 design; PMBus hot-swap chips plus
  an ADC and an expander per plane behind the isolator; per-plane
  LTC4316 address offset or a TCA9548A hub at J8 to separate planes.
- **Optimizes for**: no second firmware, no MCU in any FMEA.
- **Sharpest tradeoff**: the controller must know every plane's
  internals and population, drive 24 front ends plus helper chips
  over software I2C on GPIO6/7, and run every retry counter and
  stagger itself. Each new plane type is a controller firmware
  change.
- **Bets on**: one plane type, one plane per controller, forever. The
  2/4/8 sizes and "other plane types later" already break it.

### Integrated PMBus front end (LM5066I) instead of LM5069-2 + INA238
- **The idea**: one chip per slot that switches, protects and meters.
- **Optimizes for**: part count, one datasheet.
- **Sharpest tradeoff**: 12-bit telemetry shared with the protection
  ADC: current +/-3.5 % at 80 % of range, +/-6 % at 19 %. The spec
  asks 2 % of reading at 10 A. Also $8.58 against about $5.70 for the
  pair, 129 in stock against 7219.
- **Bets on**: metering accuracy not mattering below 10 A. It does:
  idle nodes sit at 1-4 A and that is where billing and health checks
  look.

### Power duck: MCU on the plane, RS-485 or CAN to the controller
- **The idea**: the plane MCU as decided, but a long-reach link
  instead of isolated I2C, one bus for a whole pod.
- **Optimizes for**: reach (9 frames on one controller).
- **Sharpest tradeoff**: new transceivers on both ends, a controller
  rev that has the port, an address scheme, and the rev C controller
  cannot talk to it at all.
- **Bets on**: pod-wide reach being needed before rev D. Kept open:
  the G0B1's FDCAN pads stay on the plane, unpopulated.

### Controller keeps the node section
- **The idea**: plane does DC only; PWR/RST/LED stay on the
  controller's 8 connectors as in rev C.
- **Optimizes for**: no change to rev C, the photoMOS stay where they
  are tested.
- **Sharpest tradeoff**: two cables per node in the fluid, the
  controller caps nodes at 8, and the LED-versus-current cross-check
  needs two boards to agree over I2C; the power-cycle sequence needs
  both boards alive.
- **Bets on**: nodes per controller never exceeding 8 and frames
  without a plane remaining a supported configuration. The 12 V aux
  decision made the second moot.

### Controller fed from slot 1's brick, or a 40-60 V controller input
- **The idea**: no aux converter; the controller takes 19 V from the
  first slot, or a later controller rev takes the bus directly.
- **Optimizes for**: one less buck on the plane.
- **Sharpest tradeoff**: the controller downstream of a switched slot
  violates "never downstream of a channel": cycling node 1 reboots
  the controller, a node 1 fault drops it in step with the front
  end's retry, and the slot's protection is sized for a 200 W board.
  A 54 V controller input makes rev C unusable and adds 100 V parts
  to a board that otherwise needs none.
- **Bets on**: node 1 never being power-cycled. It will be.

### Mains (220 V) modules on the same plane, and inductive current sensing
- **The idea**: a slot kind that passes 230 V AC with a relay and
  meters it; CT or Hall sensors so the plane meters any module
  without touching its circuit.
- **Optimizes for**: one plane for every load in the room.
- **Sharpest tradeoff**: mains on a SELV board in fluid means
  reinforced creepage, a safety standard and a live soak test; a
  Hall sensor at 1-2 % of full scale is 8 % of reading at 4 A, and a
  CT gives amps, not watts. Dropped 2026-10-23; a mains load gets its
  own self-contained module with SELV signals only, later, on its own
  plane type.
- **Bets on**: 48 V loads being all that matters for the next design
  cycle.

---

## Consequences

### Positive
- The controller is an I2C master with a network port. Node count,
  load class and plane size are the plane's business.
- One cable per node. The slot is the node's single attachment.
- Every fact about a node is in one place: DC state, current, LED,
  buttons. Cross-checks and the power-cycle sequence are local and
  survive a controller reboot.
- Metering at a percent of reading from under 1 A to 47 A, energy
  integrated on a crystal, no EEPROM and no address bookkeeping.
- Default ON, OV cut and current limit are all resistors, dividers
  and a latch. No firmware is in the safety chain.
- The controller survives a plane loss when fed from two planes.

### Negative
- A second firmware target (STM32, embassy-stm32, I2C slave), an
  update path, and a register map to version.
- A dead plane MCU takes a frame's control, buttons and LED sense
  included. Service is unaffected (default ON), control is not.
- Rev C's node section becomes dead weight on frames with a plane;
  a controller rev is needed to shed it.
- The 8-slot plane carries 50 A of bus copper in the fluid: heavy
  copper or a busbar, and the soak list grows (Mini-Fit housings,
  the controller connector, photoMOS, 2512 shunts).
- INA238 is not at LCSC today: consigned or hand-placed for the first
  planes.

### Risks
- MOSFET SOA retrying indefinitely into a short at 54 V with power
  limiting. Mitigation: pick the MOSFET with the LM5069 SOA
  procedure, test with a dead short on the bench, set a retry cap
  in the MCU policy.
- The OV latch and the LM5069's own retry interact: a latched slot
  must stay off through the LM5069's restart cycles. Mitigation: the
  latch drives UVLO/EN, which the LM5069 honours regardless of its
  timer; verify on the bench.
- Isolated I2C with three planes in parallel on the controller-side
  bus: capacitance and the isolator's low-level offset. Mitigation:
  100 kHz, short cables, pull-ups sized once per bus, tested with
  three planes before the 8-slot order.
- MCU brownout during a bus sag could clear an OFF the operator
  wanted. Mitigation: BOR threshold set, OFF optionally restored
  from RTC backup registers; the consequence is a node turning on,
  never off.
- Soak: photoMOS, Mini-Fit, 2512 shunts, the isolator package. Same
  soak plan as the controller parts.

---

## What an Expert Would Ask

**Q: You moved OFF into an MCU. What exactly turns a node off when
the MCU is dead, and what turns it on again?**
A: Nothing turns it off: a dead MCU releases every UVLO/EN and the
dividers hold every slot ON. That is the design, not a gap. The OV
latch still cuts a slot on a module overvoltage with the MCU dead,
and nothing clears it until an MCU is alive and told to. A slot that
was OFF when the MCU died comes back ON; the operator sees it as a
node that rebooted, and the controller sees a plane that stopped
answering. Not handled: holding a deliberately OFF slot OFF through
an MCU death. Acceptable because the failure direction is "node
runs", the same as the 2026-10-21 design when the LTC4282 lost power.

**Q: 3.5 mOhm at 15 A limit means the LM5069 regulates at 15.7 A
typical but anywhere from 13.9 to 17.6 A. A 13 A design current for
the module with a 13.9 A worst-case limit is tight. Is it?**
A: The module's own current limit is 14-16 A DC and it draws 10.5 A
at 200 W, so the slot limit only matters during the brick's inrush
and a brick fault. During inrush the LM5069 regulates, the TIMER
sets how long, and the brick's soft start (10 ms) keeps it short.
The plane limit is meant to protect the plane and the bus, not to
set the module's current. If the 13.9 A corner bites on the bench,
the shunt goes to 3.3 mOhm (16.7 A typical, 14.7 A min) at the cost
of a hair of metering range nobody uses.

**Q: Eight INA238s on one I2C bus at 10-20 Hz each, plus an expander,
plus the controller's slave traffic on the other peripheral. Does the
G0 keep up and does the isolated bus to the controller stay clean?**
A: Per INA238 read: shunt, bus, three 16-bit registers, about 15
bytes, 1.5 ms at 100 kHz; 8 slots at 20 Hz is 24 % of a 100 kHz bus,
6 % at 400 kHz, which the INA238 supports. The MCU is otherwise idle.
The controller-side bus sees one block read per plane per second:
about 300 bytes, under 3 % at 100 kHz for three planes. The two
buses are separate peripherals, so slave traffic never waits on a
monitor read. Not handled: I2C bus recovery on the monitor bus if an
INA238 hangs SDA; the MCU can clock it out, which the controller's
software I2C could not.

**Q: Why not put the INA238 on the module output too, now that J2
has SDA/SCL? You would get module efficiency per slot for 2 EUR.**
A: Because the module rev 0.1 has no room and no pins for it, and
the number is a health figure, not a requirement. The pins are
reserved so that a module rev can add it without touching any
carrier. The plane MCU already has the register fields for it. The
cost of deciding it now is a module re-spin before the first module
has been measured; the cost of deciding later is zero.

**Q: Three plane sizes is three PCB layouts. Why not one 8-slot board
depopulated to 2 and 4?**
A: Because the 8-slot board is sized for 50 A of bus copper and
eight node connectors, and a 2-node frame paying for that board and
that fluid volume is wrong. The schematic is one design with the slot
block instanced N times, so the layouts share every footprint, every
value and the firmware. The 4-slot is explicitly last and may never
be made.

**Q: What does an operator do when a plane's firmware needs an
update, in the fluid, with eight nodes running on it?**
A: Not handled in rev 0.1: update is over SWD on the bench. Nodes
keep running through an MCU reset (rule 1), so an in-fluid update
would be safe once an I2C bootloader exists, but the plane must then
be pulled or the controller cable reached. Listed as open; the
answer may be "never update a plane in service, swap it".

---

## Implementation Plan

### Decisions you will probably want to tweak

- **Message format: TLV blocks with a CRC-8.**
  Choice: 2-byte request, length-prefixed TLV payload, global tags,
  CRC-8. Alternative: fixed register offsets with a version field.
  Cost to change later: every controller firmware and every plane
  firmware in the field; it is decided and will not move.
- **Slot node connector: Mini-Fit Jr 2x4 with power and front panel in
  one housing.**
  Choice as stated. Alternative: separate power (2-pin Mini-Fit) and
  signal (JST PH 5-pin, the rev C cable) connectors. Cost to change
  later: every node cable and the plane footprint; cheap before the
  first plane order, expensive after a frame is cabled.
- **J2 at 7 pins with SDA/SCL reserved.**
  Choice as stated. Alternative: stay at 5. Cost to change later:
  every carrier and plane footprint plus the module.
- **Plane address range 0x30-0x37 and strap count.**
  Choice as stated. Alternative: a resistor-coded address read by the
  MCU ADC (more planes, one pin). Cost to change later: controller
  discovery code only; cheap.
- **STM32G0B1 with FDCAN pads.**
  Choice as stated. Alternative: STM32G431 (faster ADC, more I2C) or
  an ESP32-C3 (same toolchain as the controller, no I2C slave
  maturity). Cost to change later: plane core layout and firmware
  HAL; moderate.
- **Policy on the controller, primitives on the plane.**
  Choice: press, power-cycle sequence, retry counting on the plane;
  retry limits, soft-shutdown-first, scheduling on the controller.
  Alternative: all policy on the plane. Cost to change later: moves
  code between two firmwares; moderate, no hardware impact.

### Known unknowns and how the plan absorbs them

- **MOSFET SOA at 54 V retrying into a short.** Default: a 100 V 5x6
  part chosen with the LM5069 power-limit procedure, retry cap 5 per
  10 min in the MCU policy. Pivot signal: the bench short test heats
  the FET above its SOA curve or the LM5069 power limit sets the
  inrush time above the brick's needs.
- **INA238 sourcing.** Default: Mouser, hand-placed or consigned.
  Pivot: LCSC stock of INA238 or INA228 appears, switch the BOM line;
  same footprint.
- **Isolated I2C with three planes in parallel.** Default: ISO1640,
  100 kHz, one pull-up set at the controller. Pivot signal: eye
  closure or NACKs with three planes on the bench; then one
  isolator per branch on a J8 hub.
- **Slot count detection.** Default: a strap. Pivot: if the 6-slot
  variant proves common, read population from the ID inputs instead.
- **OV latch part.** Default: a 74LVC1G74-class flip-flop per slot
  with the MCU clear. Pivot: if the latch races the LM5069's own
  restart, move to a latch that also gates TIMER.
- **In-fluid firmware update.** Default: none, SWD on the bench.
  Pivot: the first time a fielded plane needs a fix; then the I2C
  bootloader.

### The mechanical work

- `power/pp-48/`: KiCad project, hierarchical sheets "core" and
  "slot", slot instanced 2/4/8; pcbkit.toml; tools/ for pours and
  silk as on the carrier; three board files from one schematic.
- Plane firmware: Rust, embassy-stm32, I2C slave with the register
  map, INA238 driver, sequence engine, retry policy, stagger, ADC
  for ID/TEMP, watchdog, BOR.
- Controller firmware: plane discovery, slot model, MQTT/Modbus/HTTPS
  exposure per plane and slot, node commands routed to planes,
  removal of the LTC4282 code paths.
- Module rev 0.2: J2 to 7 pins; carrier X2 follows.
- Controller rev D: node section removed, 12 V OR-ed input, J3 TVS.
- Bench: 2-slot plane first, with a rev C controller on its 12 V aux;
  then short, inrush, retry, OV latch, stagger, three-plane bus,
  soak.

Review asks, answered 2026-10-23:
1. Message format: TLV records (user's preference over fixed
   offsets). Decided; format above.
2. Node connector: one Mini-Fit Jr 2x4 housing for power and front
   panel. Decided.
3. STM32G0B1 as the plane MCU. Decided.
4. Primitives on the plane, policy on the controller. Decided.

---

## Open Questions

**Architecture-changers**
- [x] Message encoding: TLV with CRC-8 (2026-10-23).
- [ ] One plane type carries the 12 V aux; does every plane type have
      to, or only PP-48?
- [ ] Frames per controller and cable topology: daisy chain on the
      plane or a star hub at J8.

**Behavior definers**
- [ ] Retry policy defaults (count, window, action) and whether the
      plane enforces a cap with the controller absent.
- [ ] OFF across an MCU reset: plain default ON, or restored from
      backup registers.
- [ ] Stagger order and delay when a plane boots with the controller
      absent.
- [ ] Power-cycle primitive: press-if-dark timeout and what "dark"
      means for a blinking LED.
- [ ] Slot output voltage reading: on the slot side of the MOSFET as
      proposed, or the brick output (needs J2 pins 6-7).

**Polish**
- [ ] Names: PP-48-2S / 4S / 8S, controller "rev D". Proposed, not
      contested.
