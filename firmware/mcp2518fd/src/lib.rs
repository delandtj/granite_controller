#![no_std]
//! MCP2518FD CAN FD controller, configured for the Granite plane link.
//!
//! This crate is a thin facade over the [`mcp251xfd`] driver: that crate
//! owns the register map, the SPI instruction layer and the silicon
//! errata work-arounds, and this one fixes the single configuration
//! `firmware/docs/adr/0005-plane-link-can-fd-transport.md` decision 8
//! asks for and narrows the API to what the link needs.
//!
//! One clock, one bit timing, one TX FIFO, one RX FIFO, one filter:
//! 40 MHz crystal, 500 kbit/s nominal at an 80 % sample point,
//! 2 Mbit/s data at 75 %, transmitter delay compensation on, ISO CRC,
//! BRS on every frame, RX timestamps off. Both ends of the link use it:
//! the plane's G070 build (SPI1, hardware ADR 0006 decision 6) and the
//! controller, where the chip shares the W5500's SPI bus (decision 9) --
//! `SpiDevice` does the chip-select framing, so sharing needs nothing
//! from this crate.
//!
//! ```no_run
//! # fn link<S: embedded_hal::spi::SpiDevice, D: embedded_hal::delay::DelayNs>(
//! #     spi: S, delay: &mut D,
//! # ) -> Result<(), mcp2518fd::Error<S::Error>> {
//! use mcp2518fd::{Config, Mcp2518fd};
//!
//! // A plane at node 3: accept the ids addressed to it, any message type.
//! let mut can = Mcp2518fd::new(spi);
//! can.init(&Config::for_node(3), delay)?;
//! can.send(0x033, &[0x80, 0x12])?;
//! if let Some(frame) = can.receive()? {
//!     let _ = (frame.id, frame.payload());
//! }
//! # Ok(())
//! # }
//! ```
//!
//! INT0 / INT1 are left in their power-on state (GPIO inputs): both
//! boards pull them up, so nothing here drives them. See the README.

use embedded_can::{Id, StandardId};
use embedded_hal::delay::DelayNs;
use embedded_hal::spi::SpiDevice;
use mcp251xfd::registers::{CiInt, Fifo, Filter, OperationMode, PayloadSize};
use mcp251xfd::{
    ClockConfig, DataBitTiming, FdFrame, FifoLayout, FilterMatch, FrameFlags, MCP251xFd,
    NominalBitTiming, ReceivedFrame,
};

/// The driver this crate wraps, for anything the facade does not expose
/// (the raw register escape hatches, loopback modes, the async driver).
pub use mcp251xfd;

/// SYSCLK: the 40 MHz crystal of hardware ADR 0006 decision 6, no PLL,
/// no divider (X322540MPB4SI on the core, Y3 on the controller).
pub const SYSCLK_HZ: u32 = 40_000_000;

/// Nominal (arbitration) bit rate: ADR 0005 decision 8.
pub const NOMINAL_BIT_RATE_HZ: u32 = 500_000;
/// Nominal sample point in per mille: ADR 0005 decision 8 (80 %).
pub const NOMINAL_SAMPLE_POINT: u32 = 800;
/// Data-phase bit rate with BRS: ADR 0005 decision 8.
pub const DATA_BIT_RATE_HZ: u32 = 2_000_000;
/// Data-phase sample point in per mille: ADR 0005 decision 8 (75 %).
pub const DATA_SAMPLE_POINT: u32 = 750;

/// Splits a bit time into `(tseg1, tseg2)` at a prescaler of 1.
///
/// The bit is `1 + tseg1 + tseg2` time quanta of `1 / sysclk_hz` each
/// (TQ = BRP / SYSCLK, BRP 1); the sample point sits at the end of
/// TSEG1, so `1 + tseg1 = tq * sample_point / 1000`. Keeping BRP at 1
/// gives the finest resolution and the smallest quantisation error,
/// which is what the MCP2518FD family reference manual (DS20005678E
/// section 3.4) recommends and what every bit rate the link uses allows.
///
/// Panics (at compile time, in the `const` uses below) if the bit rate
/// does not divide SYSCLK exactly or the sample point is unreachable.
const fn split(sysclk_hz: u32, bit_rate_hz: u32, sample_point_permille: u32) -> (u16, u16) {
    let tq = sysclk_hz / bit_rate_hz;
    assert!(tq * bit_rate_hz == sysclk_hz, "bit rate must divide SYSCLK");
    assert!(tq >= 4, "too few time quanta per bit");
    let before = tq * sample_point_permille / 1000;
    assert!(
        before * 1000 == tq * sample_point_permille,
        "sample point not reachable at this bit rate"
    );
    assert!(before >= 2 && before < tq, "sample point out of range");
    ((before - 1) as u16, (tq - before) as u16)
}

/// Nominal bit timing at [`SYSCLK_HZ`], SJW = TSEG2 (the maximum the
/// standard allows, so the widest tolerance to clock drift along the
/// 3 m of board trace the link runs over).
pub const fn nominal_timing(bit_rate_hz: u32, sample_point_permille: u32) -> NominalBitTiming {
    let (tseg1, tseg2) = split(SYSCLK_HZ, bit_rate_hz, sample_point_permille);
    NominalBitTiming {
        brp: 1,
        tseg1,
        tseg2,
        sjw: tseg2,
    }
}

/// Data-phase bit timing at [`SYSCLK_HZ`]. Same rule as
/// [`nominal_timing`]; the data-phase fields are narrower (TSEG1 1..=32,
/// TSEG2 and SJW 1..=16), which `DataBitTiming::validate` checks.
pub const fn data_timing(bit_rate_hz: u32, sample_point_permille: u32) -> DataBitTiming {
    let (tseg1, tseg2) = split(SYSCLK_HZ, bit_rate_hz, sample_point_permille);
    DataBitTiming {
        brp: 1,
        tseg1: tseg1 as u8,
        tseg2: tseg2 as u8,
        sjw: tseg2 as u8,
    }
}

/// 500 kbit/s, 80 % sample point, at 40 MHz: 80 TQ, TSEG1 63, TSEG2 16,
/// SJW 16. Byte for byte the set ADR 0005 decision 8 asks for.
pub const NOMINAL: NominalBitTiming = nominal_timing(NOMINAL_BIT_RATE_HZ, NOMINAL_SAMPLE_POINT);

/// 2 Mbit/s, 75 % sample point, at 40 MHz: 20 TQ, TSEG1 14, TSEG2 5,
/// SJW 5. TDCO follows from it (`DBRP * DTSEG1` = 14), set by
/// `mcp251xfd`'s `init` together with TDCMOD = auto.
pub const DATA: DataBitTiming = data_timing(DATA_BIT_RATE_HZ, DATA_SAMPLE_POINT);

/// Clock, bit timing and TDC, as handed to the wrapped driver.
pub const CHIP_CONFIG: mcp251xfd::Config = mcp251xfd::Config {
    clock: ClockConfig::MHZ40,
    nominal: NOMINAL,
    data: Some(DATA),
};

/// The one transmit FIFO (ADR 0005 decision 9).
pub const TX_FIFO: Fifo = Fifo::F1;
/// The one receive FIFO.
pub const RX_FIFO: Fifo = Fifo::F2;
/// The one acceptance filter.
pub const RX_FILTER: Filter = Filter::F0;
/// Transmit FIFO depth: a 258-byte message is at most 5 segments
/// (ADR 0005 decision 3), so 8 slots hold one whole message plus slack.
pub const TX_DEPTH: u8 = 8;
/// Receive FIFO depth.
pub const RX_DEPTH: u8 = 8;

/// Message RAM layout: two 64-byte-payload FIFOs, 8 slots each. Every
/// link frame is padded to 64 bytes, so both need `PLSIZE` = B64 --
/// a smaller payload size would let a 64-byte frame overrun its slot.
/// 2 * 8 * (8 + 64) = 1152 bytes of the 2048, checked at compile time
/// by `FifoLayout`'s const builder.
pub const LAYOUT: FifoLayout = FifoLayout::new()
    .tx_fifo(TX_FIFO, PayloadSize::B64, TX_DEPTH)
    .rx_fifo(RX_FIFO, PayloadSize::B64, RX_DEPTH);

/// What the acceptance filter keeps. Everything else about the chip is
/// fixed by the ADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    /// Identifier bits to match, standard (11-bit) ids only.
    pub filter_id: u16,
    /// Which of the 11 bits must match: 1 = must match, 0 = don't care.
    /// `0` accepts every standard id.
    pub filter_mask: u16,
}

impl Config {
    /// Accepts the ids addressed to one node, any message type:
    /// ADR 0005 decision 2 puts the node in the low 4 bits of
    /// `id = (type << 4) | node`, so the mask is `0x00F`.
    pub const fn for_node(node: u8) -> Self {
        Self {
            filter_id: (node & 0x0F) as u16,
            filter_mask: 0x00F,
        }
    }

    /// Accepts every standard id (the controller, which talks to every
    /// node and is the only sender of SYNC).
    pub const fn promiscuous() -> Self {
        Self {
            filter_id: 0,
            filter_mask: 0,
        }
    }
}

/// A received frame: the 11-bit id, the length, and a 64-byte buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// Standard (11-bit) identifier.
    pub id: u16,
    /// Payload length, one of the valid CAN FD lengths.
    pub len: u8,
    /// Payload, zero beyond `len`.
    pub data: [u8; 64],
}

impl Frame {
    /// The payload, `len` bytes of it.
    pub fn payload(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

/// What can go wrong. `E` is the SPI device's own error type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error<E> {
    /// The transmit FIFO is full; the frame was not queued. Retry when a
    /// slot frees up (TX interrupt, or poll). The `WouldBlock` of
    /// ADR 0005's send path.
    WouldBlock,
    /// The identifier does not fit 11 bits.
    InvalidId,
    /// The payload is longer than 64 bytes.
    PayloadTooLong,
    /// An extended-id frame came out of the RX FIFO. The filter is
    /// standard-only (`MIDE` set), so this means the filter was changed
    /// behind the facade's back.
    ExtendedId,
    /// Anything the wrapped driver reports: SPI errors, a failed RAM
    /// echo test, mode-change timeouts, bad configuration.
    Driver(mcp251xfd::Error<E>),
}

impl<E> From<mcp251xfd::Error<E>> for Error<E> {
    fn from(e: mcp251xfd::Error<E>) -> Self {
        match e {
            mcp251xfd::Error::TxFifoFull => Error::WouldBlock,
            other => Error::Driver(other),
        }
    }
}

/// The driver.
pub struct Mcp2518fd<SPI> {
    inner: MCP251xFd<SPI>,
}

impl<SPI: SpiDevice> Mcp2518fd<SPI> {
    /// Wraps an SPI device. The bus must be mode (0,0) and at or below
    /// 17 MHz at a 40 MHz SYSCLK (`mcp251xfd::max_spi_hz`, a silicon
    /// erratum limit, not a guess). Call [`Self::init`] next.
    pub fn new(spi: SPI) -> Self {
        Self {
            inner: MCP251xFd::new(spi),
        }
    }

    /// Issues the RESET instruction (SPI opcode 0x0). Only guaranteed
    /// from Configuration mode, which is where the chip sits after
    /// power-on; [`Self::init`] resets on its own, so this is for
    /// putting a running chip back to a known state.
    pub fn reset(&mut self) -> Result<(), Error<SPI::Error>> {
        Ok(self.inner.reset()?)
    }

    /// Resets and configures the chip, then puts it on the bus in CAN FD
    /// normal mode: [`CHIP_CONFIG`] (clock, both bit timings, TDC auto,
    /// ISO CRC, BRS enabled), [`LAYOUT`] (one TX and one RX FIFO), and
    /// one standard-id acceptance filter from `config` routed to the RX
    /// FIFO.
    ///
    /// `delay` covers the oscillator stabilisation time and the mode
    /// change polls. Returns the chip variant the driver detected.
    pub fn init<D: DelayNs>(
        &mut self,
        config: &Config,
        delay: &mut D,
    ) -> Result<mcp251xfd::registers::Variant, Error<SPI::Error>> {
        let variant = self.inner.init(&CHIP_CONFIG, delay)?;
        self.inner.apply_layout(&LAYOUT)?;
        let id = StandardId::new(config.filter_id).ok_or(Error::InvalidId)?;
        let matcher = FilterMatch::with_mask(Id::Standard(id), config.filter_mask as u32);
        self.inner.set_filter(RX_FILTER, matcher, RX_FIFO)?;
        self.inner.set_mode(OperationMode::NormalFd, delay)?;
        Ok(variant)
    }

    /// Queues one CAN FD frame with BRS and requests transmission.
    ///
    /// The DLC follows from `data.len()`, zero-padded up to the next
    /// valid CAN FD length (0-8, 12, 16, 20, 24, 32, 48, 64) -- the
    /// padding rule of ADR 0005 decision 3, where the message's own
    /// length makes the padding ignorable.
    ///
    /// Returns [`Error::WouldBlock`] when the transmit FIFO is full;
    /// nothing was queued and the call can simply be retried.
    pub fn send(&mut self, id: u16, data: &[u8]) -> Result<(), Error<SPI::Error>> {
        if data.len() > 64 {
            return Err(Error::PayloadTooLong);
        }
        let id = StandardId::new(id).ok_or(Error::InvalidId)?;
        let frame = FdFrame::new_padded(
            id,
            data,
            FrameFlags {
                brs: true,
                esi: false,
            },
        )
        .ok_or(Error::PayloadTooLong)?;
        Ok(self.inner.transmit_fd(TX_FIFO, &frame)?)
    }

    /// Takes one frame off the receive FIFO, or `None` when it is empty.
    /// Advances the FIFO tail (`UINC`) for the frame it returns.
    pub fn receive(&mut self) -> Result<Option<Frame>, Error<SPI::Error>> {
        let rx = match self.inner.receive(RX_FIFO) {
            Ok(rx) => rx,
            Err(mcp251xfd::Error::RxFifoEmpty) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut out = Frame {
            id: 0,
            len: 0,
            data: [0u8; 64],
        };
        let id = match rx.frame {
            ReceivedFrame::Fd(f) => {
                let p = f.data();
                out.len = p.len() as u8;
                out.data[..p.len()].copy_from_slice(p);
                f.id()
            }
            ReceivedFrame::Classic(f) => {
                let p = f.data();
                out.len = p.len() as u8;
                out.data[..p.len()].copy_from_slice(p);
                f.id()
            }
        };
        out.id = match id {
            Id::Standard(s) => s.as_raw(),
            Id::Extended(_) => return Err(Error::ExtendedId),
        };
        Ok(Some(out))
    }

    /// Reads `C1INT`: the interrupt flags and their enables. The nINT
    /// pin is active low and push-pull (`IOCON.INTOD` left at its
    /// power-on 0), wired to PA0 on the plane and GPIO3 on the
    /// controller.
    pub fn interrupt_flags(&mut self) -> Result<CiInt, Error<SPI::Error>> {
        Ok(self.inner.interrupt_flags()?)
    }

    /// Reads `C1TREC`: `(tec, rec)`, the transmit and receive error
    /// counters. ADR 0005 decision 8 reports these in STATUS on the
    /// plane and in the controller's diagnostics.
    pub fn error_counters(&mut self) -> Result<(u8, u8), Error<SPI::Error>> {
        let trec = self.inner.error_counters()?;
        Ok((trec.tec(), trec.rec()))
    }

    /// Whether the chip is bus-off (`C1TREC.TXBO`). Recovery is
    /// automatic: ADR 0005 decision 8 wants it counted, not acted on.
    pub fn is_bus_off(&mut self) -> Result<bool, Error<SPI::Error>> {
        Ok(self.inner.error_counters()?.tx_bus_off())
    }

    /// The wrapped driver, for what the facade does not cover.
    pub fn inner(&mut self) -> &mut MCP251xFd<SPI> {
        &mut self.inner
    }

    /// Gives the SPI device back.
    pub fn release(self) -> SPI {
        self.inner.release()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Bit timing: the hardcoded set of ADR 0005 decision 8, and the
    // const fn that derives it.
    #[test]
    fn nominal_is_the_adr_set() {
        assert_eq!(NOMINAL.brp, 1);
        assert_eq!(NOMINAL.tseg1, 63);
        assert_eq!(NOMINAL.tseg2, 16);
        assert_eq!(NOMINAL.sjw, 16);
        assert_eq!(NOMINAL.tq_per_bit(), 80);
        assert_eq!(NOMINAL.bit_rate_hz(SYSCLK_HZ), 500_000);
        assert_eq!(NOMINAL.sample_point_permille(), 800);
        assert!(NOMINAL.validate().is_ok());
    }

    #[test]
    fn data_is_the_adr_set() {
        assert_eq!(DATA.brp, 1);
        assert_eq!(DATA.tseg1, 14);
        assert_eq!(DATA.tseg2, 5);
        assert_eq!(DATA.sjw, 5);
        assert_eq!(DATA.tq_per_bit(), 20);
        assert_eq!(DATA.bit_rate_hz(SYSCLK_HZ), 2_000_000);
        assert_eq!(DATA.sample_point_permille(), 750);
        assert_eq!(DATA.tdco(), 14);
        assert!(DATA.tdco_fits());
        assert!(DATA.validate().is_ok());
    }

    #[test]
    fn chip_config_validates() {
        assert!(CHIP_CONFIG.validate().is_ok());
        assert_eq!(CHIP_CONFIG.clock.sysclk_hz(), SYSCLK_HZ);
        // BRS is on precisely because the data timing is present.
        assert!(CHIP_CONFIG.data.is_some());
    }

    #[test]
    fn split_derives_other_rates_too() {
        // 1 Mbit/s at 80 %: 40 TQ -> 31 / 8.
        assert_eq!(split(SYSCLK_HZ, 1_000_000, 800), (31, 8));
        // 250 kbit/s at 80 %: 160 TQ -> 127 / 32.
        assert_eq!(split(SYSCLK_HZ, 250_000, 800), (127, 32));
        // 5 Mbit/s at 75 %: 8 TQ -> 5 / 2.
        assert_eq!(split(SYSCLK_HZ, 5_000_000, 750), (5, 2));
        // The two the link uses.
        assert_eq!(split(SYSCLK_HZ, 500_000, 800), (63, 16));
        assert_eq!(split(SYSCLK_HZ, 2_000_000, 750), (14, 5));
    }

    #[test]
    fn layout_fits_message_ram() {
        assert_eq!(LAYOUT.total_bytes(), 2 * 8 * (8 + 64));
        assert!(LAYOUT.total_bytes() <= 2048);
    }

    #[test]
    fn node_filter_matches_every_type_of_one_node() {
        let c = Config::for_node(3);
        assert_eq!(c.filter_id, 3);
        assert_eq!(c.filter_mask, 0x00F);
        // ADR 0005 decision 2: id = (type << 4) | node. REQ (2) to node
        // 3 is 0x023, RSP (3) from node 3 is 0x033; both match, node 4
        // does not.
        let matches = |id: u16| (id ^ c.filter_id) & c.filter_mask == 0;
        assert!(matches(0x023));
        assert!(matches(0x033));
        assert!(!matches(0x024));
        // Node is 4 bits wide: 15 (standby controller) stays in range.
        assert_eq!(Config::for_node(15).filter_id, 15);
        assert_eq!(Config::promiscuous().filter_mask, 0);
    }
}
