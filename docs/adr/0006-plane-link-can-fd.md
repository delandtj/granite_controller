# Plane link over isolated CAN FD

**Status**: Proposed
**Date**: 2026-11-04

Related: docs/adr/0001-48v-power-plane.md (decision 3, "one message
format over isolated I2C"; the FDCAN note under review ask 3),
docs/adr/0005-chained-plane-boards.md (core board, link connectors),
firmware/docs/adr/0003-plane-link-protocol.md (framing, blocks,
polling), docs/controller-rev-d.md ("Plane link", pin map).

---

## Context

The controller talks to up to 8 planes over one isolated I2C bus at
100 kHz, daisy chained on JST PH 5p cables (CTRL_3V3, CTRL_GND, SCL,
SDA, EXP_INT), plus a wired-OR EXP_INT. Each plane isolates the link
with an ISO1640 (I2C) and an ISO7721 (EXP_INT), side 1 on the cable.

Problems:

1. **The isolator topology is outside TI's rules.** The ISO164x side 1
   uses a static output offset against latch-up; TI states side 1 must
   connect to exactly one node and only side 2 may be shared. Today 8
   side 1 ports share the cable bus, and side 1 has a small capacitive
   load limit (TI quotes 40 pF for the ISO1540 side 1) against the
   ~0.5 nF rev D expects for a long chain. Expected to fail on the
   bench with more than one plane.
2. **I2C on cables in the fluid**: single ended, capacitance bound,
   next to 250 W converters. One stuck SDA (hung slave, damaged
   isolator) stops every plane on the chain.
3. **Throughput**: ~10 kB/s shared by 8 planes, polled, with a side
   wire for events. Firmware updates over the link are slow.
4. **One master only.** A standby controller that takes over while
   the master is down (upgrades) cannot share an I2C bus.

Constraints: parts from JLCPCB stock; the plane MCU is the
STM32G070CBT6 (no CAN) because the STM32G0B1 (FDCAN, same LQFP-48
pinout) has been out of stock since 2026-10-26 (stock 2026-11-04:
G0B1CBT6 0, CCT6 45, CET6 0); the controller is the ESP32-C6, whose
TWAI controllers are classic CAN only. Controller rev D is still a
draft (docs/controller-rev-d.md), the core board's link block is one
sheet (power/pp-48/tools/sch/blocks/link.py).

---

## Decision

1. **Physical layer: CAN FD**, 500 kbit/s arbitration, 2 Mbit/s data
   phase, 11-bit identifiers. Linear bus, daisy chained through each
   plane's two link connectors, 120 ohm at both physical ends.

2. **Isolation at each plane** with an isolated CAN FD transceiver,
   **ISO1044BDR** (C2878225, SOIC-8, 6478 in stock, $1.87; 5 Mbit/s
   FD, side 1 1.71-5.5 V, side 2 4.5-5.5 V, 3 kVrms basic). Second
   source CA-IS1044S (C7424699, 4580, $1.17), pinout to confirm.
   Side 1 on the plane's 3V3, side 2 on the cable supply. The ISO1640
   (U4), ISO7721 (U5) and the EXP_INT line go.

3. **Cable supply is 5 V** (ISO1044 side 2 needs 4.5-5.5 V; was
   CTRL_3V3). The controller feeds it per connector through a
   current-limited load switch, as rev D planned for 3V3.

4. **Plane CAN controller: either MCU, one layout.**
   - **G070 build** (default while the G0B1 is out of stock):
     STM32G070CBT6 + **MCP2518FDT-E/SL** (C626758, SOIC-14, 19157,
     $1.51) on SPI1, 40 MHz crystal (X322540MPB4SI, C9010, 68172,
     $0.11).
   - **G0B1 build**: STM32G0B1CCT6 in the same footprint, FDCAN1 on
     PB8 / PB9; MCP2518FD and its crystal not fitted.
   - ISO1044 RXD drives both the MCP2518FD RXCAN and PB8 (FDCAN1_RX,
     AF3). ISO1044 TXD is driven by the MCP2518FD TXCAN or PB9
     (FDCAN1_TX, AF3); never both, because the G0B1 build does not
     fit the MCP2518FD and the G070 firmware leaves PB9 in its reset
     state (analog, high-Z). No jumpers.

5. **Plane MCU pins** (LQFP-48, G070 and G0B1 identical for these):

   | Pin | Port | Was | Becomes |
   |---|---|---|---|
   | 47 | PB8 | SCL1 (I2C1 slave, ISO1640) | CAN_RX: ISO1044 RXD (FDCAN1_RX on the G0B1) |
   | 48 | PB9 | SDA1 (I2C1 slave, ISO1640) | CAN_TX: ISO1044 TXD (FDCAN1_TX on the G0B1) |
   | 15 | PA4 | unused | SPI1_NSS: MCP2518FD nCS |
   | 16 | PA5 | unused | SPI1_SCK |
   | 17 | PA6 | unused | SPI1_MISO: MCP2518FD SDO |
   | 18 | PA7 | unused | SPI1_MOSI: MCP2518FD SDI |
   | 11 | PA0 | unused | MCP_INT (MCP2518FD nINT, EXTI) |
   | 40 | PD2 | INT_OUT (EXP_INT via ISO7721) | free |

   Notes: pin 10 is NRST on the G070 and PF2-NRST on the G0B1 (reset
   by default, unchanged). Pins 29 / 32 are bonded on the G0B1 only
   (PA9 / PA10, UCPD dead-battery pull-downs at reset); they stay
   unconnected. The address straps ADR0-2 become the node id.

6. **Termination** on the plane: split termination (2 x 60R4 + 4.7 nF
   to cable ground), switched by a 4th DIP position (SW1 from 3 to 4
   positions, SW_DIP_x04). On only on the last plane of a chain.

7. **Controller side (rev D)**:
   - **MCP2518FD on the W5500's SPI bus** (GPIO19 / 20 / 21), nCS on
     GPIO2, nINT on GPIO3 (both freed by the I2C link). GPIO0 freed by
     EXP_INT. The MCP2518FD's SPI limit (~17 MHz at 40 MHz SYSCLK)
     applies to its transactions only; esp-hal sets the clock per
     device.
   - **Non-isolated transceiver**, TCAN1044VDRQ1 (C1852061, 15487,
     $0.46; VIO pin on 3V3, VCC on 5 V). Isolation is at each plane;
     the controller's ground is the cable ground.
   - **Both connectors are one bus, the controller in the middle**:
     chain A - controller - chain B, the transceiver tapped between
     the two connectors with short stubs. The controller's own
     termination is switchable (on when only one connector is used);
     the far plane of each chain terminates its end.
   - ESD on CANH / CANL at each connector (part to pick, a 2-line CAN
     TVS in SOT-23 class).

8. **Connector pinout** (JST PH 5p, unchanged part, pin 1 to pin 1):
   1 CAN_5V, 2 CAN_GND, 3 CANL, 4 CANH, 5 CAN_GND. CANH / CANL
   adjacent; the second ground keeps a return next to the pair.

9. **Message layer: blocks and tags of firmware ADR 0003 unchanged.**
   What changes is the transport:
   - **Identifiers** (11 bit): `id = (type << 4) | node`. Node 0-7 =
     plane (address straps), 14 = controller, 15 = standby
     controller. Types, in priority order:

     | Type | Name | From | Content |
     |---|---|---|---|
     | 0 | SYNC | controller | master keepalive / broadcast (decision 11) |
     | 1 | EVENT | plane | "events pending" + head seq; replaces EXP_INT |
     | 2 | REQ | controller | read request or write (block, index, L, records, crc) |
     | 3 | RSP | plane | response (L, records, crc) |
     | 4 | BULK | either | FW and LOG segments, lowest priority |

   - **Segmentation**: a request or response is the ADR 0003 byte
     string, cut into CAN FD frames of up to 64 bytes. Byte 0 of each
     frame = segment index (bits 0-6) + last flag (bit 7); 63 payload
     bytes per frame, so L <= 254 needs at most 5 frames. A receiver
     drops a message on a gap in the index.
   - **CRC-8 stays**, end to end over the reassembled message, with
     the node id in place of the I2C address. CAN FD's own frame CRC
     covers each frame; the CRC-8 covers reassembly and the SPI hop.
   - **Events**: a plane sends an EVENT frame when it raises an event
     and repeats it every 1 s until an EV_ACK covers the head seq.
   - **Polling** unchanged in content (STATUS + SUMMARY per plane per
     second). Load: ~150 bytes = 3 frames, ~1 ms per plane; 8 planes
     ~1 % of the bus.

10. **Bit timing**: G0B1 FDCAN kernel clock 40 MHz from PLLQ (8 MHz
    HSE); MCP2518FD SYSCLK 40 MHz. Sample point 80 % nominal, 75 %
    data; transmitter delay compensation on (the isolator adds loop
    delay).

11. **Standby controller: designed for, not built.** SYNC (type 0,
    highest priority) is the master's keepalive and carries an epoch
    and the commanded slot states. A standby (node 15) listens; on
    no SYNC for T it polls and publishes monitoring only (never CTRL
    writes) until a master SYNC appears again, which then wins
    arbitration by priority. The details are a later ADR; this one
    only reserves the identifiers and the switchable controller
    termination that makes a standby at a chain end possible.

---

## Architecture Overview

### Component Breakdown

1. **Plane core link block** (power/pp-48/tools/sch/blocks/link.py)
   - ISO1044 U4 (side 1 +3V3 / GND, side 2 CAN_5V / CAN_GND),
     decoupling both sides, split termination with SW1 position 4,
     J2 / J3 PH 5p (decision 8).
   - Interfaces: CAN_TX / CAN_RX to the MCU sheet.
2. **Plane core MCU block** (power/pp-48/tools/sch/blocks/mcu.py)
   - MCP2518FD U5 with 40 MHz crystal, on SPI1 and PA0; marked
     "G070 build only" in its description field.
   - CORE_PINS / PIN_USE per decision 5; firmware/plane-core-pins.toml
     regenerated from it.
3. **Controller rev D** (docs/controller-rev-d.md, other session)
   - MCP2518FD on the W5500 SPI, TCAN1044V, switchable termination,
     two connectors on one bus, 5 V load switch per connector.
4. **granite-proto** (firmware/granite-proto)
   - Framing gains `segment` / `reassemble` and the id helpers; blocks,
     tags, records, CRC-8 unchanged. Still no I/O.
5. **plane-logic link** (firmware/plane-logic/src/link.rs)
   - A transport trait (receive message, send message, send event)
     with two drivers: FDCAN (embassy-stm32, G0B1) and MCP2518FD
     (SPI). Selected at build time by the fitted MCU.
6. **Controller firmware** (firmware/granite-fw)
   - MCP2518FD driver on the shared SPI, transport as above, EVENT
     frames in place of the EXP_INT GPIO.

### Data Flow / Interaction

```
 controller rev D                         plane n (core board)
 ESP32-C6 --SPI-- MCP2518FD --TCAN1044V==CANH/CANL==ISO1044--+-- MCP2518FD --SPI-- G070
                  (W5500 shares SPI)        CAN_5V    |  side2|side1          (G070 build)
                                            CAN_GND   |       +-- FDCAN1 (PB8/PB9) G0B1
                                                      |                       (G0B1 build)
 chain A <==== J_A [controller] J_B ====> chain B     |
              120R switchable                  last plane: 120R via SW1-4

 REQ  0x2E  (type 2, node 14)  ->  "read SUMMARY"           (1-2 frames)
 RSP  0x3n  (type 3, node n)   <-  L, records, crc-8        (1-5 frames)
 EVENT 0x1n                    <-  events pending, head seq (repeated 1 s until EV_ACK)
 SYNC 0x0E                     ->  keepalive, epoch, commanded states (standby, later)
```

---

## Alternatives Considered

### Keep I2C, flip the isolators
- **The idea**: turn each ISO1640 around so side 2 (shareable) faces
  the cable and side 1 faces the plane MCU alone; nothing else moves.
- **Optimizes for**: the smallest change; fixes problem 1 only.
- **Sharpest tradeoff**: still single-ended I2C on cables in the
  fluid at 100 kHz, one stuck slave still blocks the chain, and still
  one master.
- **Bets on**: short cables, a quiet tank, and no standby controller.

### Isolated RS-485, half duplex, polled
- **The idea**: UART on both MCUs (the G070 has hardware DE), an
  isolated transceiver with its own isolated supply per plane
  (CA-IS3092W, C2890051, 10680, $3.01, 500 kbit/s).
- **Optimizes for**: no new controller chips on either side; the G070
  stays as is.
- **Sharpest tradeoff**: collisions, retries and master handover are
  all firmware; a babbling driver jams the bus for good. A standby
  controller needs a listen-before-talk scheme or a second bus.
- **Bets on**: one master forever.

### Classic CAN on the ESP32-C6's TWAI
- **The idea**: no MCP2518FD on the controller; 8-byte frames.
- **Optimizes for**: one chip less on the controller.
- **Sharpest tradeoff**: SUMMARY (~100 bytes) becomes ~15 frames, the
  1 Mbit/s ceiling, and the planes still need a CAN controller. A
  classic controller also destroys FD frames, so the choice is for
  good.
- **Bets on**: blocks staying small. They do not (SUMMARY, LOG, FW).

### STM32G431CBT6 on the plane
- **The idea**: FDCAN on chip, no MCP2518FD (C529355, 9519, $4.78).
- **Optimizes for**: one chip and one CAN driver on the plane.
- **Sharpest tradeoff**: a different LQFP-48 pinout: re-pin of the
  core MCU sheet and placement, and a G0 to G4 firmware port.
- **Bets on**: the G0B1 not coming back. If that holds for months,
  this is the next step after the G070 build.

---

## Consequences

### Positive
- The isolator topology is inside the vendor's rules.
- Differential bus with +/-12 V-class common-mode range, bus-off
  confines a faulty node; a stuck transceiver releases after its
  dominant timeout.
- ~20x the useful throughput of the I2C link; firmware updates over
  the link become practical.
- Events need no side wire; arbitration orders them before bulk.
- A standby controller needs firmware only (decision 11).
- The plane takes the G0B1 the day it is in stock, no respin.

### Negative
- Two chips more per plane in the G070 build (ISO1044 replaces two
  isolators; the MCP2518FD and crystal are new): ~+$1.60 per plane.
- One chip more on the controller, sharing the W5500's SPI.
- Two CAN drivers on the plane (MCP2518FD and FDCAN), one per build.
- The cable supply moves from 3V3 to 5 V.
- Termination is a setting the user can get wrong.

### Risks
- **Loop delay through the isolator at 2 Mbit/s**: the ISO1044 adds
  delay to the round trip. Mitigation: transmitter delay compensation
  on both controllers; fall back to 1 Mbit/s data phase if the bench
  shows errors.
- **Untwisted PH cable**: CANH / CANL run as parallel wires, not a
  twisted pair. At 2 Mbit/s over a few metres this usually works;
  bench test with the longest chain. Mitigation: a twisted pair
  crimped into the PH housing, or a lower data rate.
- **Wrong termination**: no termination or three terminators.
  Mitigation: the controller firmware reads the bus error counters
  and reports "link marginal"; the manual's commissioning step
  measures 60 ohm between CANH and CANL with power off.
- **G0B1 build untested until a G0B1 is bought**: the layout is the
  same, the firmware path is not. Mitigation: build one G0B1 board
  from global-sourced parts before relying on it.

---

## What an Expert Would Ask

**Q: Why put the CAN controller behind SPI instead of buying an MCU
with FDCAN?**
A: Stock. The G0B1 (same footprint) is at 45 pieces; the G431 is in
stock but means a re-pin and a port. The MCP2518FD is the cheapest
way to get FD today without touching the MCU, and the layout keeps
the G0B1 door open.

**Q: Two connectors on the controller: is that a star?**
A: Only if both chains hang off a stub. The controller sits in the
middle of one line (A - controller - B) with the transceiver tapped
between the connectors on short traces, so it is a linear bus. Its
own terminator must be off when both chains are used and on when one
is. A switch, not automatic; the commissioning check catches it.

**Q: What happens when one plane's transceiver fails dominant?**
A: Its TXD dominant timeout releases the bus after a few ms; the CAN
controller's error counters take the node bus-off. A transceiver that
fails dominant inside the chip (not via TXD) jams the whole bus, as
on any shared bus; the controller reports "bus dead" and the planes
keep their slot states (ADR 0001: the plane holds state without a
controller).

**Q: Is CRC-8 still enough with messages up to 5 frames?**
A: Per frame, CAN FD's CRC-17 / CRC-21 is far stronger than CRC-8.
The CRC-8 only has to catch reassembly mistakes and SPI corruption,
which a dropped-segment check already catches mostly. If a field case
shows otherwise, CRC-16 is a framing change in one crate.

**Q: Does the 5 V cable supply fit the controller's budget?**
A: ISO1044 side 2 draws a few mA recessive and tens of mA while its
node drives dominant, and only one node drives at a time. For 8
planes that is in the order of 100-150 mA per chain worst case
(datasheet figure to confirm), from the controller's 5 V rail through
the load switch. Rev D budgets about 2 W from 12 V today; this adds
under 1 W.

---

## Implementation Plan

### Decisions you will probably want to tweak

- **Data rate**
  - **Choice**: 500k / 2M.
  - **Alternative**: 1M / 5M, or 250k / 1M.
  - **Cost to change later**: firmware constants only.
- **Identifier layout**
  - **Choice**: type in the upper bits, node in the low 4 bits.
  - **Alternative**: a per-node priority, or 29-bit ids.
  - **Cost to change later**: low until planes ship. After that, a
    protocol version bump on both sides.
- **Termination control**
  - **Choice**: a 4th DIP position on the plane, a switch on the
    controller.
  - **Alternative**: a solder jumper, or automatic termination when
    J3 is empty (needs a switched connector).
  - **Cost to change later**: a board rev.
- **G070 + MCP2518FD as the default build**
  - **Choice**: the G070 build.
  - **Alternative**: the G431 re-pin now.
  - **Cost to change later**: a re-pin and a firmware port, the same
    work later as now.

### Known unknowns and how the plan absorbs them

- **2 Mbit/s through ISO1044 on a PH cable chain**: default 2M with
  TDC. Pivot to 1M if the error counters climb with 8 planes on the
  bench.
- **G0B1 availability**: default G070 build. Pivot to the G0B1 build
  when LCSC holds more than one batch; to the G431 if it stays out
  through the next order.
- **CA-IS1044S as second source**: default ISO1044BDR. Use the
  CA-IS1044S only after its datasheet confirms the pinout and FD rate.

### The mechanical work

- Core board: link.py (ISO1044, termination, J2 / J3 pinout), mcu.py
  (MCP2518FD, crystal, pins), SW1 to 4 positions; regenerate the
  schematic and pin map, re-place and re-route the link corner.
- Controller rev D: the "Plane link" section and pin map, handed to
  the rev D session.
- Firmware: granite-proto segmentation and ids; plane transport trait
  with two drivers; controller MCP2518FD driver; granite-sim models
  the CAN transport.
- Docs: ADR 0001 decision 3 and ADR 0003 decisions 1 and 5 point
  here; ADR 0005 component list says CAN FD (it says "RS-485 link"
  today, stale); site pages on connections and pinouts.

### Review asks

1. CAN FD over the flipped-isolator I2C fix: yes / no?
2. G070 + MCP2518FD as the build to make now, G0B1 as the drop-in
   later: yes / no (or the G431 re-pin now)?
3. 5 V on the link cable: yes / no?
4. Termination on a 4th DIP position: yes / or a solder jumper?

---

## Open Questions

**Architecture-changers**
- [ ] Does controller rev D take the MCP2518FD on the W5500 SPI, or
      does it need its own SPI pins? (rev D session)

**Behavior definers**
- [ ] SYNC period and the standby's takeover timeout T (later ADR).
- [ ] Plane behaviour on bus-off: auto recovery after 128 x 11
      recessive bits (CAN default) and a counter in STATUS, or stay
      off until reset. Default: auto recovery and count.
- [ ] ESD part for CANH / CANL on both boards (LCSC stock check).
