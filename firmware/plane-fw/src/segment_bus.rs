//! [`SegmentBus`] over I2C2: the chain of unit boards behind the
//! TCA9548A (ADR 0002 decision 4 and 5).
//!
//! The bus task is the only owner of I2C2, so the transfers are the
//! blocking ones: a byte transfer at 100 kHz is tens of microseconds,
//! shorter than the executor would need to park and wake a task, and
//! the poll cycle is budgeted as one blocking pass over the present
//! units (~25 ms at four units).

use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::i2c::{I2c, Master};
use embassy_stm32::mode::Blocking;
use embassy_stm32::pac;
use embassy_stm32::pac::gpio::vals::Moder;
use embassy_time::{Duration, block_for};
use plane_logic::device::{DevResult, DeviceError, SegmentBus};

use crate::pins;

/// How long MUX_RST is held high. The TCA9548A needs 6 ns; 20 us is
/// what a human-readable number costs here.
const MUX_RESET_US: u64 = 20;

/// Half-period of the nine recovery clocks: 100 kHz, the bus speed.
const UNSTICK_HALF_US: u64 = 5;

/// Bit of GPIOA that SCL2 sits on (PA11). The port itself is named in
/// [`pins`], but a register write needs the number.
const SCL2_BIT: usize = 11;

/// I2C2 and the two lines that are not on it.
pub struct Bus {
    i2c: I2c<'static, Blocking, Master>,
    mux_rst: Output<'static>,
}

impl Bus {
    /// Take I2C2 at 100 kHz (ADR 0002 decision 5: the chain is four
    /// board-to-board joints long) and MUX_RST low, which is the mux
    /// running.
    pub fn new(board_i2c2: BusPins) -> Self {
        let mut config = embassy_stm32::i2c::Config::default();
        config.frequency = embassy_stm32::time::Hertz::khz(100);
        let i2c = I2c::new_blocking(board_i2c2.i2c2, board_i2c2.scl, board_i2c2.sda, config);
        let mux_rst = Output::new(board_i2c2.mux_rst, Level::Low, Speed::Low);
        Bus { i2c, mux_rst }
    }
}

/// What [`Bus::new`] needs out of [`crate::pins::CoreBoard`].
pub struct BusPins {
    /// The I2C2 peripheral.
    pub i2c2: embassy_stm32::Peri<'static, embassy_stm32::peripherals::I2C2>,
    /// SCL2.
    pub scl: embassy_stm32::Peri<'static, pins::SCL2>,
    /// SDA2.
    pub sda: embassy_stm32::Peri<'static, pins::SDA2>,
    /// MUX_RST.
    pub mux_rst: embassy_stm32::Peri<'static, pins::MUX_RST>,
}

/// Map an embassy I2C error onto the three cases the plane logic
/// branches on (`plane_logic::device::DeviceError`).
fn map(e: embassy_stm32::i2c::Error) -> DeviceError {
    use embassy_stm32::i2c::Error;
    match e {
        // Discovery reads a NAK as "no unit on this channel".
        Error::Nack => DeviceError::Nak,
        // A transfer that never finishes is a slave holding the bus:
        // the channel wants the nine clocks and a MUX_RST pulse.
        Error::Timeout => DeviceError::Stuck,
        _ => DeviceError::Bus,
    }
}

impl SegmentBus for Bus {
    fn write(&mut self, addr: u8, bytes: &[u8]) -> DevResult<()> {
        self.i2c.blocking_write(addr, bytes).map_err(map)
    }

    fn read(&mut self, addr: u8, cmd: &[u8], out: &mut [u8]) -> DevResult<()> {
        if out.is_empty() {
            return Ok(());
        }
        if cmd.is_empty() {
            self.i2c.blocking_read(addr, out).map_err(map)
        } else {
            self.i2c.blocking_write_read(addr, cmd, out).map_err(map)
        }
    }

    fn reset_mux(&mut self) -> DevResult<()> {
        // Q6 inverts: the pin high is /RESET low on the part, every
        // channel off, and an MCU in reset leaves the mux running.
        self.mux_rst.set_high();
        block_for(Duration::from_micros(MUX_RESET_US));
        self.mux_rst.set_low();
        block_for(Duration::from_micros(MUX_RESET_US));
        Ok(())
    }

    fn unstick(&mut self) -> DevResult<()> {
        // Nine SCL clocks with the peripheral off, so a slave that was
        // left mid-byte finishes it and releases SDA (ADR 0002 decision
        // 4). The pins belong to I2C2, so this borrows SCL2 back from
        // the alternate function for the duration: PE = 0 first, which
        // is also what resets the peripheral's own state machine.
        let i2c = pac::I2C2;
        i2c.cr1().modify(|w| w.set_pe(false));

        let gpio = pac::GPIOA;
        gpio.bsrr().write(|w| w.set_bs(SCL2_BIT, true));
        gpio.moder().modify(|w| w.set_moder(SCL2_BIT, Moder::OUTPUT));
        for _ in 0..9 {
            gpio.bsrr().write(|w| w.set_br(SCL2_BIT, true));
            block_for(Duration::from_micros(UNSTICK_HALF_US));
            gpio.bsrr().write(|w| w.set_bs(SCL2_BIT, true));
            block_for(Duration::from_micros(UNSTICK_HALF_US));
        }
        gpio.moder()
            .modify(|w| w.set_moder(SCL2_BIT, Moder::ALTERNATE));

        i2c.cr1().modify(|w| w.set_pe(true));
        Ok(())
    }
}
