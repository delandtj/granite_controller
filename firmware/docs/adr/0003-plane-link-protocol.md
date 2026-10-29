# Plane link protocol and the granite-proto crate

**Status**: Proposed
**Date**: 2026-10-29
**Updated**: 2026-10-29 (review asks answered by the user: EV_ACK, SUMMARY, asynchronous command outcome)

Related: docs/adr/0001-48v-power-plane.md ("Message format": the first
draft of this protocol), firmware/docs/adr/0002-plane-core-firmware.md
(plane side), docs/adr/0005-chained-plane-boards.md (units, chain).

---

## Context

The controller talks to up to 8 planes (0x30-0x37) on one isolated I2C
link at 100 kHz, plus a shared open-drain EXP_INT. ADR 0001 fixed the
shape: a 2-byte request (block, index), a length-prefixed TLV answer
with a CRC-8, global tags that never change meaning, unknown tags
skipped. It left open the block ids, the CRC polynomial, how a
control write is acknowledged, and how a lost answer is retried.

Nothing implements ADR 0001's tag table yet. Since then the plane has
changed: the chained plane has 1-4 units with their own hot-swap, input
INA238 and presence; the plane owns sequencing, so commands complete
asynchronously; the plane is updated and configured over the link.

One crate, `granite-proto`, holds the wire format. The plane firmware
(plane-logic), the controller firmware (granite-core / granite-fw) and
the simulator (granite-sim) all use it, so the two ends cannot drift.

From this ADR on, the tag table here is the authority; ADR 0001's table
is its draft. Tags freeze at the first plane shipped to a site.

---

## Decision

### 1. Framing

All multi-byte integers little endian. Address byte = the plane's
7-bit address.

**Read** (one I2C transaction, repeated start between write and read):

```
 W: block, index
 R: L, records[L], crc
 crc = CRC-8 over (address, block, index, L, records)
```

The CRC covers the request bytes and the address, which the plane does
not send back: an answer to a misheard request, or from another plane,
fails the check.

**Write** (one I2C write):

```
 W: block, index, L, records[L], crc
 crc = CRC-8 over (address, block, index, L, records)
```

A write with a bad CRC or a bad length is dropped and counted; the
plane never acts on it. The outcome is read back (decision 4).

**CRC-8**: polynomial 0x07, init 0x00, no reflection, no final XOR
(SMBus PEC).

**Record**: tag u8, length u8, value[length]. A reader skips unknown
tags by length and never assumes order. L <= 254.

### 2. Blocks

| Id | Name | Dir | Index | Content |
|---|---|---|---|---|
| 0x01 | INFO | R | 0xFF | identity, versions, topology, config hash, image state |
| 0x02 | STATUS | R | 0xFF | bus, flags, last command results, event head, counters |
| 0x03 | SUMMARY | R | 0xFF | one compact record per present slot (the 1 Hz poll) |
| 0x04 | UNIT | R | 0-3 | one unit: presence, hot-swap, input V / I / P |
| 0x05 | SLOT | R | 0-7 | one slot, full telemetry and state |
| 0x06 | EVENTS | R | 0xFF | oldest unacknowledged events, up to L |
| 0x07 | CONFIG | R | 0xFF | current plane config |
| 0x08 | LOG | R | 0xFF | recent log lines from the RAM ring |
| 0x10 | CTRL | W | 0xFF | commands (each record names its slot or unit) |
| 0x11 | EV_ACK | W | 0xFF | acknowledge events up to a sequence number |
| 0x12 | CONFIG_W | W | 0xFF | config records, applied and stored as one set |
| 0x13 | FW | W | 0xFF | firmware update records |

Slot index on the wire: 2 x unit + (A = 0, B = 1), unit 0-3, slot 0-7.
Reading a slot or unit that is not present returns L = 1 with a status
record that says so, never an I2C NAK (a NAK means the plane is gone).

### 3. Tags

Value types in brackets. Units: V in 10 mV, I in mA, P in 0.1 W, T in
0.1 C, time in s unless stated.

**INFO (0x01-0x0F)**
- 0x01 format version [u8] (this ADR = 1)
- 0x02 plane type [u16: 0x0001 PP-48 single board (rev1), 0x0002
  PP-48 chained]
- 0x03 slot capacity [u8] (8 for the chained plane)
- 0x04 firmware version [u8 x3]
- 0x05 serial [u32] (STM32 UID hash)
- 0x06 capabilities [u32 bits: node control, power limit, unit
  hot-swap, press deadline hardware, firmware update]
- 0x07 units present [u8 bitmap]
- 0x08 boot count [u32]
- 0x09 config hash [u32]
- 0x0A image state [u8: confirmed, probation, rolled back]
- 0x0B bootloader version [u8 x3]

**STATUS (0x10-0x1F)**
- 0x10 core input voltage [u16 V]
- 0x11 core temperature [i16 T] (MCU sensor)
- 0x12 uptime [u32 s]
- 0x13 flags [u16: any slot fault, any unit fault, events pending,
  sequence running, config dirty, update in progress, unconfirmed
  image]
- 0x14, 0x15 reserved (were the aux 12 V monitor; no such sensor)
- 0x16 command result [u8 seq, u8 code], one per finished or
  refused command still held (the last 8)
- 0x17 event head [u16 next seq]
- 0x18 update state [u8 state, u32 next offset]
- 0x19 link errors [u16 bad CRC, u16 bad length]
- 0x1A segment errors [u16 x4, per channel]
- 0x1B last reset cause [u8]

**SUMMARY (0x1C)**
- 0x1C slot summary [u8 slot, u16 status, u16 V, u16 I, u16 P, u8
  LED], one per present slot: 10 bytes, 8 slots = 96 bytes.

**SLOT (0x20-0x2F, 0x70-0x7F)**
- 0x20 slot status [u16 bits: present (module ID), enabled (OFF
  released), PGD, module PGOOD, OV latched, power limit tripped,
  retrying, sequence running, module run off, disarmed]
- 0x21 retry count [u16]
- 0x22 module class [u8]
- 0x23 module ID raw [u16]
- 0x24 V [u16], 0x25 I [u16], 0x26 P [u16]
- 0x27 P min / max since last read [u16, u16]
- 0x28 temperature [i16 T] (INA238 die)
- 0x29 energy [u64 J, u32 boot count]
- 0x2A reserved (module output V / I, not on this plane)
- 0x2B node LED [u8 off / on / blinking, u16 period ms]
- 0x2C cross-check flags [u8: LED at 0 A, LED while OFF, no LED after
  a power-cycle]
- 0x2D sequence state [u8: idle, off-wait, on-wait, pressing, done,
  failed]
- 0x70 power limit in force [u16 P]

**UNIT (0x30-0x3F)**
- 0x30 unit status [u16 bits: present, lost, reset seen (came back at
  hardware defaults), expander ok, input INA238 ok, hot-swap PGD,
  U_OFF asserted, hot-swap latched off]
- 0x31 input V [u16], 0x32 input I [u16], 0x33 input P [u16]
- 0x34 input INA238 temperature [i16 T]
- 0x35 segment errors [u16]
- 0x36 times lost [u16]

**CTRL (0x40-0x5F)**, every CTRL write starts with 0x4F:
- 0x4F command seq [u8]. The plane remembers the last 8 seqs; a write
  with a seq it has seen is not executed again, only its result kept.
  The controller retries a write with the same seq.
- 0x40 enable [u8 slot, u8 0/1]
- 0x41 clear OV latch [u8 slot]
- 0x42 retry policy [u8 slot, u8 max, u16 window, u8 action] (also a
  config record, see CONFIG)
- 0x43 reserved (was the boot stagger delay; dropped, ADR 0002)
- 0x44 clear min / max [u8 slot or 0xFF]
- 0x45 press PWR [u8 slot, u16 ms 50-10000]
- 0x46 press RST [u8 slot, u16 ms 50-10000]
- 0x47 power-cycle [u8 slot, u16 off delay, u8 press-if-dark, u16 LED
  timeout]
- 0x48 abort sequence [u8 slot]
- 0x49 module run [u8 slot, u8 0 = off / 1 = on]
- 0x4A unit hot-swap reset [u8 unit]
- 0x4B stagger on [u8 slot bitmap, u16 delay ms]
- 0x4C reboot [u32 magic 0x52424F54]
- 0x4D reset unit expanders [u32 magic 0x45585252] (EXP_RSTN, bused:
  every unit's expander at once, so every slot goes ON before the
  plane re-applies the state; recovery for a wedged expander only)
- 0x50 node on [u8 slot, u16 LED timeout s] (rail on if off, then a PWR
  press if the host LED stays dark; controller ADR 0004 verb `on`)
- 0x51 node off [u8 slot, u16 timeout s, u8 escalate: 0 none, 1 hold
  PWR 5 s, 2 cut the rail] (short PWR press, wait for the LED to go
  dark; controller ADR 0004 verb `off`)

Result codes (0x16): 0 ok, 1 accepted (running; a later result
follows), 2 busy, 3 bad argument, 4 conflict (sequence running on that
slot), 5 not present, 6 unknown tag, 7 refused (safety: e.g. RUNOFF
without DISARM is not offered), 8 failed (sequence ended in failure).
An unknown tag in a CTRL write is an error (code 6) for that record,
unlike a read where it is skipped.

**EVENTS (0x60-0x6F)**
- 0x60 event [u16 seq, u8 target (slot 0-7, unit 0x10-0x13, plane
  0xFF), u8 kind, u32 uptime s]
- 0x61 events lost [u16 count] (ring overflowed)
- Kinds: 1 slot on, 2 slot off, 3 PGD lost, 4 OV latched, 5 power
  limit trip, 6 retry limit reached (slot set OFF), 7 LED changed, 8
  sequence done, 9 sequence failed, 16 unit present, 17 unit lost, 18
  unit reset seen, 19 unit hot-swap latched off, 32 boot, 33 config
  stored, 34 update state changed, 35 image confirmed, 36 rolled back.

**EV_ACK (0x62)**: 0x62 ack through [u16 seq]. Reading EVENTS never
clears anything; the controller acknowledges after it has processed
them. EXP_INT stays asserted while unacknowledged events exist.

**CONFIG (0x80-0x8F)**, read in CONFIG, written in CONFIG_W:
- 0x80 config hash [u32] (read only; CRC-32 of the stored set)
- 0x81 retry policy [u8 slot, u8 max, u16 window, u8 action]
- 0x83 power limit [u8 slot, u16 P]
- 0x84 LED blink detection [u16 min period ms, u16 max period ms]
- 0x8F commit [] (last record of a CONFIG_W set; without it the set
  is discarded)

**FW (0x90-0x9F)**, in FW writes:
- 0x90 begin [u32 size, u32 CRC-32, u8 x3 version] (erases DFU)
- 0x91 data [u32 offset, u8 x n, n <= 128]
- 0x92 commit [u32 CRC-32] (verifies DFU, marks the swap)
- 0x93 confirm [] (the running image is good)
- 0x94 abort []

**LOG (0xA0)**: 0xA0 line [u32 uptime ms, u8 level, text <= 48 bytes].

### 4. Commands and their outcome

A CTRL write carries one seq and one or more commands. The controller
then reads STATUS: 0x16 gives (seq, code). Immediate commands end with
0; sequences answer 1 (accepted), then 0 or 8 when they end, and an
event (8 / 9) is raised so the controller learns it on EXP_INT
without polling. A missing result after 3 reads means the write was
lost: the controller resends with the same seq, and the plane's seq
memory makes that safe.

### 5. Polling and interrupts

- Every second per plane: STATUS (~40 bytes) and SUMMARY (~100 bytes).
  SLOT and UNIT blocks every 10 s, or at once when SUMMARY or an event
  shows a change. INFO at discovery, after a reboot (boot count), and
  when the config hash or image state changes.
- EXP_INT is shared by all planes (wired OR). On EXP_INT the
  controller reads STATUS of each plane; the "events pending" flag
  says which ones to read EVENTS from.
- Link load at 100 kHz: ~150 bytes plus overhead, ~20 ms per plane per
  second; 8 planes ~16 %. The full SLOT reads add ~5 % on average.

### 6. The crate

`firmware/granite-proto`, `no_std`, no alloc, no unsafe, no
dependencies beyond core.
- `crc8`, framing helpers: build a request, check and split a frame.
- `RecordWriter` into a `&mut [u8]` (returns an error when L would
  pass 254), `Records` iterator over a payload (yields `(tag, &[u8])`).
- Tag and block constants, result and event kind enums with
  `TryFrom<u8>`.
- Typed records: one struct per tag group (`SlotSummary`,
  `SlotStatus` bitflags, `UnitStatus`, `Event`, `Command`, ...) with
  `encode` / `decode`. Plane side encodes from its model; controller
  side decodes into its model.
- Host tests: round trips for every typed record, CRC vectors, a
  fuzz target on `Records` and frame checks (no panic on any input).

---

## Architecture Overview

### Component Breakdown

1. **granite-proto** (`firmware/granite-proto`)
   - Wire format only: no I2C, no timing, no state.
   - Used by plane-logic, granite-core (controller), granite-sim.
2. **Plane side** (plane-logic, ADR 0002)
   - Request handler: block + index -> encode from the model; CTRL /
     CONFIG_W / FW / EV_ACK -> queue.
3. **Controller side** (granite-core plane client, controller rev D
   spec)
   - Discovery, the poll schedule of decision 5, command seq and
     retry, event acknowledgement, the plane model per address.

### Data Flow / Interaction

```
 controller                          plane 0x3n
 ----------                          ----------
 W[01 FF] R[L INFO crc]   -------->  INFO      (discovery)
 W[02 FF] R[L STATUS crc] -------->  STATUS    (1 Hz, on EXP_INT)
 W[03 FF] R[L SUMMARY crc] ------->  SUMMARY   (1 Hz)
 W[10 FF L {4F seq}{45 slot ms} crc] --> CTRL  (press PWR)
 W[02 FF] R[... {16 seq 1} ...]      STATUS    accepted
       ... EXP_INT ...               event 8 "sequence done"
 W[06 FF] R[L {60 ...} crc]          EVENTS
 W[11 FF L {62 seq} crc]             EV_ACK    EXP_INT released
```

---

## Alternatives Considered

### Fixed register map (Modbus-like offsets)
- **The idea**: every value at a fixed register address, read in
  ranges.
- **Optimizes for**: the simplest decoder; tools like i2ctransfer can
  read it by hand.
- **Sharpest tradeoff**: every new plane type or field is a map
  version; optional values cost space on every plane.
- **Bets on**: one plane type forever. ADR 0001 already chose TLV.

### Events cleared on read (ADR 0001)
- **The idea**: reading EVENTS clears them and releases EXP_INT.
- **Optimizes for**: one transaction per event batch.
- **Sharpest tradeoff**: a read with a bad CRC loses the events it
  carried; the controller cannot ask again.
- **Bets on**: a clean link. Replaced by EV_ACK.

### SMBus block read with hardware PEC
- **The idea**: use the I2C peripherals' PEC hardware and the SMBus
  block read framing.
- **Optimizes for**: no software CRC.
- **Sharpest tradeoff**: the ESP32-C6's LP I2C (the controller's
  external bus) has no PEC hardware, and SMBus caps blocks at 32 bytes.
- **Bets on**: small blocks. The CRC here uses the same polynomial, so
  it can move to hardware later where it exists.

---

## Consequences

### Positive
- Both ends share one codec; a field added on one side exists on the
  other at compile time.
- Lost reads and writes are safe to retry (CRC over the request, seq
  memory, events acknowledged explicitly).
- The 1 Hz poll stays light enough for 8 planes on 100 kHz.

### Negative
- A command's outcome takes a second read (STATUS) instead of an
  immediate answer.
- The tag table is a frozen contract once a plane ships; mistakes stay
  as reserved numbers.

### Risks
- **Seq memory of 8 commands**: a controller that sends more than 8
  commands before reading results loses older results (not the
  commands). The controller keeps at most 4 in flight per plane.
- **Wired-OR EXP_INT with 8 planes**: one plane with an unacknowledged
  event holds it low; the controller must acknowledge or the line is
  useless. A plane whose events the controller cannot read (CRC
  failures) keeps it asserted; the controller falls back to the 1 Hz
  poll and flags the plane.

---

## What an Expert Would Ask

**Q: The controller reboots mid-command. What does it find?**
A: Its seq counter starts at a random value, so it cannot collide
with the plane's seq memory by accident. It rediscovers the plane
(INFO, STATUS), sees "sequence running" in the flags and the slot's
sequence state, and picks up the outcome from the event when it ends.
The plane does not care that the controller rebooted.

**Q: Two planes strapped to the same address.**
A: Both answer; every read fails its CRC (the two answers mix), and
the controller reports "address collision suspected" for that
address after repeated CRC failures with valid lengths. It cannot be
resolved in software; the straps must be fixed.

**Q: Can a corrupted write turn a slot off?**
A: Only if the corruption leaves a valid CRC-8 over the address and
the whole frame, about 1 in 256 for random corruption of a frame that
also keeps a valid length and tag structure. Enable / disable is a
deliberate action with a slot number and a 0/1 value; for higher
assurance the enable record could carry the slot number twice. Not
done; the link is a short isolated cable inside one box.

**Q: Is 254 bytes enough for SUMMARY at 8 slots plus future fields?**
A: 96 bytes now. Adding fields to the summary record means a new tag
(a tag never changes), so a future SUMMARY may carry both for a
while; there is room.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **Events acknowledged explicitly (EV_ACK).** Alternative: clear on
  read. Cost to change after release: a protocol version.
- **SUMMARY block for the 1 Hz poll.** Alternative: read every SLOT
  block each second (~6x the link load). Cost: controller only.
- **Command seq per write, results in STATUS.** Alternative: the result
  in a read right after the write (a pending-answer block). Cost to
  change after release: high, both ends.
- **CRC-8 over address and request.** Alternative: payload only (ADR
  0001's wording). Cost: none before release.

### Known unknowns and how the plan absorbs them
- Link error rate across several planes on the daisy chain: default
  100 kHz and 3 retries; if CRC errors pass ~1 %, lower the clock or
  shorten the daisy chain.
- Whether the controller's I2C driver does a repeated start into a
  read with the ESP-IDF API the firmware uses: default yes
  (write_read); fallback a stop between write and read, which the
  plane accepts (the request stays valid until the next request).

### The mechanical work
- Crate skeleton, constants, codec, typed records, host tests, fuzz
  target.
- Golden frames shared between plane-logic and granite-core tests.

Review asks: answered 2026-10-29, all three as proposed (EV_ACK,
SUMMARY for the 1 Hz poll, command outcome via seq + STATUS).

---

## Open Questions

**Architecture-changers**
- [x] EV_ACK, SUMMARY, asynchronous outcome (user 2026-10-29).

**Behavior definers**
- [ ] Number of planes on one daisy chain before the link needs a
  repeater or a second controller I2C port (signal integrity, not the
  protocol). Default: up to 8, measured on the bench.
