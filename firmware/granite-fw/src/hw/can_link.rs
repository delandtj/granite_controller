//! The CAN FD plane link: an MCP2518FD on the W5500's SPI bus
//! (hardware ADR 0006 decision 9, firmware ADR 0005).
//!
//! The transport is [`granite_core::plane::can_link::CanPlaneLink`],
//! host-tested; this file is the hardware under it and nothing else:
//!
//! - a [`SpiDeviceDriver`] of our own on the shared bus
//!   ([`crate::platform::net::spi_bus`]), chip select GPIO2, 10 MHz
//!   (the MCP2518FD's ceiling is ~17 MHz at a 40 MHz SYSCLK, an erratum
//!   limit; the W5500 keeps its own 20 MHz, since the clock is per
//!   device);
//! - reset, configure and go on the bus through the `mcp2518fd` facade,
//!   which fixes the one configuration the ADR asks for (500 kbit/s
//!   nominal, 2 Mbit/s data, TDC auto, one TX and one RX FIFO);
//! - the acceptance filter: **promiscuous**. The controller talks to
//!   every plane, so it must accept every identifier - an answer from
//!   plane n comes in under `(RSP, n)` and an EVENT under `(EVENT, n)`.
//!   `Config::for_node(14)` would match only identifiers whose low
//!   nibble is 14, which is nothing a plane ever sends; the mcp2518fd
//!   crate documents `promiscuous()` as the controller's filter for
//!   exactly this reason;
//! - INT0 / INT1 driven low. They are open on controller-fd (U23 pins
//!   8 / 9, no pull-ups), and a floating CMOS input is a current path,
//!   so TRIS0 / TRIS1 are cleared: the 32-bit `IOCON` write that does it
//!   also clears LAT0 / LAT1 (erratum DS80000789F item 5), which is the
//!   level this board wants;
//! - the TCAN1044V's STB on GPIO0, driven low: the transceiver has an
//!   internal pull-up to VIO, so it sits in standby until the firmware
//!   says otherwise. Without this nothing reaches the bus;
//! - nINT on GPIO3, claimed as an input and read for the diagnostics.
//!   The transport polls the receive FIFO rather than waiting on the
//!   pin: the dispatcher is a 50 ms loop, a transaction waits at most
//!   20 ms, and an interrupt would buy nothing a poll does not.
//!
//! Bus-off recovery is the chip's (ADR 0005 decision 8: automatic,
//! counted); the counters are in [`diag`] and on the console's `link`
//! command.
//!
//! **Untested on hardware.** No controller-fd board exists, so nothing
//! here has seen an MCP2518FD. What a bench has to check first is in the
//! `mcp2518fd` README ("Untested on hardware").

use std::sync::Mutex;

use esp_idf_hal::delay::{Delay, Ets};
use esp_idf_hal::gpio::{Gpio0, Gpio2, Gpio3, Input, Output, PinDriver, Pull};
use esp_idf_hal::spi::config::Config as SpiConfig;
use esp_idf_hal::spi::{SpiDeviceDriver, SpiDriver};
use esp_idf_hal::units::FromValueType;
use granite_core::hal::{LinkError, PlaneLink};
use granite_core::plane::can_link::{
    BusState, CanCounters, CanFrames, CanPlaneLink, DriverError, NODE_CONTROLLER, RxFrame,
};
use mcp2518fd::mcp251xfd::registers::addr::IOCON;
use mcp2518fd::{Config as CanConfig, Mcp2518fd};

/// SPI clock of the MCP2518FD's own transactions.
///
/// The erratum ceiling is `mcp251xfd::max_spi_hz` (~17 MHz at a 40 MHz
/// SYSCLK); 10 MHz leaves room for the ribbon-free but still 2.54 mm
/// routed path on the board and makes a 70-byte transfer ~60 us.
pub const SPI_HZ: u32 = 10_000_000;

/// How long one poll of the receive FIFO waits before the next
/// \[microseconds\].
///
/// A busy wait, not a task delay: `CONFIG_FREERTOS_HZ` is 100, so the
/// shortest `vTaskDelay` is 10 ms and a 20 ms transaction would get two
/// polls. 200 us gives ~100 polls, each an SPI read of a few bytes, and
/// the whole wait is bounded by the transaction timeout.
pub const POLL_GAP_US: u32 = 200;

/// `IOCON.TRIS0` and `TRIS1`: 1 = input (power-on), 0 = output.
const IOCON_TRIS01: u32 = 0x0000_0003;

/// The SPI device the CAN controller sits on.
type CanSpi = SpiDeviceDriver<'static, &'static SpiDriver<'static>>;

/// The MCP2518FD as the transport's frame driver.
pub struct CanHw {
    can: Mcp2518fd<CanSpi>,
    /// nINT, an input. Held so nothing else claims GPIO3, and read by
    /// [`CanHw::int_low`] for the diagnostics.
    int: Option<PinDriver<'static, Input>>,
    /// The transceiver's STB, held low for normal mode.
    _stb: PinDriver<'static, Output>,
}

impl CanHw {
    /// True while nINT is asserted (active low): the chip has an
    /// interrupt flag set. Only a diagnostic; the transport polls.
    pub fn int_low(&self) -> bool {
        self.int.as_ref().is_some_and(|pin| pin.is_low())
    }

    /// The chip's interrupt flags, for the console.
    pub fn interrupt_flags(&mut self) -> Option<u32> {
        self.can.interrupt_flags().ok().map(|flags| flags.0)
    }
}

impl CanFrames for CanHw {
    fn send(&mut self, id: u16, data: &[u8]) -> Result<(), DriverError> {
        match self.can.send(id, data) {
            Ok(()) => Ok(()),
            Err(mcp2518fd::Error::WouldBlock) => Err(DriverError::WouldBlock),
            Err(e) => {
                log::warn!("can: send of {id:#05x} failed: {e:?}");
                Err(DriverError::Bus)
            }
        }
    }

    fn receive(&mut self) -> Result<Option<RxFrame>, DriverError> {
        match self.can.receive() {
            Ok(None) => Ok(None),
            Ok(Some(frame)) => Ok(RxFrame::new(frame.id, frame.payload())),
            Err(e) => {
                log::warn!("can: receive failed: {e:?}");
                Err(DriverError::Bus)
            }
        }
    }

    fn now_ms(&self) -> u64 {
        crate::platform::now_ms()
    }

    fn bus_state(&mut self) -> Result<BusState, DriverError> {
        // One register read for all three: C1TREC carries the counters
        // and the bus-off flag.
        match self.can.inner().error_counters() {
            Ok(trec) => Ok(BusState {
                tec: trec.tec(),
                rec: trec.rec(),
                bus_off: trec.tx_bus_off(),
            }),
            Err(e) => {
                log::warn!("can: error counters unreadable: {e:?}");
                Err(DriverError::Bus)
            }
        }
    }

    fn idle(&mut self) {
        Ets::delay_us(POLL_GAP_US);
    }
}

/// The plane link of an FD image.
///
/// A thin wrapper around the transport, for one reason: the counters
/// have to reach the console and the API, and the link itself lives in
/// the dispatcher thread. Every call publishes the counters into
/// [`diag`], which costs one uncontended mutex per transaction.
pub struct CanLink(CanPlaneLink<CanHw>);

impl CanLink {
    /// Claim the pins, bring the chip up and put it on the bus.
    ///
    /// `Err` means the link hardware is not there (no answer on SPI, a
    /// failed RAM echo test, a crystal that does not start). The caller
    /// decides what that means for the image: see
    /// [`crate::platform::ota::mark_link_hw`].
    pub fn new(
        spi: &'static SpiDriver<'static>,
        cs: Gpio2<'static>,
        int: Gpio3<'static>,
        stb: Gpio0<'static>,
    ) -> anyhow::Result<Self> {
        // The transceiver first: in standby it neither drives nor
        // receives, so a chip that comes up before it would see an idle
        // bus and could go error-passive on its own frames.
        let mut stb = PinDriver::output(stb)?;
        stb.set_low()?;
        log::info!("can: TCAN1044V STB low (normal mode)");

        let int = match PinDriver::input(int, Pull::Up) {
            Ok(pin) => Some(pin),
            Err(e) => {
                log::warn!("can: nINT (GPIO3) could not be claimed: {e}");
                None
            }
        };

        let device = SpiDeviceDriver::new(
            spi,
            Some(cs),
            &SpiConfig::new().baudrate(SPI_HZ.Hz()),
        )?;
        let mut delay = Delay::new_default();
        let mut can = Mcp2518fd::new(device);
        can.reset()
            .map_err(|e| anyhow::anyhow!("MCP2518FD reset failed: {e:?}"))?;
        // Promiscuous: the controller is the one node that talks to
        // every other one (see the module docs).
        let variant = can
            .init(&CanConfig::promiscuous(), &mut delay)
            .map_err(|e| anyhow::anyhow!("MCP2518FD init failed: {e:?}"))?;
        log::info!(
            "can: MCP2518FD ({variant:?}) up on SPI at {} MHz, nCS GPIO2, nINT GPIO3, node {}",
            SPI_HZ / 1_000_000,
            NODE_CONTROLLER
        );

        // INT0 / INT1 as outputs, low. The 32-bit write clears
        // LAT0 / LAT1 by erratum, which is the level we want.
        match can.inner().read_register_raw(IOCON) {
            Ok(iocon) => {
                if let Err(e) = can
                    .inner()
                    .write_register_raw(IOCON, iocon & !IOCON_TRIS01)
                {
                    log::warn!("can: INT0 / INT1 could not be driven low: {e:?}");
                }
            }
            Err(e) => log::warn!("can: IOCON unreadable, INT0 / INT1 left as inputs: {e:?}"),
        }

        let hw = CanHw {
            can,
            int,
            _stb: stb,
        };
        let mut link = CanPlaneLink::new(hw);
        link.refresh_bus_state();
        let out = CanLink(link);
        out.publish(true);
        Ok(out)
    }

    /// What the transport has counted.
    pub fn counters(&self) -> CanCounters {
        self.0.counters()
    }

    /// Copy the counters where the console and the API can read them.
    fn publish(&self, up: bool) {
        if let Ok(mut guard) = DIAG.lock() {
            *guard = Some(CanDiag {
                up,
                counters: self.0.counters(),
            });
        }
    }
}

impl PlaneLink for CanLink {
    fn write_read(&mut self, addr: u8, req: &[u8], resp: &mut [u8]) -> Result<usize, LinkError> {
        let out = self.0.write_read(addr, req, resp);
        self.publish(true);
        out
    }

    fn write(&mut self, addr: u8, frame: &[u8]) -> Result<(), LinkError> {
        let out = self.0.write(addr, frame);
        self.publish(true);
        out
    }

    fn event_nodes(&mut self) -> u8 {
        let out = self.0.event_nodes();
        self.publish(true);
        out
    }

    fn sync(&mut self, epoch: u32) -> Result<(), LinkError> {
        let out = self.0.sync(epoch);
        self.publish(true);
        out
    }
}

/// What the console and the API may know about the link.
#[derive(Debug, Clone, Copy)]
pub struct CanDiag {
    /// True once the chip came up; false while there is no link at all.
    pub up: bool,
    /// The transport's counters.
    pub counters: CanCounters,
}

/// The last published counters. `None` until the link comes up, which
/// is itself the answer to "is the CAN controller there".
static DIAG: Mutex<Option<CanDiag>> = Mutex::new(None);

/// The last published counters.
pub fn diag() -> Option<CanDiag> {
    DIAG.lock().ok().and_then(|guard| *guard)
}

/// Note that the link did not come up, so the diagnostics say so
/// instead of staying silent.
pub fn note_down() {
    if let Ok(mut guard) = DIAG.lock() {
        *guard = Some(CanDiag {
            up: false,
            counters: CanCounters::default(),
        });
    }
}

/// The console's `link` lines for an FD build.
pub fn lines() -> Vec<String> {
    let Some(diag) = diag() else {
        return vec![String::from(
            "link      canfd: the MCP2518FD never came up (no chip on SPI?)",
        )];
    };
    let c = diag.counters;
    let mut out = Vec::new();
    out.push(format!(
        "link      canfd, node {}, chip {}",
        NODE_CONTROLLER,
        if diag.up { "up" } else { "down" }
    ));
    out.push(format!(
        "frames    tx {} rx {} (into messages {}), blocked {}",
        c.tx_frames, c.rx_frames, c.rx_in_messages, c.tx_blocked
    ));
    out.push(format!(
        "messages  reads {} writes {} timeouts {} events {}",
        c.transactions, c.writes, c.timeouts, c.events
    ));
    out.push(format!(
        "dropped   foreign {} unknown-id {} tx-err {} rx-err {}",
        c.foreign, c.unknown_id, c.tx_errors, c.rx_errors
    ));
    let r = c.reassembly;
    out.push(format!(
        "reassembly out-of-order {} timeout {} overflow {} evicted {} restarted {}",
        r.out_of_order, r.timeout, r.overflow, r.evicted, r.restarted
    ));
    out.push(format!(
        "bus       tec {} rec {} bus-off {}{}",
        c.tec,
        c.rec,
        c.bus_off,
        if c.bus_off_now { " (now)" } else { "" }
    ));
    out
}
