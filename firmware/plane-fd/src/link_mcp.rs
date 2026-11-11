//! The controller link over an MCP2518FD on SPI1 (ADR 0005 decision 9,
//! hardware ADR 0006 decision 6: the G070 build).
//!
//! The transport is `plane_logic::can_link`, which is host-tested and
//! shared with the FDCAN build; this module is the three SPI calls and
//! two FIFO operations it needs ([`plane_logic::can_link::CanFrames`]),
//! plus the task that drives it.
//!
//! The chip:
//!
//! - SPI1 **blocking**, mode 0, 8 MHz (SYSCLK 64 / 8). The erratum
//!   limit is 17 MHz at the chip's 40 MHz SYSCLK (`mcp251xfd::max_spi_hz`),
//!   and 8 MHz makes a whole 64-byte frame a ~70 us burst, which is
//!   shorter than parking and waking a task would take.
//! - nCS is a software output on PA4: the G0's SPI1 NSS cannot frame a
//!   multi-byte instruction the way this chip needs, so
//!   `embedded_hal_bus::spi::ExclusiveDevice` drives the pin around each
//!   transaction.
//! - nINT on PA0, active low, as an EXTI **falling-edge** wake with a
//!   [`IDLE_MS`] timeout. The edge and not the level on purpose: nINT is
//!   a level that stays low while any enabled interrupt flag is set, and
//!   a flag this transport does not clear (an RX overflow, say) would
//!   turn a "wait while low" into a loop that never awaits and starves
//!   the bus task until the watchdog bites. Waiting on the edge always
//!   yields; the timeout is what bounds the latency of a missed edge and
//!   what runs the EVENT and counter schedules on a quiet bus.
//! - PB8 / PB9 are not touched at all. PB9 and the MCP2518FD's TXCAN
//!   drive the same ISO1044 TXD net, so this build must leave PB9 in its
//!   reset state (analog, high-Z); `pins.rs` holds both pins and
//!   hands them to nobody here.
//! - Bus-off: automatic, in the chip (ADR 0005 decision 8). The
//!   transport counts the transitions and reports them in STATUS 0x1D.

use embassy_stm32::exti::{self, ExtiInput};
use embassy_stm32::interrupt::typelevel::EXTI0_1;
use embassy_stm32::gpio::{Level, Output, Pull, Speed};
use embassy_stm32::mode::Blocking;
use embassy_stm32::spi::{self, Spi, mode::Master};
use embassy_stm32::time::Hertz;
use embassy_stm32::{Peri, bind_interrupts};
use embassy_time::{Delay, Duration, Instant, Timer, with_timeout};
use embedded_hal_bus::spi::{DeviceError, ExclusiveDevice};
use mcp2518fd::Mcp2518fd;
use plane_logic::can_link::{CanFrames, CanLink, RawFrame, TxError};

use crate::PlaneMutex;
use crate::pins;
use crate::status;

// MCP_INT is PA0, whose EXTI channel shares the EXTI0_1 vector.
bind_interrupts!(struct Irqs {
    EXTI0_1 => exti::InterruptHandler<EXTI0_1>;
});

/// SPI1 clock. 8 MHz is SYSCLK / 8 exactly and less than half the
/// chip's 17 MHz ceiling.
const SPI_HZ: u32 = 8_000_000;

/// How long the task waits for a nINT edge before it looks anyway.
///
/// Also the worst-case answer latency: the controller's read timeout is
/// 20 ms after the last REQ segment (ADR 0005 decision 5), so 2 ms of
/// polling costs a tenth of that and buys an RX FIFO that cannot sit
/// full because one edge was missed.
const IDLE_MS: u64 = 2;

/// How long the task waits before it tries the chip again after a
/// failed configuration (no chip fitted, a broken solder joint, a
/// crystal that is not running).
const RETRY_MS: u64 = 500;

/// The SPI device the driver sees: SPI1 with a software nCS.
type Device = ExclusiveDevice<Spi<'static, Blocking, Master>, Output<'static>, Delay>;

/// What that device can fail with. The CS half is `Infallible` on
/// embassy's GPIO, so every error is really an SPI1 error.
type DeviceFault = DeviceError<embassy_stm32::spi::Error, core::convert::Infallible>;

/// What the task needs out of [`crate::pins::CoreBoard`].
pub struct LinkParts {
    /// SPI1.
    pub spi1: Peri<'static, embassy_stm32::peripherals::SPI1>,
    /// SPI1 SCK.
    pub sck: Peri<'static, pins::SPI_SCK>,
    /// SPI1 MISO (the chip's SDO).
    pub miso: Peri<'static, pins::SPI_MISO>,
    /// SPI1 MOSI (the chip's SDI).
    pub mosi: Peri<'static, pins::SPI_MOSI>,
    /// nCS, driven by software.
    pub cs: Peri<'static, pins::MCP_CS>,
    /// nINT.
    pub int: Peri<'static, pins::MCP_INT>,
    /// The EXTI line of nINT.
    pub exti: Peri<'static, embassy_stm32::peripherals::EXTI0>,
}

/// The MCP2518FD as the transport's frame driver.
pub struct Mcp {
    chip: Mcp2518fd<Device>,
}

impl CanFrames for Mcp {
    type Error = mcp2518fd::Error<DeviceFault>;

    fn send(&mut self, id: u16, data: &[u8]) -> Result<(), TxError<Self::Error>> {
        match self.chip.send(id, data) {
            Ok(()) => Ok(()),
            // The facade already maps the driver's TxFifoFull onto
            // WouldBlock, which is this transport's "retry later".
            Err(mcp2518fd::Error::WouldBlock) => Err(TxError::Full),
            Err(e) => Err(TxError::Driver(e)),
        }
    }

    fn receive(&mut self) -> Result<Option<RawFrame>, Self::Error> {
        Ok(self
            .chip
            .receive()?
            .map(|frame| RawFrame::new(frame.id, frame.payload())))
    }

    fn error_counters(&mut self) -> Result<(u8, u8), Self::Error> {
        self.chip.error_counters()
    }

    fn is_bus_off(&mut self) -> Result<bool, Self::Error> {
        self.chip.is_bus_off()
    }
}

/// Answer the controller over CAN FD.
#[embassy_executor::task]
pub async fn task(plane: &'static PlaneMutex, node: u8, parts: LinkParts) {
    let mut config = spi::Config::default();
    config.frequency = Hertz(SPI_HZ);
    config.mode = spi::MODE_0;
    let bus = Spi::new_blocking(parts.spi1, parts.sck, parts.mosi, parts.miso, config);

    // nCS idles high. `ExclusiveDevice::new` is the one that drives it
    // there, and it can only fail on the pin, which on embassy's GPIO
    // is `Infallible` - so this pattern is irrefutable.
    let cs = Output::new(parts.cs, Level::High, Speed::VeryHigh);
    let Ok(device) = ExclusiveDevice::new(bus, cs, Delay);

    let mut can = Mcp {
        chip: Mcp2518fd::new(device),
    };
    // 100k pull-up on the board; the internal one costs nothing and
    // keeps the line defined if R34 is ever missing.
    let mut int = ExtiInput::new(parts.int, parts.exti, Pull::Up, Irqs);

    // One configuration, from the ADR: 40 MHz crystal, 500 kbit/s
    // nominal at 80 %, 2 Mbit/s data at 75 %, TDC on, one TX and one RX
    // FIFO, and an acceptance filter on this plane's node id.
    loop {
        match can.chip.init(&mcp2518fd::Config::for_node(node), &mut Delay) {
            Ok(_variant) => {
                defmt::info!("MCP2518FD up, node {}", node);
                break;
            }
            Err(_) => {
                // Nothing else to do: the plane runs its slots without a
                // controller (ADR 0001), the heartbeat keeps blinking
                // and the bus task keeps feeding the watchdog.
                defmt::error!("MCP2518FD not configured, retrying");
                Timer::after(Duration::from_millis(RETRY_MS)).await;
            }
        }
    }

    let mut link: CanLink = CanLink::new(node);

    loop {
        // Both arms of this yield, whatever nINT is doing (see the
        // module docs on why the edge and not the level).
        let _ = with_timeout(
            Duration::from_millis(IDLE_MS),
            int.wait_for_falling_edge(),
        )
        .await;

        let now_ms = Instant::now().as_millis() as u32;
        let mut plane = plane.lock().await;
        link.poll(now_ms, &mut plane, &mut can);
        plane.set_can_errors(link.can_errors());
        // A write can have acknowledged events (EV_ACK), and the LED
        // pattern follows an update: publish what the plane wants now.
        status::publish(&plane);
    }
}
