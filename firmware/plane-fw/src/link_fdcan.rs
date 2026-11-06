//! The controller link over the G0B1's own FDCAN (ADR 0005 decision 9,
//! hardware ADR 0006 decision 6: the G0B1 build).
//!
//! The transport is `plane_logic::can_link`, the same host-tested code
//! the MCP2518FD build drives; this module is the frame driver under it
//! and the task that turns the crank. No MCP2518FD is fitted on this
//! build, so SPI1 and PA0 are left alone and PB8 / PB9 are FDCAN1_RX /
//! FDCAN1_TX AF3.
//!
//! What is configured, all of it ADR 0005 decision 8:
//!
//! - **Kernel clock 40 MHz** from PLLQ (`crate::clocks`, hardware ADR
//!   0006 decision 11), which is the same clock the MCP2518FD gets from
//!   its crystal: one bit timing serves both builds.
//! - **500 kbit/s nominal at an 80 % sample point**: 80 TQ of 25 ns,
//!   TSEG1 63, TSEG2 16, SJW 16 at a prescaler of 1.
//! - **2 Mbit/s data at 75 %** with BRS on every frame: 20 TQ, TSEG1
//!   14, TSEG2 5, SJW 5.
//! - **Transmitter delay compensation** with TDCO = 14 mtq (the data
//!   phase's TSEG1, the usual setting and the one `mcp2518fd` derives).
//!   embassy-stm32 0.6 carries a `transceiver_delay_compensation` flag
//!   but never writes `DBTP.TDC` (`can/fd/config.rs` leaves it a TODO),
//!   so [`enable_tdc`] does it through the PAC.
//! - **One acceptance filter** on this plane's node id, mask 0x00F, into
//!   RX FIFO 0, and every non-matching frame rejected: the same filter
//!   `mcp2518fd::Config::for_node` sets.
//! - **Automatic bus-off recovery**, counted by the transport
//!   (embassy's own default, named here because the ADR asks for it).
//!
//! Frames go through embassy's buffered FD wrapper, whose channels have
//! the non-blocking `try_receive` / `try_write` the synchronous
//! [`CanFrames`] trait needs; the unbuffered `Can` only offers `async`
//! reads.

use embassy_stm32::can::enums::BusErrorMode;
use embassy_stm32::can::filter::{Action, FilterType, StandardFilter, StandardFilterSlot};
use embassy_stm32::can::frame::{FdFrame, Header};
use embassy_stm32::can::{
    BufferedCanFd, CanConfigurator, IT0InterruptHandler, IT1InterruptHandler, RxFdBuf, TxFdBuf,
    config::{DataBitTiming, FrameTransmissionConfig, GlobalFilter, NominalBitTiming},
};
use embassy_stm32::{Peri, bind_interrupts, pac, peripherals};
use embassy_time::{Duration, Instant, with_timeout};
use embedded_can::{Id, StandardId};
use plane_logic::can_link::{CanFrames, CanLink, RawFrame, TxError};
use static_cell::StaticCell;

use crate::PlaneMutex;
use crate::pins;
use crate::status;

bind_interrupts!(struct Irqs {
    TIM16_FDCAN_IT0 => IT0InterruptHandler<peripherals::FDCAN1>;
    TIM17_FDCAN_IT1 => IT1InterruptHandler<peripherals::FDCAN1>;
});

/// Frames the transmit channel holds. A message is at most five
/// segments (ADR 0005 decision 3), so eight is one whole answer plus an
/// EVENT frame and slack.
const TX_FRAMES: usize = 8;

/// Frames the receive channel holds.
const RX_FRAMES: usize = 8;

/// Transmitter delay compensation offset \[mtq\]: the data phase's
/// TSEG1, which is where the receiver samples.
const TDCO: u8 = 14;

/// How long a quiet bus waits before the task runs its schedules
/// anyway (the EVENT period and the counter sample are 1 s).
const IDLE_MS: u64 = 2;

/// 500 kbit/s at the 40 MHz kernel clock, 80 % sample point.
const NOMINAL: NominalBitTiming = NominalBitTiming {
    prescaler: nonzero_u16(1),
    seg1: nonzero_u8(63),
    seg2: nonzero_u8(16),
    sync_jump_width: nonzero_u8(16),
};

/// 2 Mbit/s at the 40 MHz kernel clock, 75 % sample point. The flag is
/// set for the record; [`enable_tdc`] is what reaches the register.
const DATA: DataBitTiming = DataBitTiming {
    transceiver_delay_compensation: true,
    prescaler: nonzero_u16(1),
    seg1: nonzero_u8(14),
    seg2: nonzero_u8(5),
    sync_jump_width: nonzero_u8(5),
};

/// `NonZeroU8` in a const, with the zero case a compile-time panic.
const fn nonzero_u8(n: u8) -> core::num::NonZeroU8 {
    match core::num::NonZeroU8::new(n) {
        Some(n) => n,
        None => panic!("a bit timing field is never zero"),
    }
}

/// `NonZeroU16` in a const, with the zero case a compile-time panic.
const fn nonzero_u16(n: u16) -> core::num::NonZeroU16 {
    match core::num::NonZeroU16::new(n) {
        Some(n) => n,
        None => panic!("a prescaler is never zero"),
    }
}

static TX_BUF: StaticCell<TxFdBuf<TX_FRAMES>> = StaticCell::new();
static RX_BUF: StaticCell<RxFdBuf<RX_FRAMES>> = StaticCell::new();

/// What the task needs out of [`crate::pins::CoreBoard`].
pub struct LinkParts {
    /// FDCAN1.
    pub fdcan1: Peri<'static, peripherals::FDCAN1>,
    /// CAN_RX (ISO1044 RXD).
    pub rx: Peri<'static, pins::CAN_RX>,
    /// CAN_TX (ISO1044 TXD).
    pub tx: Peri<'static, pins::CAN_TX>,
}

/// FDCAN1 as the transport's frame driver.
pub struct Fdcan {
    inner: BufferedCanFd<'static, TX_FRAMES, RX_FRAMES>,
}

impl CanFrames for Fdcan {
    // Nothing the peripheral does fails the way an SPI bus can: a send
    // either fits the channel or does not, and a receive either has a
    // frame or has not.
    type Error = core::convert::Infallible;

    fn send(&mut self, id: u16, data: &[u8]) -> Result<(), TxError<Self::Error>> {
        let Some(id) = StandardId::new(id) else {
            // An 11-bit id out of range cannot come from `CanId::raw`.
            return Ok(());
        };
        let header = Header::new_fd(Id::Standard(id), data.len() as u8, false, true);
        let Ok(frame) = FdFrame::new(header, data) else {
            // `segment` pads every frame to a valid CAN FD length, so
            // this is unreachable; dropping it is better than a panic on
            // a board with no console.
            return Ok(());
        };
        self.inner
            .writer()
            .try_write(frame)
            .map_err(|_| TxError::Full)
    }

    fn receive(&mut self) -> Result<Option<RawFrame>, Self::Error> {
        let reader = self.inner.reader();
        loop {
            match reader.try_receive() {
                Ok(Ok(envelope)) => match envelope.frame.id() {
                    Id::Standard(id) => {
                        let id = id.as_raw();
                        return Ok(Some(RawFrame::new(id, envelope.frame.data())));
                    }
                    // The filter is standard-only, so this cannot
                    // happen; skipping it keeps the drain going.
                    Id::Extended(_) => continue,
                },
                // A bus error the peripheral put in the channel instead
                // of a frame. What the ADR reports is the error
                // counters, and those are read from the peripheral, so
                // the entry is dropped and the drain continues.
                Ok(Err(_)) => continue,
                // The channel is empty: that, and only that, is what
                // ends the transport's drain loop.
                Err(_) => return Ok(None),
            }
        }
    }

    fn error_counters(&mut self) -> Result<(u8, u8), Self::Error> {
        let p = self.inner.properties();
        Ok((p.tx_error_count(), p.rx_error_count()))
    }

    fn is_bus_off(&mut self) -> Result<bool, Self::Error> {
        Ok(matches!(
            self.inner.properties().bus_error_mode(),
            BusErrorMode::BusOff
        ))
    }
}

/// Turn on transmitter delay compensation.
///
/// `DBTP.TDC` and `TDCR.TDCO` are only writable with CCCR.INIT and
/// CCCR.CCE set, and embassy rewrites the whole of DBTP on its way out
/// of configuration mode, so this cannot be done before `start`: the
/// node goes back into initialisation for the handful of cycles it takes
/// and comes out again. Nothing has been sent or received at that point
/// (it is the first thing after `into_normal_mode`), so no traffic is
/// lost, and embassy's own state does not describe DBTP, so nothing
/// there goes stale.
fn enable_tdc(tdco: u8) {
    let can = pac::FDCAN1;
    can.cccr().modify(|w| w.set_init(true));
    while !can.cccr().read().init() {}
    can.cccr().modify(|w| w.set_cce(true));
    can.tdcr().modify(|w| {
        w.set_tdco(tdco);
        w.set_tdcf(0);
    });
    can.dbtp().modify(|w| w.set_tdc(true));
    can.cccr().modify(|w| w.set_cce(false));
    can.cccr().modify(|w| w.set_init(false));
    while can.cccr().read().init() {}
}

/// Answer the controller over CAN FD.
#[embassy_executor::task]
pub async fn task(plane: &'static PlaneMutex, node: u8, parts: LinkParts) {
    let mut configurator = CanConfigurator::new(parts.fdcan1, parts.rx, parts.tx, Irqs);

    let mut config = configurator.config();
    config.nbtr = NOMINAL;
    config.dbtr = DATA;
    // CAN FD with bit rate switching: every frame of this link carries
    // its payload at 2 Mbit/s.
    config.frame_transmit = FrameTransmissionConfig::AllowFdCanAndBRS;
    // The acceptance filter below is the only way in.
    config.global_filter = GlobalFilter::reject_all();
    config.automatic_bus_off_recovery = true;
    configurator.set_config(config);

    // id = (type << 4) | node (ADR 0005 decision 2): match the node
    // field and take any message type, which is what a plane has to see
    // (REQ and BULK for itself; the transport drops the rest).
    configurator.properties().set_standard_filter(
        StandardFilterSlot::_0,
        StandardFilter {
            filter: FilterType::BitMask {
                filter: u16::from(node & 0x0F),
                mask: 0x00F,
            },
            action: Action::StoreInFifo0,
        },
    );

    let can = configurator.into_normal_mode();
    enable_tdc(TDCO);
    defmt::info!("FDCAN1 up, node {}", node);

    let mut can = Fdcan {
        inner: can.buffered_fd(TX_BUF.init(TxFdBuf::new()), RX_BUF.init(RxFdBuf::new())),
    };
    let mut link: CanLink = CanLink::new(node);

    loop {
        {
            // The buffered channel wakes this as soon as a frame lands;
            // the timeout is what runs the EVENT and counter schedules
            // on a quiet bus. Waiting on readiness rather than on the
            // frame itself leaves the frame in the channel, where the
            // transport's own drain loop takes it.
            let reader = can.inner.reader();
            let _ = with_timeout(
                Duration::from_millis(IDLE_MS),
                core::future::poll_fn(|cx| reader.poll_ready_to_receive(cx)),
            )
            .await;
        }

        let now_ms = Instant::now().as_millis() as u32;
        let mut plane = plane.lock().await;
        link.poll(now_ms, &mut plane, &mut can);
        plane.set_can_errors(link.can_errors());
        status::publish(&plane);
    }
}
