# mcp2518fd

The MCP2518FD CAN FD controller as the Granite plane link uses it: one
clock, one bit timing, one TX FIFO, one RX FIFO, one acceptance filter.
`no_std`, no alloc, blocking `embedded-hal` 1.0 `SpiDevice`.

Both ends of the link use this crate. On the plane it is the G070 build's
controller on SPI1 (hardware ADR 0006 decision 6); on the controller the
same chip shares the W5500's SPI bus (decision 9). Sharing needs nothing
from here: `SpiDevice` owns the chip-select framing, so the bus manager
decides who talks.

## What it is

A thin facade over [`mcp251xfd`](https://crates.io/crates/mcp251xfd)
0.2. That crate owns the register map, the SPI instruction layer and the
silicon errata work-arounds; this one fixes the configuration firmware
ADR 0005 decision 8 asks for and narrows the API to the link's needs.
Writing a second register-level driver would have duplicated work that
is already cited to the datasheet, already errata-aware and already
better tested than a fresh one could be in a day.

Fixed, not configurable: 40 MHz crystal (no PLL, no divider), 500 kbit/s
nominal at an 80 % sample point, 2 Mbit/s data phase at 75 %, SJW =
TSEG2, transmitter delay compensation in auto mode, ISO CRC, BRS on every
transmitted frame, RX timestamps off, one 8-deep TX FIFO and one 8-deep
RX FIFO with 64-byte payloads (1152 of the 2048 bytes of message RAM).
`Config` carries only the acceptance filter: a standard-id match value
and mask, with `Config::for_node(n)` for the `id = (type << 4) | node`
scheme of ADR 0005 decision 2.

The bit timing is derived, not typed in: `split()` computes TSEG1/TSEG2
from a bit rate and a sample point at BRP 1, and the tests assert that it
reproduces the ADR's numbers exactly (500 kbit/s -> 80 TQ, TSEG1 63,
TSEG2 16, SJW 16; 2 Mbit/s -> 20 TQ, TSEG1 14, TSEG2 5, SJW 5, TDCO 14).

## API

`Mcp2518fd::new(spi)`, `reset()`, `init(&Config, delay)`,
`send(id, data)`, `receive() -> Option<Frame>`, `interrupt_flags()`,
`error_counters() -> (tec, rec)`, `is_bus_off()`, `inner()` for the
wrapped driver, `release()` for the SPI device back.

`send` sets the DLC from the payload length and zero-pads up to the next
valid CAN FD length; a full transmit FIFO is `Error::WouldBlock` and
nothing was queued, so the call can simply be retried. `receive` returns
`Ok(None)` on an empty FIFO rather than an error.

`Frame` is `{ id: u16, len: u8, data: [u8; 64] }` with `payload()` for
the first `len` bytes.

## Deliberate omissions

**No INT0 / INT1 as GPIO outputs.** Both boards pull those pins up (the
core's R35 / R36, the controller's 10k), so their power-on state -- GPIO
inputs -- already leaves them defined, and driving them would only fight
the pull-up. The controller schematic note "firmware sets INT0 / INT1 as
outputs" (`controller-fd/tools/gen_canlink.py`) predates that reading and
is worth revisiting rather than implementing. The pins are unused on both
designs.

Reaching them would need `IOCON` byte writes anyway: a 32-bit write to
`IOCON` clears `LAT0` / `LAT1` (erratum DS80000789F item 5), and
`mcp251xfd` keeps its single-byte SFR write private, so this would take a
small patch upstream rather than a local work-around.

**No `nINT` open-drain choice.** `IOCON.INTOD` stays at its power-on 0,
so `nINT` is active-low push-pull, which is what both boards want (plane
PA0 with a 100k pull-up, controller GPIO3 with a 10k).

**No async facade.** `mcp251xfd` has an async driver behind its `async`
feature and this crate re-exports the whole crate (`pub use mcp251xfd`),
so an async caller can use `MCP251xFdAsync` directly. Mirroring the
facade for async would have meant writing it twice for no present
caller.

**No CRC-protected SPI transfers** (`READ_CRC` / `WRITE_CRC`) and no TEF,
no timestamps, no second filter: nothing on the link needs them yet.

## Untested on hardware

**All of it.** No MCP2518FD has run this code: as of this commit the FD
boards (`power/pp-48-core-fd`, `pp-48-unit-fd`, `controller-fd`) are
schematics only, not ordered. Every test here runs against a mock SPI
device, which proves the bytes the driver puts on the wire and nothing
about what the chip does with them.

Specifically unverified, and worth checking first on the bench:

- that a 40 MHz crystal starts and `OSC.OSCRDY` comes up in the 3 ms the
  wrapped driver waits;
- that the computed bit timing actually interoperates with the G0B1's
  FDCAN at the other end, and with the controller's TCAN1044V -- the
  sample points and TDCO are from the recipe, not from a scope;
- the SPI clock ceiling: 17 MHz at a 40 MHz SYSCLK is an erratum limit
  (`mcp251xfd::max_spi_hz`), and the shared W5500 bus on the controller
  must be configured per-device to respect it;
- bus-off and its automatic recovery, which no test exercises;
- whether one 8-deep TX FIFO is enough for a 5-segment message under
  real arbitration loss.

## Dependency note

`mcp251xfd` builds its sync and async drivers from one source with
`maybe-async-cfg`, which pulls `proc-macro-error` 1.0.4 --
**RUSTSEC-2024-0370, unmaintained**. It is a build-time proc-macro
dependency: no part of it reaches the firmware image. It will still show
up in `cargo audit` / `cargo deny`, so it needs an allow entry rather
than a fix on our side.
