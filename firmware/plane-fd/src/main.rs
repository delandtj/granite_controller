//! plane-fd: the plane core application for the PP-48 core-fd board.
//!
//! The FD-only plane binary of ADR 0005 decision 9: one CAN transport
//! per image, chosen by a cargo feature (`compile_error!` below if the
//! set is not exactly one). The rev D I2C binary is frozen in
//! attic/firmware/plane-fw.
//!
//! | Feature | Board | MCU | Link |
//! |---|---|---|---|
//! | `link-mcp` | pp-48-core-fd | STM32G070CB | MCP2518FD on SPI1, nINT on PA0 |
//! | `link-fdcan` | pp-48-core-fd | STM32G0B1CC | FDCAN1 on PB8 / PB9 AF3 |
//!
//! The link decides the MCU, so the feature carries the embassy-stm32
//! chip feature too, and `build.rs` picks the linker script and the
//! generated pin map to check against.
//!
//! The behaviour is not here. It is in `plane-logic`, which is
//! hardware-free, host-tested and shared with granite-sim's fake plane
//! (ADR 0002 decision 1); this crate is the glue that gives it a board:
//!
//! - [`pins`]: every net of the board's generated pin map, checked
//!   against that file by `build.rs`.
//! - [`segment_bus`]: I2C2 master behind the TCA9548A, as
//!   `plane_logic::SegmentBus`.
//! - [`core_pins`]: PRESS_TRIG, EXP_RSTN and the VIN_SNS ADC.
//! - [`backup_regs`] / [`config_store`] / [`update_store`]: the TAMP
//!   words, the two CONFIG pages, and the DFU slot (not yet).
//! - the link module of the build: `link_mcp` or `link_fdcan`, both
//!   `plane_logic::can_link` over a CAN FD controller, ending in
//!   `Plane::handle_frame`.
//! - [`status`]: LED_RUN. Events go out as EVENT frames (ADR 0005
//!   decision 6); there is no INT_OUT and PD2 is free.
//!
//! Three tasks, as ADR 0002 decision 1 splits them: the bus task owns
//! I2C2 and the whole plane state (`Plane::tick`), the link task
//! answers the controller (`Plane::handle_frame`), the status task
//! drives the LED. The plane itself lives behind one mutex, and neither
//! holder awaits while it has it, so the two tasks cannot interleave
//! inside the state machine.

#![no_std]
#![no_main]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

// Exactly one link per image (ADR 0005 decision 9). The link chooses
// the MCU and the linker script, so "none" and "two" are not builds:
// they are a mistake with a message.
#[cfg(not(any(feature = "link-mcp", feature = "link-fdcan")))]
compile_error!(
    "plane-fd needs exactly one link feature: link-mcp (G070 + MCP2518FD on SPI1, the default) \
     or link-fdcan (G0B1CC + FDCAN1). Building with --no-default-features and nothing else \
     selects no MCU either."
);
#[cfg(all(feature = "link-mcp", feature = "link-fdcan"))]
compile_error!(
    "plane-fd builds one link per image: link-mcp and link-fdcan are exclusive \
     (they select different MCUs and different linker scripts). The default is link-mcp, \
     so link-fdcan needs --no-default-features."
);

mod backup_regs;
mod config_store;
mod core_pins;
#[cfg(feature = "link-fdcan")]
mod link_fdcan;
#[cfg(feature = "link-mcp")]
mod link_mcp;
mod pins;
mod reset;
mod segment_bus;
mod status;
mod update_store;

use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_stm32::peripherals;
use embassy_stm32::time::Hertz;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::{Duration, Instant, Ticker};
use panic_probe as _;
use plane_logic::device::DeviceSet;
use plane_logic::{Plane, Setup};
use static_cell::StaticCell;

/// Which link this image was built with, for the boot log.
#[cfg(feature = "link-mcp")]
const LINK: &str = "CAN FD, MCP2518FD on SPI1";
/// Which link this image was built with, for the boot log.
#[cfg(feature = "link-fdcan")]
const LINK: &str = "CAN FD, FDCAN1";

/// Firmware version of this image (INFO 0x04).
const VERSION: granite_proto::Version = granite_proto::Version::new(0, 1, 0);

/// Version of the bootloader (INFO 0x0B). There is none yet: this image
/// is flashed over SWD at 0x08000000, so it reports 0.0.0.
const BOOTLOADER_VERSION: granite_proto::Version = granite_proto::Version::new(0, 0, 0);

/// Independent watchdog period (ADR 0002 decision 3).
///
/// The bus task feeds it only after a poll cycle in which the sequencer
/// also ran (`Plane::may_feed_watchdog`), so a stalled cycle resets the
/// plane and the new image releases every press on its way up.
const WATCHDOG_US: u32 = 500_000;

/// How often the bus task runs `Plane::tick`.
///
/// The poll cycle inside it is scheduled at 50 ms (ADR 0002 decision 5,
/// `plane_logic::model::POLL_PERIOD_MS`); ticking five times as often
/// costs almost nothing and gives the sequences and the PRESS_TRIG
/// toggling a 10 ms resolution instead of 50 ms.
const TICK_MS: u64 = 10;

/// Everything the plane logic touches on this board.
type PlaneDevices = DeviceSet<
    segment_bus::Bus,
    core_pins::Pins,
    backup_regs::Backup,
    config_store::Config,
    update_store::NoUpdates,
>;

/// The plane, shared by the bus task and the link task.
pub type PlaneMutex = Mutex<CriticalSectionRawMutex, Plane>;

static PLANE: StaticCell<PlaneMutex> = StaticCell::new();

/// 8 MHz HSE crystal (the pin map's `hse_hz`) to a 64 MHz SYSCLK: PLL
/// with M = 1, N = 16 (128 MHz VCO, inside the G0's 64-344 MHz range)
/// and R = 2.
///
/// The HSI stays on as the ADC's clock: the G0's ADC takes at most
/// 35 MHz, so it cannot run off the 64 MHz SYSCLK.
///
/// On the `link-mcp` build the MCP2518FD has its own 40 MHz crystal
/// (hardware ADR 0006 decision 6), so nothing of the CAN timing hangs
/// on this tree; SPI1 is divided down to 8 MHz in `link_mcp`.
#[cfg(not(feature = "link-fdcan"))]
fn clocks() -> embassy_stm32::Config {
    use embassy_stm32::rcc::{
        APBPrescaler, AHBPrescaler, Hse, HseMode, Pll, PllMul, PllPreDiv, PllRDiv, PllSource,
        Sysclk, mux,
    };

    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz::mhz(8),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV1,
        mul: PllMul::MUL16,
        divp: None,
        divq: None,
        divr: Some(PllRDiv::DIV2),
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.mux.adcsel = mux::Adcsel::HSI;
    config
}

/// The G0B1 clock tree: 8 MHz HSE, VCO 320 MHz (M = 1, N = 40), SYSCLK
/// 64 MHz off R = 5 and **FDCAN kernel clock 40 MHz off Q = 8**, which
/// is what hardware ADR 0006 decision 11 asks for (and the same 40 MHz
/// the MCP2518FD gets from its crystal on the other build, so one bit
/// timing serves both).
///
/// The G070 build's 128 MHz VCO cannot do it: Q divides by 2 to 8, so
/// 128 MHz gives 64, 42.7, 32 ... and never 40. 320 MHz is inside the
/// 96-344 MHz VCO range, R = 5 lands exactly on the 64 MHz SYSCLK
/// maximum and Q = 8 exactly on 40 MHz, with no fractional parts
/// anywhere: the bit timing of ADR 0005 decision 8 (80 TQ nominal,
/// 20 TQ data) needs the clock to be exact, not close.
///
/// The HSI stays on as the ADC clock, as on the G070.
#[cfg(feature = "link-fdcan")]
fn clocks() -> embassy_stm32::Config {
    use embassy_stm32::rcc::{
        APBPrescaler, AHBPrescaler, Hse, HseMode, Pll, PllMul, PllPreDiv, PllQDiv, PllRDiv,
        PllSource, Sysclk, mux,
    };

    let mut config = embassy_stm32::Config::default();
    config.rcc.hse = Some(Hse {
        freq: Hertz::mhz(8),
        mode: HseMode::Oscillator,
    });
    config.rcc.pll = Some(Pll {
        source: PllSource::HSE,
        prediv: PllPreDiv::DIV1,
        mul: PllMul::MUL40,
        divp: None,
        divq: Some(PllQDiv::DIV8),
        divr: Some(PllRDiv::DIV5),
    });
    config.rcc.sys = Sysclk::PLL1_R;
    config.rcc.ahb_pre = AHBPrescaler::DIV1;
    config.rcc.apb1_pre = APBPrescaler::DIV1;
    config.rcc.mux.adcsel = mux::Adcsel::HSI;
    config.rcc.mux.fdcansel = mux::Fdcansel::PLL1_Q;
    config
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let board = pins::split(embassy_stm32::init(clocks()));

    let address = reset::address(board.adr0, board.adr1, board.adr2);
    let cause = reset::cause();
    defmt::info!(
        "plane-fd at 0x{:02x} ({}), reset cause {}",
        address,
        LINK,
        cause
    );

    // The devices the plane logic drives. The bus task owns all of
    // them: it is the only task that may touch the segment bus, the
    // flash or the backup registers.
    let mut devices = DeviceSet::new(
        segment_bus::Bus::new(segment_bus::BusPins {
            i2c2: board.i2c2,
            scl: board.scl2,
            sda: board.sda2,
            mux_rst: board.mux_rst,
        }),
        core_pins::Pins::new(core_pins::CorePinParts {
            press_trig: board.press_trig,
            exp_rstn: board.exp_rstn,
            vin_sns: board.vin_sns,
            adc: board.adc,
        }),
        backup_regs::Backup::init(),
        config_store::Config::new(board.flash),
        update_store::NoUpdates,
    );

    let mut plane = Plane::new(Setup {
        address,
        serial: reset::serial(),
        firmware_version: VERSION,
        bootloader_version: BOOTLOADER_VERSION,
    });

    // ADR 0002 decision 3: adopt the intended state, then release every
    // present unit's photoMOS outputs before reading anything.
    plane.boot(Instant::now().as_millis(), &mut devices, cause);
    defmt::info!(
        "booted, units 0b{:04b}, boot count {}",
        plane.units_present().bits(),
        plane.boot_count()
    );
    status::publish(&plane);

    // The watchdog starts after boot, not before it: a four-channel
    // discovery scan with NAKs can take longer than its period, and
    // what bounds a held button during boot is the hardware press
    // deadline (PRESS_TRIG is not toggled until a press runs, so
    // PRESS_EN is already dead by the time the application starts).
    let mut watchdog = IndependentWatchdog::new(board.iwdg, WATCHDOG_US);
    watchdog.unleash();

    let plane: &'static PlaneMutex = PLANE.init(Mutex::new(plane));

    // The CAN node id is the three address straps (hardware ADR 0006
    // decision 7); the ADR 0003 address the CRC covers is 0x30 + node,
    // which is the address above.
    let node = address - reset::BASE_ADDRESS;

    #[cfg(feature = "link-mcp")]
    {
        spawner.spawn(defmt::unwrap!(status::task(board.led_run)));
        spawner.spawn(defmt::unwrap!(link_mcp::task(
            plane,
            node,
            link_mcp::LinkParts {
                spi1: board.spi1,
                sck: board.spi_sck,
                miso: board.spi_miso,
                mosi: board.spi_mosi,
                cs: board.mcp_cs,
                int: board.mcp_int,
                exti: board.mcp_int_exti,
            }
        )));
    }

    #[cfg(feature = "link-fdcan")]
    {
        spawner.spawn(defmt::unwrap!(status::task(board.led_run)));
        spawner.spawn(defmt::unwrap!(link_fdcan::task(
            plane,
            node,
            link_fdcan::LinkParts {
                fdcan1: board.fdcan1,
                rx: board.can_rx,
                tx: board.can_tx,
            }
        )));
    }

    bus_task(plane, devices, watchdog).await;
}

/// The bus task: the poll cycle, the queued writes and the sequences,
/// and the only owner of I2C2 (ADR 0002 decision 1).
///
/// It is `main`'s own loop rather than a fourth task, so the devices do
/// not have to be `'static`.
async fn bus_task(
    plane: &'static PlaneMutex,
    mut devices: PlaneDevices,
    mut watchdog: IndependentWatchdog<'static, peripherals::IWDG>,
) -> ! {
    let mut ticker = Ticker::every(Duration::from_millis(TICK_MS));
    loop {
        {
            let mut plane = plane.lock().await;
            plane.tick(Instant::now().as_millis(), &mut devices);
            status::publish(&plane);
            if plane.may_feed_watchdog() {
                watchdog.pet();
            }
            if plane.reset_requested() {
                // The intended state is already in the backup
                // registers, so the next boot adopts it (ADR 0002
                // decision 3, commanded reset).
                defmt::info!("reset requested");
                cortex_m::peripheral::SCB::sys_reset();
            }
        }
        ticker.next().await;
    }
}
