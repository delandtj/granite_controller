# plane-fw

The plane core application for the PP-48 core board: embassy on
thumbv6m, with all of the behaviour in the workspace crate
[`plane-logic`](../plane-logic) and nothing but board glue here.

**One link per image**, chosen by a cargo feature (ADR 0005 decision 9);
the link decides the MCU, so it carries the embassy-stm32 chip feature
and `build.rs` picks the linker script and the pin map with it:

| Feature | Board | MCU | Link | memory.x | Pin map |
|---|---|---|---|---|---|
| `link-i2c` | pp-48-core (rev D) | STM32G070CB | I2C1 slave through the ISO1640, INT_OUT for EXP_INT | `memory-g070cb.x` | `plane-core-pins.toml` |
| `link-mcp` (**default**) | pp-48-core-fd | STM32G070CB | MCP2518FD on SPI1, nINT on PA0 | `memory-g070cb.x` | `plane-core-fd-pins.toml` |
| `link-fdcan` | pp-48-core-fd | STM32G0B1CC | FDCAN1 on PB8 / PB9 AF3 | `memory-g0b1cc.x` | `plane-core-fd-pins.toml` |

`link-mcp` is the default because the first batch is the G070 + an
MCP2518FD (hardware ADR 0006 decision 6: the G0B1 was out of stock).
None of the three set, or two of them, is a build error from both
`build.rs` and `src/main.rs`.

- Design: `firmware/docs/adr/0002-plane-core-firmware.md`
- Link protocol: `firmware/docs/adr/0003-plane-link-protocol.md`
- CAN FD transport: `firmware/docs/adr/0005-plane-link-can-fd-transport.md`
- CAN FD hardware: `docs/adr/0006-plane-link-can-fd.md`
- Pins: `firmware/plane-core-pins.toml` and
  `firmware/plane-core-fd-pins.toml` (generated; the truth)
- Press deadline: `docs/press-deadline-line.md`

This crate is **excluded from the firmware/ workspace**
(`firmware/Cargo.toml` `exclude`), like `granite-fw`: it has its own
target, linker scripts, toolchain file and `.cargo/config.toml`.

## Build

```sh
cd firmware/plane-fw
rustup target add thumbv6m-none-eabi     # once

cargo build --release                                            # link-mcp
cargo build --release --no-default-features --features link-i2c
cargo build --release --no-default-features --features link-fdcan

cargo clippy --release -- -D warnings
cargo clippy --release --no-default-features --features link-i2c -- -D warnings
cargo clippy --release --no-default-features --features link-fdcan -- -D warnings
```

`rust-toolchain.toml` pins stable with the target and `llvm-tools`.
`build.rs` parses the build's pin map and the `core_pins!` table in its
pin module (`src/pins.rs` for rev D, `src/pins_fd.rs` for the FD board)
and fails the build when a net moved port, changed mode, or is missing
from the firmware's map: there is no test harness on the board, so that
check is the pin map's test. It also copies the build's `memory-*.x` to
`OUT_DIR/memory.x` and puts `OUT_DIR` on the linker's search path -
which is why there is no `memory.x` in the crate root: `ld` resolves an
`INCLUDE` from the current directory first, and one there would silently
win over the file the feature chose.

## Flash and watch (SWD on PA13 / PA14)

```sh
cargo run --release                      # = probe-rs run --chip STM32G070CBTx
probe-rs attach --chip STM32G070CBTx target/thumbv6m-none-eabi/release/plane-fw

# link-fdcan is another part, and the runner in .cargo/config.toml
# cannot follow a feature:
probe-rs run --chip STM32G0B1CCTx target/thumbv6m-none-eabi/release/plane-fw
```

The runner is in `.cargo/config.toml`; `DEFMT_LOG=info` is set there
too, so `trace`/`debug` calls are compiled out of the image. Logs come
over RTT (defmt).

## Size

`llvm-size -A` of each release image, against the ~52 KB ACTIVE slot of
ADR 0002 decision 2 (53 248 bytes; the ADR calls the sizes "to confirm
after the first build" and derives 52 KB from the G070's 128 KB):

| Section | `link-i2c` | `link-mcp` | `link-fdcan` |
|---|---|---|---|
| `.vector_table` | 184 | 184 | 188 |
| `.text` | 39 820 | 42 860 | 45 820 |
| `.rodata` | 7 040 | 7 692 | 7 712 |
| `.data` | 80 | 80 | 168 |
| **flash total** | **47 124 (46.0 KB)** | **50 816 (49.6 KB)** | **53 888 (52.6 KB)** |
| against ACTIVE | 6 124 spare | 2 432 spare | **640 over** |
| RAM (`.data` + `.bss` + `.uninit`) | 7 756 of 36 KB | 7 880 of 36 KB | 9 272 of 144 KB |

Three things to know about that table:

- **`link-mcp` fits the slot** with 2.4 KB to spare, but it is past the
  ~48 KB line at which ADR 0002's "Code size" risk says to drop from the
  A/B swap to a copy-only update. That is a decision for the bootloader
  work (there is no plane-boot yet), not something this crate can settle.
- **`link-fdcan` does not fit** the 52 KB slot: it is 640 bytes over.
  Nothing is broken today - both images are flashed whole over SWD at
  0x08000000 - but the slot plan has to say something about this part
  before plane-boot lands. The G0B1CC has 256 KB, of which the ADR 0002
  plan uses 128 KB, so the natural answer is a per-part slot plan (and
  an ADR 0002 update) rather than trimming 640 bytes out of the only
  build that has room to spare around it. The +3 KB over `link-mcp` is
  embassy's FDCAN driver and its message RAM handling, against the
  MCP2518FD facade over SPI.
- `link-i2c` grew 312 bytes (46 812 -> 47 124) when the CAN work landed:
  `Plane` gained the STATUS 0x1D record it never encodes and
  `granite_proto::frame::trim_padded`, which the I2C link does not call
  either. Both are cheaper than a second `Plane`.

`opt-level = "z"`, fat LTO, one codegen unit; debug info stays in the
ELF (it is not loaded) because probe-rs and defmt-decoder need it.

## What runs

- **Clocks, G070 builds** (`link-i2c`, `link-mcp`): 8 MHz HSE crystal
  (`hse_hz` in the pin map) -> PLL M=1 N=16 R=2 -> 64 MHz SYSCLK. The
  HSI stays on as the ADC clock: the G0's ADC takes at most 35 MHz.
  Nothing of the CAN timing hangs on this tree - the MCP2518FD has its
  own 40 MHz crystal.
- **Clocks, `link-fdcan`**: 8 MHz HSE -> PLL M=1 **N=40** (320 MHz VCO)
  -> **R=5 for a 64 MHz SYSCLK** and **Q=8 for a 40 MHz FDCAN kernel
  clock** (`FDCANSEL = PLL1_Q`), which is hardware ADR 0006 decision 11
  and the same 40 MHz the MCP2518FD gets from its crystal, so one bit
  timing serves both builds. The G070's 128 MHz VCO cannot do it: Q
  divides by 2 to 8, giving 64, 42.7, 32 ... and never 40. 320 MHz is
  inside embassy's 96-344 MHz VCO range, R=5 lands exactly on the 64 MHz
  SYSCLK maximum, and nothing in the chain is fractional - the 80 TQ /
  20 TQ bit timing needs the clock exact, not close.
- **Link task, `link-mcp`** (`src/link_mcp.rs`): `plane_logic::can_link`
  over an MCP2518FD on SPI1 (PA5 / PA6 / PA7 AF0) at 8 MHz, mode 0, with
  nCS a software output on PA4 (`embedded_hal_bus::spi::ExclusiveDevice`)
  and nINT on PA0 as an EXTI falling edge with a 2 ms timeout. The chip
  is configured by the `mcp2518fd` facade: 500 kbit/s nominal at 80 %,
  2 Mbit/s data at 75 %, TDC on, one TX and one RX FIFO of 64-byte
  payloads, one acceptance filter on this plane's node id. **PB8 / PB9
  are not touched**: the chip's TXCAN drives the same ISO1044 TXD net as
  PB9, so this build must leave PB9 in its reset state (analog, high-Z),
  and `src/pins_fd.rs` holds both pins and hands them to nobody.
- **Link task, `link-fdcan`** (`src/link_fdcan.rs`): the same transport
  over FDCAN1 (PB8 / PB9 AF3) through embassy's buffered FD wrapper,
  whose channels have the non-blocking `try_receive` / `try_write` the
  transport's synchronous driver trait needs. The bit timing is written
  out rather than derived from `set_bitrate`: nominal prescaler 1, TSEG1
  63, TSEG2 16, SJW 16 (80 TQ, 500 kbit/s, 80 %); data prescaler 1,
  TSEG1 14, TSEG2 5, SJW 5 (20 TQ, 2 Mbit/s, 75 %); `AllowFdCanAndBRS`,
  automatic bus-off recovery, non-matching frames rejected and one
  standard filter `(id & 0x00F) == node` into RX FIFO 0. **TDC** is set
  through the PAC (`DBTP.TDC`, `TDCR.TDCO = 14`): embassy-stm32 0.6
  carries a `transceiver_delay_compensation` flag but never writes the
  register (`can/fd/config.rs` leaves it a TODO), and DBTP is only
  writable with INIT and CCE set, so `enable_tdc` dips the node back
  into initialisation for a few cycles right after `into_normal_mode`,
  before any frame has moved.
- **Events on the CAN builds**: no INT_OUT. The plane sends an EVENT
  frame from its node, payload `0x01`, when the first event is raised
  and again every second until an EV_ACK covers them all (ADR 0005
  decision 6); the schedule is in `plane_logic::can_link` and runs off
  the same `event_pending` bit rev D puts on PD2.
- **CAN counters**: the transport publishes TEC / REC, the bus-off
  count, the reassembly drops and the frames a full TX FIFO refused into
  `Plane` once per round, and STATUS carries them as the new record
  0x1D (ADR 0005 decision 8). The I2C build never publishes any, so its
  STATUS block is byte for byte what it was.
- **Link task, `link-i2c`** (`src/link.rs`): I2C1 (PB8 / PB9, AF6) as an I2C slave
  at `0x30 + ADR[2:0]` (PA15 / PD1 / PA8, pulled up, DIP closed = 0).
  A 2-byte write is a read request and is answered out of the model on
  the spot (`Plane::handle_frame`); the answer is clocked out on the
  read that follows the repeated start and kept afterwards, so a retried
  read gets the same snapshot. A 4+ byte write is a frame, CRC-checked
  and queued for the bus task. embassy-stm32 0.6's own slave support is
  used (`into_slave_multimaster`, `listen`, `respond_to_write`,
  `respond_to_read`, including repeated-start detection): the ADR's
  PAC-level fallback was not needed.
- **Bus task** (`src/main.rs`, `src/segment_bus.rs`): I2C2 (PA11 /
  PA12, AF6) at 100 kHz as `plane_logic::SegmentBus` - blocking
  transfers, because this task is the only owner of the bus. It calls
  `Plane::tick` every 10 ms; the poll cycle inside it is scheduled at
  50 ms (ADR 0002 decision 5) and the finer tick only buys resolution
  for the sequences and the PRESS_TRIG toggling. It also feeds the
  500 ms IWDG, but only when `Plane::may_feed_watchdog` says a full
  cycle ran, and resets the board when the plane asks.
- **Status task** (`src/status.rs`): LED_RUN (PB14) 1 Hz heartbeat,
  5 Hz while an update runs, double blink while unconfirmed; on
  `link-i2c` also INT_OUT (PD2) high while unacknowledged events exist.
  Both from atomics the other tasks publish, so the heartbeat cannot be
  stalled by a poll cycle holding the plane. The CAN builds have no
  INT_OUT at all (PD2 is free on the FD board) and the task is the
  heartbeat alone.
- **Press deadline**: PRESS_TRIG (PB12) is toggled by the plane logic
  from the task that owns the press, about every 200 ms, and never
  otherwise - no timer, no DMA, so a hung firmware loses PRESS_EN after
  about a second.
- **MUX_RST** (PB13) and **EXP_RSTN** (PD0, open drain) are the two
  recovery lines; `unstick` drives nine SCL clocks on PA11 with I2C2
  disabled (through the PAC) before the mux reset.
- **VIN_SNS** (PB11, ADC_IN15): 220k / 10k divider, integer conversion
  to the wire's 10 mV units, read on every tick.
- **TAMP backup registers** (`src/backup_regs.rs`): the intended state
  of ADR 0002 decision 3. embassy-stm32's RTC backup-register accessors
  are stubs on this family (the words are in TAMP, not RTC), so this
  goes through the PAC after enabling the RTC/TAMP APB clock and
  clearing the backup-domain write protection.
- **CONFIG pages** (`src/config_store.rs`): the two copies with a
  sequence number and a CRC-32, at **0x0801F000 / 0x0801F800** for now -
  see below. Both parts have 2 KB pages and 8-byte writes, so the same
  two pages serve all three builds.

Panics use `panic-probe` with `print-defmt`: the location is printed
over RTT and the core stops, which on a board in the field means the
IWDG resets it and the new image releases every press on its way up
(ADR 0002 decision 3). `panic-halt` would have cost the message and
gained nothing.

## What is stubbed or deliberately different

- **Firmware update over the link** (`src/update_store.rs`,
  ADR 0002 decision 9): every DFU operation refuses with a storage
  error, so FW_BEGIN fails and the controller reads update state
  "failed". There is no `plane-boot` and no partition split yet, and a
  DFU write with these linker scripts would land in the running image.
  Needs: plane-boot (embassy-boot-stm32, IWDG fed through the swap),
  the memory.x split of decision 2, and `FirmwareUpdater` behind the
  six methods of `UpdateStore`.
- **The linker script takes the whole part** (128 KB on the G070,
  256 KB on the G0B1CC) and the image is flashed at 0x08000000 over SWD.
  The slot plan (16 KB BOOT / 2 KB STATE / 4 KB CONFIG / 52 KB ACTIVE /
  54 KB DFU) is written out in both `memory-*.x` files as the comment it
  will become; on the G0B1CC it leaves 128 KB of bank 2 spare, and the
  `link-fdcan` image is 640 bytes too big for the 52 KB ACTIVE slot
  (see "Size"). embassy-stm32's `memory-x` feature is **off** on
  purpose: these files are the ones that carry the slot plan, and two
  memory.x on the linker's search path would be a coin toss.
- **CONFIG pages are at 0x0801F000**, not at 0x08004800. They move with
  the bootloader; the first boot after the move reads blank pages, falls
  back to the defaults and the controller re-writes the set (it compares
  the config hash in INFO). On the G0B1CC those are the last two pages
  of the **first** 128 KB rather than of the part, which is bank 1
  whichever way that part's DUAL_BANK option bit is set: a page erase
  then names the same physical page in single-bank numbering (page 62 of
  128) and in dual-bank numbering (page 62 of bank 1, BKER = 0). That is
  also why the `link-fdcan` feature tells embassy `single-bank` - the
  crate refuses to build for a configurable part without being told, and
  the choice cannot be wrong for a store that never leaves bank 1.
- **MCU temperature** (STATUS 0x11) reports nothing: the G0 sensor
  needs the two factory calibration words and a division per sample,
  and no plane decision depends on it (the unit hot-swap latch judges
  the *core input voltage*, which is read).
- **The IWDG starts after `Plane::boot`**, not before it. A
  four-channel discovery scan with NAK timeouts can outlast 500 ms, and
  what bounds a held button during boot is the hardware press deadline:
  PRESS_TRIG is not toggled until a press runs, so PRESS_EN is already
  dead when the application starts.
- **A link write longer than 255 bytes is truncated** (`link-i2c`) and
  then fails its length check (counted in STATUS 0x19). The slave sets
  its transfer up in chunks below 256 bytes; the longest frame ADR 0003
  defines is a FW data record at ~137 bytes. The CAN builds have no such
  limit: a message is at most 5 segments and the reassembly buffer is
  320 bytes.
- **The plane is behind one mutex.** Neither task awaits while holding
  it, so they cannot interleave inside the state machine, but a request
  that arrives mid-poll-cycle waits until the cycle ends (a few ms, up
  to ~25 ms at four units). On `link-i2c` that is clock stretching,
  which ADR 0002 decision 8 wants to be a fallback; on the CAN builds it
  is just latency, inside the controller's 20 ms read timeout at one
  unit and over it at four, so the same measurement is due on the bench:
  if it bites, cut the cycle into per-unit steps that release the mutex
  between units.
- **SYNC is ignored** (ADR 0005 decision 7): the frame is dropped and
  counted as "not ours". It becomes the standby controller's keepalive
  in a later ADR.

## Untested

Nothing in this crate has run on hardware: there is no core board here,
and no FD board of any kind exists yet. What is verified is
`cargo build --release` and `cargo clippy --release -- -D warnings` for
thumbv6m-none-eabi on all three features, the pin map against the
generated TOML, and the behaviour underneath - which is `plane-logic`'s
host test suite over its simulated devices, the CAN transport included
(`plane-logic/tests/can_link.rs` drives it against a fake controller).

Everything that only exists on the board is unproven: the 64 MHz PLL
bring-up on either part, the I2C slave against a real master through the
ISO1640 (clock stretching, the repeated start, the lost-byte question
ADR 0002 lists as a risk), the ADC scaling of the divider, the TAMP
words surviving a reset, the flash CONFIG pages, the nine-clock
recovery, and the press deadline monostable being retriggered often
enough.

The CAN builds add their own unproven list: the MCP2518FD answering on
SPI1 at all (and its nINT timing), the G0B1's 320 MHz VCO and the 40 MHz
FDCAN kernel clock, the hand-written bit timing against a real
transceiver, the `enable_tdc` PAC sequence, whether 2 Mbit/s survives a
full row through the ISO1044s (ADR 0006 keeps 1 Mbit/s as the fallback,
which is two constants), and the answer latency against the controller's
20 ms read timeout.
