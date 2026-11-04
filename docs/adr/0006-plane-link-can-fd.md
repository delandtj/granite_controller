# Plane link over isolated CAN FD, through the plane row

**Status**: Proposed
**Date**: 2026-11-04
**Updated**: 2026-11-04 (user: no cables between the controller and the
planes either; planes sit in one row on the bar, plane n+1's core
plugs onto plane n's last unit, the link passes through every unit;
was: PH 5p cables, controller in the middle of two chains)

Related: docs/adr/0001-48v-power-plane.md (decision 3, "one message
format over isolated I2C"; the FDCAN note under review ask 3),
docs/adr/0005-chained-plane-boards.md (core and unit boards, the 2x7
chain joint, the mounting bar), firmware/docs/adr/0003-plane-link-protocol.md
(framing, blocks, polling), docs/controller-rev-d.md ("Plane link",
"12 V", pin map).

---

## Context

Today (boards as routed, 2026-11-04):

- Controller rev D: two JST PH 5p link connectors (J13 / J14: CTRL_3V3,
  CTRL_GND, SCL, SDA, EXP_INT) and two PH 2p 12 V inputs (J11 / J12),
  cabled to the planes.
- Core: PH 5p CTRL IN / OUT (J2 / J3) and PH 2p 12V AUX OUT (J4) on
  its top edge; ISO1640 (I2C) and ISO7721 (EXP_INT) with side 1 on
  the cable; the 2x7 chain socket J301 on its right edge.
- Units: 2x7 chain plug J301 (left) and socket J302 (right); no
  controller link.

The user drops the cables (2026-11-04): the planes of a box stand in
one row on the mounting bar, core 1, its units, core 2, its units, and
so on, every board plugged into the next. The controller plugs onto
the left end of the row. So the link has to pass through every unit.

Problems with keeping the current link on that row:

1. **Shared ISO1640 side 1 is outside TI's rules.** Side 1 is meant
   for one node; only side 2 may be shared. Datasheet: side 1 load
   80 pF max, side 1 output low 570-710 mV (the ESP32-C6 reads it with
   VIL max 0.825 V, ~0.1 V margin), and another side 1's input
   threshold is only guaranteed 50 mV under its own output low. One
   plane is in spec; several on one bus are not.
2. **Bus length**: 8 planes of a core and 4 units are ~8 x 375 mm,
   ~3 m of board trace through ~45 joints. I2C on side 2 (if the
   isolators were flipped) allows 400 pF; 3 m of trace plus 8
   isolators reaches that. One stuck SDA still stops every plane.
3. **Throughput**: ~10 kB/s shared by 8 planes, plus a side wire for
   events; firmware updates over the link are slow.
4. **One master only**: a standby controller that takes over during
   master upgrades cannot share an I2C bus.

Constraints: parts from JLCPCB stock; the plane MCU is the
STM32G070CBT6 (no CAN), the STM32G0B1 (FDCAN, same LQFP-48 pinout) is
out of stock (2026-11-04: G0B1CBT6 0, CCT6 45, CET6 0); the ESP32-C6's
TWAI is classic CAN only. Nothing of the core, units or rev D is
ordered (handoff "Boards ordered": rev C only).

---

## Decision

1. **Physical layer: CAN FD**, 500 kbit/s arbitration, 2 Mbit/s data
   phase, 11-bit identifiers. One linear bus along the row: controller
   at the left end, the last board of the row at the right end.

2. **Isolation at each core** with an isolated CAN FD transceiver,
   **ISO1044BDR** (C2878225, SOIC-8, 6478 in stock, $1.87; 5 Mbit/s
   FD, side 1 1.71-5.5 V, side 2 4.5-5.5 V, 3 kVrms basic). Second
   source CA-IS1044S (C7424699, 4580, $1.17), pinout to confirm. The
   ISO1640, ISO7721 and EXP_INT go. The link domain (LINK_GND, CANH,
   CANL, AUX_12V) is one isolated net set along the row, shared with
   the controller; it never touches the bus ground. Units carry it
   through without parts.

3. **Link connector: a separate 2x3 right-angle plug / socket beside
   the 2x7 chain joint**, same family as the chain (CJT A2541, 2.54 mm):
   plug A2541WR-2x3P (C239334, 10210 in stock, $0.12), socket
   A2541HWR-2x3P (C5333425, 1983, $0.16).

   | Board | Left edge | Right edge |
   |---|---|---|
   | Controller | - | link socket |
   | Core | link plug (IN) | link socket (OUT), beside chain socket J301 |
   | Unit | link plug, beside chain plug J301 | link socket, beside chain socket J302 |

   Pinout, pin n to pin n as on the chain: 1 CANH, 2 CANL, 3 LINK_GND,
   4 LINK_GND, 5 AUX_12V, 6 AUX_12V. Same height and face rules as the
   chain (boardfit.link), so one push mates both joints.

   Plane n+1's core has no 2x7 on its left edge: plane n's chain
   (I2C segments, 3V3, GND, EXP_RSTN, PRESS_EN) ends at its last
   unit's J302, unmated. Planes share nothing but the link.

4. **12 V aux on the link**: each core feeds its isolated 12 V module
   output onto AUX_12V through a diode (OR). The controller takes its
   12 V from the link (replaces J11 / J12 and the core's J4). A core
   whose module fails costs nothing while one other core runs.

5. **ISO1044 side 2 supply is local**: each core makes 5 V on the link
   domain from its own isolated 12 V (small LDO, ~70 mA peak while its
   node drives dominant). No 5 V on the link, no load switches on the
   controller.

6. **Plane CAN controller: either MCU, one layout.**
   - **G070 build** (default while the G0B1 is out of stock):
     STM32G070CBT6 + **MCP2518FDT-E/SL** (C626758, SOIC-14, 19157,
     $1.51) on SPI1, 40 MHz crystal (X322540MPB4SI, C9010, 68172,
     $0.11).
   - **G0B1 build**: STM32G0B1CCT6 in the same footprint, FDCAN1 on
     PB8 / PB9; MCP2518FD and its crystal not fitted.
   - ISO1044 RXD drives both the MCP2518FD RXCAN and PB8; ISO1044 TXD
     is driven by the MCP2518FD TXCAN or PB9, never both (the G0B1
     build does not fit the MCP2518FD, the G070 firmware leaves PB9 in
     its reset state, analog high-Z). No jumpers.

7. **Plane MCU pins** (LQFP-48, G070 and G0B1 identical for these):

   | Pin | Port | Was | Becomes |
   |---|---|---|---|
   | 47 | PB8 | SCL1 (I2C1 slave, ISO1640) | CAN_RX: ISO1044 RXD (FDCAN1_RX AF3 on the G0B1) |
   | 48 | PB9 | SDA1 (I2C1 slave, ISO1640) | CAN_TX: ISO1044 TXD (FDCAN1_TX AF3 on the G0B1) |
   | 15 | PA4 | unused | SPI1_NSS: MCP2518FD nCS |
   | 16 | PA5 | unused | SPI1_SCK |
   | 17 | PA6 | unused | SPI1_MISO: MCP2518FD SDO |
   | 18 | PA7 | unused | SPI1_MOSI: MCP2518FD SDI |
   | 11 | PA0 | unused | MCP_INT (MCP2518FD nINT, EXTI) |
   | 40 | PD2 | INT_OUT (EXP_INT via ISO7721) | free |

   Pin 10 is NRST on the G070 and PF2-NRST on the G0B1 (reset by
   default). Pins 29 / 32 are bonded on the G0B1 only (PA9 / PA10,
   UCPD dead-battery pull-downs at reset) and stay unconnected. Source:
   stm32-metapac 21 (G0B1CC alternate functions), KiCad
   MCU_ST_STM32G0 symbols. The address straps ADR0-2 become the node
   id.

8. **Termination**: the controller always terminates the left end
   (split, 2 x 60R4 + 4.7 nF to LINK_GND). The right end: a split
   termination on every board's link socket side (core and unit),
   behind a solder jumper, open by default; closed on the last board
   of the row. A standby controller at the right end (decision 13)
   terminates instead.

9. **Controller rev D**:
   - **MCP2518FD on the W5500's SPI bus** (GPIO19 / 20 / 21), nCS on
     GPIO2, nINT on GPIO3 (freed by the I2C link); GPIO0 freed by
     EXP_INT. The MCP2518FD's SPI limit (~17 MHz at 40 MHz SYSCLK)
     applies to its own transactions; esp-hal sets the clock per
     device.
   - **TCAN1044VDRQ1** (C1852061, 15487, $0.46), VIO 3V3, VCC 5 V, not
     isolated: the controller's ground is LINK_GND.
   - One link socket on its right edge, at the bar height of the core's
     link plug; the controller stands on the bar left of core 1.
     J11-J14 go; 12 V comes from AUX_12V (decision 4).
   - ESD on CANH / CANL at the socket (part to pick, 2-line CAN TVS).

10. **Message layer: blocks and tags of firmware ADR 0003 unchanged.**
    The transport changes:
    - **Identifiers** (11 bit): `id = (type << 4) | node`. Node 0-7 =
      plane (address straps), 14 = controller, 15 = standby
      controller. Types, in priority order:

      | Type | Name | From | Content |
      |---|---|---|---|
      | 0 | SYNC | controller | master keepalive / broadcast (decision 13) |
      | 1 | EVENT | plane | "events pending" + head seq; replaces EXP_INT |
      | 2 | REQ | controller | read request or write (block, index, L, records, crc) |
      | 3 | RSP | plane | response (L, records, crc) |
      | 4 | BULK | either | FW and LOG segments, lowest priority |

    - **Segmentation**: a request or response is the ADR 0003 byte
      string cut into CAN FD frames of up to 64 bytes. Byte 0 of each
      frame = segment index (bits 0-6) + last flag (bit 7); 63 payload
      bytes per frame, so L <= 254 takes at most 5 frames. A receiver
      drops a message on a gap in the index.
    - **CRC-8 stays**, end to end over the reassembled message, with
      the node id in place of the I2C address.
    - **Events**: a plane sends an EVENT frame when it raises an event
      and repeats it every 1 s until an EV_ACK covers the head seq.
    - **Polling** unchanged in content (STATUS + SUMMARY per plane per
      second): ~150 bytes = 3 frames, ~1 ms per plane, 8 planes ~1 %
      of the bus.

11. **Bit timing**: G0B1 FDCAN kernel clock 40 MHz from PLLQ (8 MHz
    HSE); MCP2518FD SYSCLK 40 MHz. Sample point 80 % nominal, 75 %
    data; transmitter delay compensation on.

12. **Layout rules for the link on the units**: CANH / CANL as a
    differential pair (In1 or B, the reference plane under it on the
    link domain or none: never bus GND, which is not the pair's
    return), LINK_GND and AUX_12V beside it; clearance to bus nets as
    for functional isolation at <= 60 V (0.5 mm on outer layers).

13. **Standby controller: designed for, not built.** It plugs into the
    last board's link socket at the right end of the row (and
    terminates it). SYNC (type 0, highest priority) is the master's
    keepalive with an epoch and the commanded slot states. The standby
    (node 15) listens; with no SYNC for T it polls and publishes
    monitoring only, never CTRL writes, until the master's SYNC
    returns. Details in a later ADR; this one reserves the ids and the
    right-end position.

---

## Architecture Overview

### Component Breakdown

1. **Core link block** (power/pp-48/tools/sch/blocks/link.py)
   - ISO1044 U4, 12 V -> 5 V LDO on the link domain, OR diode from the
     12 V module onto AUX_12V, right-end split termination with its
     solder jumper.
   - Link plug (left edge) and socket (right edge), decision 3. J2, J3,
     J4 and U5 go.
2. **Core MCU block** (power/pp-48/tools/sch/blocks/mcu.py)
   - MCP2518FD U5 with its 40 MHz crystal on SPI1 and PA0, "G070
     build only" in its description.
   - CORE_PINS / PIN_USE per decision 7; firmware/plane-core-pins.toml
     regenerated.
3. **Unit** (power/pp-48/tools/sch/blocks/unit.py, chain.py)
   - Link plug and socket beside J301 / J302, four nets straight
     through, the split termination with its solder jumper.
4. **Controller rev D** (docs/controller-rev-d.md, the rev D session)
   - MCP2518FD on the W5500 SPI, TCAN1044V, fixed termination, one
     link socket, 12 V from AUX_12V.
5. **granite-proto** (firmware/granite-proto)
   - Framing gains `segment` / `reassemble` and the id helpers; blocks,
     tags, records and CRC-8 unchanged. Still no I/O.
6. **plane-logic link** (firmware/plane-logic/src/link.rs)
   - A transport trait (receive message, send message, send event)
     with two drivers: FDCAN (embassy-stm32, G0B1) and MCP2518FD
     (SPI), chosen at build time by the fitted MCU.
7. **Controller firmware** (firmware/granite-fw)
   - MCP2518FD driver on the shared SPI, the same transport, EVENT
     frames in place of the EXP_INT GPIO.

### Data Flow / Interaction

```
 bar:  [controller]=[core 1]=[unit]=[unit]=[unit]=[unit]=[core 2]=[unit]= ... =[last unit]
         120R        ISO1044  ----- link passes through -----  ISO1044          SJ: 120R
                     2x7 chain to its units ------------->|    2x7 chain ...  (or standby)
                                                  (J302 of plane 1's last unit stays unmated)

 link (2x3): CANH, CANL, LINK_GND x2, AUX_12V x2       one isolated domain along the row
 per core:   12 V iso module -> diode -> AUX_12V        5 V LDO -> ISO1044 side 2
 controller: AUX_12V -> its 5 V / 3V3; MCP2518FD + TCAN1044V

 REQ   0x2E (type 2, node 14) ->  "read SUMMARY"            (1-2 frames)
 RSP   0x3n (type 3, node n)  <-  L, records, crc-8         (1-5 frames)
 EVENT 0x1n                   <-  events pending, head seq  (every 1 s until EV_ACK)
 SYNC  0x0E                   ->  keepalive, epoch, states  (standby, later)
```

---

## Alternatives Considered

### Keep I2C, flip the isolators
- **The idea**: each ISO1640 turned so side 2 (shareable, 400 pF)
  faces the row bus and side 1 the plane MCU alone; the same 2x3
  joint carries SCL, SDA, EXP_INT, 3V3, GND.
- **Optimizes for**: the fewest new parts; the G070 stays alone.
- **Sharpest tradeoff**: 3 m of board trace and 8 isolators on one
  I2C bus is at the 400 pF limit; single ended, one stuck slave
  blocks the row, one master, ~10 kB/s.
- **Bets on**: rows of 2-3 planes, never 8, and no standby controller.

### Link pins inside a larger chain connector (2x10)
- **The idea**: one joint per board edge; the chain grows from 2x7 to
  2x10 with the link on the extra pins.
- **Optimizes for**: one connector per edge, one mating action.
- **Sharpest tradeoff**: the 2x10 plug has 168 in stock (C5454488);
  the routed unit chain band is redone; plane n's chain signals meet
  plane n+1's core plug (pins left open there); the isolated domain
  shares a housing with bus ground.
- **Bets on**: the 2x10 plug staying stocked.

### Isolated RS-485, half duplex, polled
- **The idea**: UART on both MCUs (the G070 has hardware DE), an
  isolated transceiver with its own isolated supply per core
  (CA-IS3092W, C2890051, 10680, $3.01, 500 kbit/s).
- **Optimizes for**: no new controller chips; the G070 stays as is.
- **Sharpest tradeoff**: collisions, retries and master handover are
  all firmware; a babbling driver jams the bus for good.
- **Bets on**: one master forever.

### Classic CAN on the ESP32-C6's TWAI
- **The idea**: no MCP2518FD on the controller; 8-byte frames.
- **Optimizes for**: one chip less on the controller.
- **Sharpest tradeoff**: SUMMARY (~100 bytes) becomes ~15 frames,
  1 Mbit/s ceiling, and a classic controller destroys FD frames, so
  the choice is for good.
- **Bets on**: blocks staying small. They do not (SUMMARY, LOG, FW).

### STM32G431CBT6 on the core
- **The idea**: FDCAN on chip, no MCP2518FD (C529355, 9519, $4.78).
- **Optimizes for**: one chip and one CAN driver on the plane.
- **Sharpest tradeoff**: another LQFP-48 pinout: re-pin and re-place
  the core MCU sheet, a G0 to G4 firmware port.
- **Bets on**: the G0B1 not coming back.

---

## Consequences

### Positive
- No cables between the controller and the planes, or between planes:
  the whole box is one row of boards on the bar.
- The isolators are used inside the vendor's rules; one isolated
  domain along the row, carrying no bus current.
- Differential bus with bus-off fault confinement; a stuck transceiver
  releases after its dominant timeout.
- ~20x the useful throughput of the I2C link; firmware updates over
  the link become practical.
- The controller's 12 V is OR'ed from every core (was two cables).
- A standby controller needs no new board design and has a natural
  place (the right end).
- The core takes the G0B1 the day it is in stock, no respin.

### Negative
- Every unit carries the link: two more connectors and four nets
  across a routed 4-layer board; a respin of the unit and its route.
- Per core in the G070 build: ISO1044, MCP2518FD, crystal, LDO, OR
  diode (~+$2 per core); per unit two connectors and a termination
  (~$0.30).
- Two CAN drivers on the plane (MCP2518FD and FDCAN), one per build.
- A row is one mechanical unit: replacing a board in the middle means
  sliding its neighbours apart (true already within a plane).
- Termination is a solder jumper the user must close on the right
  board.

### Risks
- **Loop delay at 2 Mbit/s through the ISO1044**: transmitter delay
  compensation on both controllers; 1 Mbit/s data phase as fallback.
- **~45 joints and ~3 m of trace**: each joint is a small impedance
  step. Bench test the longest row; 1 Mbit/s data phase if the error
  counters climb.
- **Wrong termination**: none or three. The controller reads the
  error counters and reports "link marginal"; the commissioning step
  measures 60 ohm between CANH and CANL with power off.
- **Plug stock**: the 2x3 plug and socket are plentiful; the 2x7
  chain plug C5136987 already has only 87 in stock, worth a check
  before ordering (not this ADR's part, but the same joint).
- **G0B1 build untested until a G0B1 is bought**: build one board from
  global-sourced parts before relying on it.

---

## What an Expert Would Ask

**Q: Why not isolate once, at the controller, and run the bus on bus
ground through the row?**
A: The cores already share the bus ground through their units'
supply wiring, and the 48 V return currents of a plane (tens of A)
make that ground differ along the row by tens to hundreds of mV. A
signal ground on bus GND across 45 joints would carry part of that
return current through 2.54 mm pins. With an isolated link domain no
bus current flows in the link, and CAN's common-mode range is not
needed for it.

**Q: What does plane n's last unit do with its unmated chain socket?**
A: Nothing: J302 of the last unit of every plane is unmated already
today. Its pins are 3V3, GND and I2C of plane n; the next core's left
edge has only the link plug, so they stay open.

**Q: What happens when one core's transceiver fails dominant?**
A: Its TXD dominant timeout releases the bus after a few ms; the CAN
controller's error counters take the node bus-off. A transceiver
that fails dominant inside the chip jams the whole bus, as on any
shared bus; the controller reports "bus dead" and every plane keeps
its slot states (ADR 0001: the plane holds state without a
controller).

**Q: Is CRC-8 still enough with messages up to 5 frames?**
A: CAN FD's own CRC-17 / CRC-21 guards each frame far better than
CRC-8. The CRC-8 covers reassembly and the SPI hop; the segment
index check catches a lost frame. If a field case shows otherwise,
CRC-16 is a change in one crate.

**Q: Can the AUX_12V OR carry the controller with one core left?**
A: Each core's isolated module is the 6 W class budgeted for the
controller alone in ADR 0001; the controller needs ~2 W, the cores'
LDOs a few hundred mW. One working core is enough.

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
  - **Cost to change later**: low until planes ship; then a protocol
    version bump on both sides.
- **Separate 2x3 link joint**
  - **Choice**: 2x3 beside the 2x7.
  - **Alternative**: 2x10 chain with the link inside.
  - **Cost to change later**: a respin of core and unit.
- **Right-end termination**
  - **Choice**: solder jumper on every board's socket side.
  - **Alternative**: a terminator plug (tiny 2x3 board), or a DIP on
    the units.
  - **Cost to change later**: a board rev; a terminator plug can be
    added at any time without one.
- **G070 + MCP2518FD as the default build**
  - **Choice**: the G070 build.
  - **Alternative**: the G431 re-pin now.
  - **Cost to change later**: the same re-pin and port as now.

### Known unknowns and how the plan absorbs them

- **2 Mbit/s along a full row**: default 2M with TDC. Pivot to 1M if
  the error counters climb with 8 planes on the bench.
- **G0B1 availability**: default G070 build. Pivot to the G0B1 build
  when LCSC holds more than one batch; to the G431 if it stays out
  through the next order.
- **Where the 2x3 sits on each edge**: default above the 2x7, clear of
  the unit's XT60 at the top edge and the core's U4 corner. Pivot if
  the unit route cannot take four more nets there.
- **CA-IS1044S as second source**: use only after its datasheet
  confirms the pinout and FD rate.

### The mechanical work

- Core: link.py (ISO1044, LDO, OR diode, termination, link plug and
  socket; J2 / J3 / J4 / U5 out), mcu.py (MCP2518FD, crystal, pins);
  regenerate schematic and pin map; re-place the left half and the
  link corner, re-route.
- Unit: link plug / socket and termination in unit.py / chain.py;
  re-place both edges, route the four link nets (decision 12).
- boardfit.link() for the 2x3 (front offsets of the 2x3 footprints,
  as for the 2x7).
- Controller rev D: the "Plane link" and "12 V" sections, pin map and
  board edge, handed to the rev D session.
- Firmware: granite-proto segmentation and ids; plane transport trait
  with two drivers; controller MCP2518FD driver; granite-sim models
  the CAN transport.
- Docs: ADR 0001 decision 3, ADR 0003 decisions 1 and 5, ADR 0005
  decisions 4 / 7 (the link joint beside the chain), rev D doc, site
  pages (connections, pinouts, planes).

### Review asks

1. CAN FD over the flipped-isolator I2C: yes / no?
2. Separate 2x3 link joint beside the 2x7 (not a 2x10 chain): yes / no?
3. G070 + MCP2518FD now, G0B1 as the drop-in later: yes / no (or the
   G431 re-pin now)?
4. Right-end termination: solder jumper on every board, or a
   terminator plug?

---

## Open Questions

**Architecture-changers**
- [ ] Controller rev D on the bar: its board (105 x 50 mm) must put
      its link socket at the core's link-plug height; does it stand
      on the bar like a core, or hang off a bracket? (rev D session)

**Behavior definers**
- [ ] SYNC period and the standby's takeover timeout T (later ADR).
- [ ] Plane behaviour on bus-off: auto recovery after 128 x 11
      recessive bits (CAN default) and a counter in STATUS, or stay
      off until reset. Default: auto recovery and count.
- [ ] ESD part for CANH / CANL on the core and the controller (LCSC
      stock check).
