# Plane link transport over CAN FD

**Status**: Accepted (2026-11-04)
**Date**: 2026-11-04
**Updated**: 2026-11-04 (open questions answered by the user: EVENT stays a bare flag, bus-off auto recovery); 2026-11-04 (as built: granite-proto::can 7c39a71, plane-fw 8d898e6, mcp2518fd 0af8d56 as a facade over mcp251xfd, granite-core / sim 7319d12); 2026-11-06 (as built, controller side: `granite_core::plane::can_link::CanPlaneLink` over a frame-driver trait, granite-fw features `link-canfd` / `link-i2c` / `link-bench`, MCP2518FD on the W5500's SPI bus, link kind in the image and the OTA guard. See "As built: the controller side" below)

Related: docs/adr/0006-plane-link-can-fd.md (hardware: ISO1044 per
core, MCP2518FD on the G070 or FDCAN on the G0B1, MCP2518FD on the
controller, identifiers, segmentation), firmware/docs/adr/0003-plane-link-protocol.md
(blocks, tags, records, CRC-8: unchanged), firmware/docs/adr/0002-plane-core-firmware.md
(plane firmware), firmware/docs/adr/0004-controller-rev-d-node-model.md
(controller side, `PlaneLink`).

---

## Context

Rev D keeps the isolated I2C link (ADR 0003). The FD generation
(power/pp-48-core-fd, pp-48-unit-fd, controller-fd) moves the same
messages onto CAN FD. Everything above the link already works on
whole messages: `plane_logic::Plane::handle_frame(bytes, out)` on the
plane, `granite_core::hal::PlaneLink::write_read / write` on the
controller. This ADR fixes the transport between them so that both
builds share one protocol and one code base, and so the parts can be
written and tested on the host before the boards exist.

---

## Decision

1. **Messages are ADR 0003 frames, unchanged.** A read request is
   `block, index`; a write is `block, index, L, records, crc`; an
   answer is `L, records, crc`. The CRC-8 still covers the 7-bit
   address: on CAN the address is `0x30 + node`, so the plane's CRC
   code does not change.

2. **Identifiers** (11 bit): `id = (type << 4) | node`.
   - node 0-7: plane (its address straps), 14: controller, 15: standby
     controller. Others reserved.
   - type 0 SYNC, 1 EVENT, 2 REQ, 3 RSP, 4 BULK; 5-15 reserved. Lower
     id wins arbitration, so the order is the priority.
   - REQ / RSP carry every block except FW (0x13) and LOG (0x08),
     which travel as BULK in both directions. The node field always
     names the plane the message is to or from; the direction follows
     from the sender (a controller sends, a plane answers).

3. **Segmentation.** A message is cut into CAN FD frames. Byte 0 of
   every frame: bits 0-6 segment index (0, 1, 2 ...), bit 7 set on the
   last segment. Up to 63 message bytes follow. A frame is padded with
   0x00 to the next valid CAN FD length (0-8, 12, 16, 20, 24, 32, 48,
   64); the message length comes from its own content (ADR 0003 L),
   so padding is ignored. A message of up to 258 bytes (L <= 254)
   takes at most 5 segments.

4. **Reassembly.** One message in flight per (type, node). A segment
   index that is not the next one, or a gap of more than 50 ms
   between segments, drops the partial message and counts it. A
   segment 0 always starts a new message. No allocation: fixed buffers
   of 320 bytes. The index byte carries no message identity: a message
   that loses its segment 0 within 50 ms of a partial of the same id
   splices onto it; ADR 0003's CRC-8 refuses the result (pinned by a
   test in granite-proto::can).

5. **Transactions.**
   - Read: controller sends REQ (or BULK for LOG) to node n, waits for
     the RSP (or BULK) from node n, timeout 20 ms after the last REQ
     segment, then ADR 0003's retry rules.
   - Write: controller sends REQ (or BULK for FW); no answer frame, the
     outcome is read back from STATUS as today.
   - A plane answers only REQ / BULK addressed to its own node.

6. **Events replace EXP_INT.** A plane with unacknowledged events
   sends EVENT from its node, payload one byte `0x01`, when the first
   one is raised and again every 1 s until EV_ACK covers them all.
   `PlaneLink::int_asserted() -> bool` becomes
   `PlaneLink::event_nodes() -> u8`: a bit mask of planes that sent
   EVENT since the last call. The I2C implementation returns 0xFF
   while EXP_INT is low (any plane may be the one) and 0 otherwise;
   the client reads STATUS of the planes in the mask.

7. **SYNC.** The controller sends SYNC from node 14 once per second,
   payload `epoch: u32` little endian (random at boot, +1 per send).
   Planes ignore it for now; it is the keepalive a standby controller
   (node 15) will watch (hardware ADR 0006 decision 13).

8. **Bit timing and errors.** 500 kbit/s nominal (80 % sample point),
   2 Mbit/s data (75 %), transmitter delay compensation on, 40 MHz
   CAN clock on both the MCP2518FD and the G0B1 FDCAN. Bus-off:
   automatic recovery, counted. The plane reports CAN error counters
   in STATUS under a new tag (next free number in granite-proto's tag
   table); the controller keeps its own in its diagnostics.

9. **Crates.**
   - `granite-proto::can` (no_std, no alloc): id encode / decode,
     `MsgType`, a segmenter (message -> frames) and a reassembler
     (frames -> message) as above. Host tests and a fuzz-style test
     (no panic on any frame sequence).
   - `mcp2518fd` (new crate, no_std): a thin facade over the
     maintained `mcp251xfd` crate (embedded-hal 1.0 `SpiDevice`,
     errata work-arounds), fixing this link's one configuration; used
     by both the plane (G070 build) and the controller. INT0 / INT1:
     inputs on the plane (pulled up), outputs driven low on the
     controller (open there). Configure (clock, bit timing, TDC, one TX FIFO, one RX
     FIFO, accept filter), send, receive, interrupt status, error
     counters, bus-off. Host tests against a mock SPI.
   - `granite-core`: `PlaneLink::event_nodes`, the client reads the
     planes in the mask; SYNC sender.
   - `granite-sim`: a CAN link fake that segments and reassembles
     through `granite-proto::can`, with frame loss injection.
   - Plane binary `plane-fw` (new, outside the workspace like
     granite-fw: thumbv6m, embassy-stm32, STM32G070CB): builds the
     rev D link (I2C1 slave) first; the CAN transports (MCP2518FD,
     FDCAN) are features of the same binary.

---

## As built: the controller side (2026-11-06)

Nothing below changes a decision; it records where the decisions
landed and the three places the code is narrower or wider than the text.

- **The transport is `granite_core::plane::can_link`**
  (`CanPlaneLink<D>`), a `PlaneLink` over a small frame-driver trait
  `CanFrames`: send one frame with an 11-bit id, take one received
  frame, `now_ms`, read the error counters, `idle` between polls. So
  decisions 2-8 are host-tested against a queue-backed fake driver (17
  tests: read, write, BULK both ways, timeout, lost segment, EVENT
  during a transaction, an answer from the wrong node, a full TX FIFO,
  bus-off edges, 5000 random frames without a panic), and the firmware
  only owns the chip.
- **A timeout is `LinkError::Timeout`**, which `plane::link` retries
  (ADR 0003's rules); `LinkError::Nak` stays "no plane strapped there"
  and is what an address outside 0x30-0x37 gets, without a frame on the
  bus. Nothing on CAN acknowledges a frame, so a plane that never heard
  the request and one that did not answer are the same event.
- **Frames that are not the answer are sorted, not dropped blindly**:
  an EVENT from a plane sets its bit in the `event_nodes` mask even in
  the middle of a transaction (decision 6), a foreign SYNC or REQ and
  an answer from another node are counted as `foreign`, and the receive
  FIFO is pumped before every transaction so a leftover answer cannot
  be read as this one's. Counters for all of it, plus the
  reassembler's own drop reasons and TEC / REC, on the console's `link`
  command (decision 8: "the controller keeps its own in its
  diagnostics").
- **The controller's acceptance filter is promiscuous**, not
  `Config::for_node(14)`: the node field of an identifier always names
  the *plane* (decision 2), so every answer and every EVENT the
  controller must read has a plane's number in it. A node filter of 14
  would accept nothing a plane sends. `mcp2518fd::Config::promiscuous`
  exists for this.
- **SYNC goes through the segmenter** like every other message, so the
  one frame is `[0x80, epoch LE]` and a receiver has one code path
  (this is what granite-sim's CAN fake already did). A standby
  controller's link would send it from node 15; nothing builds that.
- **Polling, not the interrupt pin**: nINT (GPIO3) is claimed as an
  input and read only for the diagnostics. The dispatcher is a 50 ms
  loop and a transaction waits at most 20 ms, so the transport polls the
  receive FIFO every 200 us (a busy wait: `CONFIG_FREERTOS_HZ` is 100,
  so the shortest task delay is 10 ms and would give a transaction two
  polls).
- **granite-sim keeps its own `CanBus`** (commit 7319d12) instead of
  running `CanPlaneLink` on the controller side. Its fake already
  segments and reassembles through `granite_proto::can` on both sides
  and adds frame-loss injection; putting the real transport in would
  mean the driver owning the fake planes that `FakePlaneLink` owns
  today - a cyclic-ownership refactor of granite-sim for no new
  coverage of the transport, which is tested directly. Left alone
  deliberately.
- **granite-fw**: features `link-canfd` (default) and `link-i2c`,
  exactly one, `compile_error!` otherwise (ADR 0006 component 7). The
  MCP2518FD is a second `SpiDeviceDriver` on the W5500's bus at 10 MHz
  (the bus is built once, before either device), nCS GPIO2, nINT
  GPIO3, the TCAN1044V's STB on GPIO0 driven low, INT0 / INT1 driven
  low through `IOCON` (the 32-bit write clears LAT0 / LAT1 by erratum,
  which is the level the board wants). An FD build opens no HP I2C bus
  and reads the TMP1075 over LP I2C, because that is where
  controller-fd puts it.
- **One image per link, enforced twice**: the image carries its link
  kind as a tagged 24-byte marker in its read-only data
  (`granite_core::link_kind`), the OTA upload refuses an image of the
  other kind or without a marker (HTTP 400, the path a hash mismatch
  already uses), and an image whose link hardware never came up does
  not pass its probation ladder, so it rolls back. The bench feature
  `link-bench` only logs that last one.
- **Untested on hardware.** No controller-fd board exists. The image
  builds (1,706,432 B of the 0x3D0000 slot, 42.7 %, against 1,654,816 B
  before) and the transport is host-tested; the chip is not.

---

## Consequences

### Positive
- One protocol and one plane core for rev D and FD; only the
  transport differs.
- The CAN parts are testable on the host and in the sim before any
  board exists.

### Negative
- `PlaneLink` changes signature (`int_asserted` -> `event_nodes`): the
  rev D stub, the sim fakes and the client change together.
- Two CAN drivers on the plane (MCP2518FD, FDCAN).

### Risks
- MCP2518FD and the W5500 share the controller's SPI bus: the
  ESP-IDF Ethernet driver must allow a second device on its host.
  Check when granite-fw gets the driver; fallback is the second SPI
  pin set (none free today, needs a rev).

---

## Open Questions
- [x] EVENT payload richer than 0x01? No (user 2026-11-04): a bare
      flag, the controller reads STATUS then EVENTS as on rev D; one
      code path for both builds.
- [x] Bus-off: automatic recovery, counted (user 2026-11-04;
      decision 8).
