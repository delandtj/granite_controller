# Plane link transport over CAN FD

**Status**: Accepted (2026-11-04)
**Date**: 2026-11-04
**Updated**: 2026-11-04 (open questions answered by the user: EVENT stays a bare flag, bus-off auto recovery); 2026-11-04 (as built: granite-proto::can 7c39a71, plane-fw 8d898e6, mcp2518fd 0af8d56 as a facade over mcp251xfd, granite-core / sim 7319d12)

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
