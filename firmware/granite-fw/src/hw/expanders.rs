//! The internal I2C bus, the shared expander reset line and both
//! MCP23017s (ADR 0001 component 2).
//!
//! Everything that can touch U14 or U15 goes through one
//! [`Shared`] mutex, because the two chips share EXP_nRESET_INT (GPIO10):
//! a press on U14 and a sense read on U15 cannot be allowed to fight over
//! the reset line.
//!
//! # The shared reset line
//!
//! The ADR says U14 is "held in reset through GPIO10 whenever no press is
//! in progress". GPIO10 is EXP_nRESET_INT and resets U14 *and* U15
//! (docs/controller.md, "Relay channels and node power sensing"), so an
//! idle-low line also holds the sense expander in reset: the power LEDs
//! and dry contacts can only be read while the line is high, and U15's
//! INTA can only assert while the line is high.
//!
//! This module resolves that by making "out of reset" a short-lived state
//! owned by the mutex. [`Expanders::powered`] raises the line and
//! reconfigures both chips; [`Expanders::idle`] drops it again, unless a
//! press is holding it. Dropping the line never closes a relay, and
//! neither does raising it: a just-reset MCP23017 has IODIR = 0xffff, so
//! no pin drives a photoMOS LED between the reset release and the IODIR
//! write.
//!
//! The cost is one reset plus one reconfiguration per sense read, about
//! 8 ms at 400 kHz, and a dead EXP_INT: U15 cannot assert INTA while it is
//! in reset, so between polls the sense side is blind and a dry contact
//! that closes and opens inside one poll interval is missed. Set
//! [`IDLE_IN_RESET`] to false to keep the line high between presses
//! instead (live EXP_INT, no per-read reconfiguration, U14 configured as
//! outputs with all latches low and verified once a second while idle).
//! This is the one open question in this module: see the report in
//! firmware/docs/adr/0001-firmware-architecture.md, component 2 and 4.
//!
//! One devboard-only artifact to expect in the log: with nothing on the
//! bus and no pull-ups, [`I2cBus::present`] times out instead of seeing a
//! NACK, and the ESP-IDF driver logs "probe device timeout" itself for
//! every attempt. A populated board pulls both lines up, so an absent
//! chip NACKs and neither this module nor the driver says anything beyond
//! the one error below.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use esp_idf_hal::gpio::{Output, PinDriver};
use esp_idf_sys::{EspError, gpio_set_level};
use granite_core::hal::{HalError, HalResult};

use super::i2c::I2cBus;
use super::mcp23017::{IOCON_MIRROR, IOCON_ODR, Mcp23017};

/// Internal (HP) bus clock. docs/controller.md: "The internal bus is fine
/// at 400 kHz".
pub const INT_BUS_HZ: u32 = 400_000;
/// U14, the relay expander.
pub const U14_ADDR: u8 = 0x20;
/// U15, the sense expander.
pub const U15_ADDR: u8 = 0x21;
/// GPIO number of EXP_nRESET_INT, for the ISR-safe path that cannot hold a
/// `PinDriver`.
pub const EXP_RESET_GPIO: i32 = 10;
/// How long the reset line is held low. The datasheet needs microseconds;
/// 2 ms also covers the RC on the line.
pub const RESET_LOW: Duration = Duration::from_millis(2);
/// Settling time after the reset is released, before the first transfer.
pub const RESET_SETTLE: Duration = Duration::from_millis(2);

/// Whether GPIO10 is driven low between presses. See the module docs.
// Coordinator decision 2026-10-10: GPIO10 also resets U15, and the actuator
// needs the LED sense right after a press, so the line stays high while
// idle. Safety while idle rests on OLAT = 0, verified every second, and on
// the press deadline that resets both expanders mid-press.
pub const IDLE_IN_RESET: bool = false;

/// Set by [`force_reset_low`], which is called from the press deadline
/// timer and from the panic hook. Both drive GPIO10 low without being able
/// to take the mutex, so they leave this flag behind instead and the next
/// [`Expanders::powered`] reconfigures from scratch.
static RESET_FORCED: AtomicBool = AtomicBool::new(false);

/// Drive EXP_nRESET_INT low right now, from any context.
///
/// Safe to call from an ISR, a timer callback or a panic hook: it touches
/// one GPIO register and one atomic, allocates nothing and takes no lock.
/// The pin must already be configured as an output, which it is from the
/// first lines of `main`.
pub fn force_reset_low() {
    unsafe {
        gpio_set_level(EXP_RESET_GPIO, 0);
    }
    RESET_FORCED.store(true, Ordering::Release);
}

/// True if [`force_reset_low`] has run since the last reconfiguration.
pub fn reset_was_forced() -> bool {
    RESET_FORCED.load(Ordering::Acquire)
}

/// U14: all 16 pins are outputs, GPA0-7 = PWR1-8, GPB0-7 = RST1-8.
const U14_IODIR: u16 = 0x0000;
/// U15 inputs: GPA0-6 = NODE_ON1-7, GPB0-3 = DRY_IN1-4, GPB4 = NODE_ON8,
/// GPB5-6 spare. GPA7/GPB7 are output-only on the part and stay outputs.
const U15_IODIR: u16 = 0x7f7f;
/// Internal pull-ups on U15: only the two unconnected spare pins
/// (GPB5, GPB6). NODE_ON has an external 47k (R39-R46) and DRY_IN an
/// external 10k; a parallel 100k would move the sense threshold.
const U15_GPPU: u16 = 0x6000;
/// Interrupt-on-change for the 8 node LEDs and the 4 dry contacts.
const U15_GPINTEN: u16 = 0x1f7f;

/// Bit of U15's GPIO word carrying NODE_ON8.
pub const U15_NODE8_BIT: u8 = 12;
/// First bit of the DRY_IN group in U15's GPIO word.
pub const U15_DRY_SHIFT: u8 = 8;

/// The internal bus, the reset line and both expanders.
pub struct Expanders {
    bus: I2cBus,
    reset: PinDriver<'static, Output>,
    u14: Mcp23017,
    u15: Mcp23017,
    out_of_reset: bool,
    press_active: bool,
    /// True once "no answer on the internal bus" has been logged at error
    /// level. Without this a board with a dead bus writes one error per
    /// second for ever, and those lines go to MQTT.
    absent_reported: bool,
}

/// How the rest of the firmware holds the expanders.
pub type Shared = Arc<Mutex<Expanders>>;

/// Take the expander lock, recovering from a poisoned mutex.
///
/// A panic in one task must not take relay control away from the others:
/// every field is either a shadow that is re-derived after a reset pulse
/// or the reset line itself, so the contents are never left half-written
/// in a way the next reset pulse does not fix.
pub fn lock(shared: &Shared) -> MutexGuard<'_, Expanders> {
    match shared.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            log::error!("expanders: mutex was poisoned, recovering");
            poisoned.into_inner()
        }
    }
}

impl Expanders {
    /// Claim the bus and the reset line and bring both chips up.
    ///
    /// The outer `Result` only fails when the I2C driver refuses to
    /// allocate a device handle, which does not touch the bus and so says
    /// nothing about the board. The inner one carries the bring-up result:
    /// a missing or unresponsive expander is logged and returned there,
    /// never panicked on, because the USB console and the network have to
    /// stay reachable on a board where I2C is broken.
    pub fn new(
        bus: I2cBus,
        reset: PinDriver<'static, Output>,
    ) -> Result<(Self, HalResult<()>), EspError> {
        let u14 = Mcp23017::new(bus.device(U14_ADDR, INT_BUS_HZ)?, "U14");
        let u15 = Mcp23017::new(bus.device(U15_ADDR, INT_BUS_HZ)?, "U15");

        let mut exp = Expanders {
            bus,
            reset,
            u14,
            u15,
            out_of_reset: false,
            press_active: false,
            absent_reported: false,
        };
        let result = exp.reset_cycle();
        if result.is_ok() && IDLE_IN_RESET {
            // Leave the board the way the ADR wants it: no press in
            // progress means U14 is in reset.
            let _ = exp.idle();
        }
        Ok((exp, result))
    }

    /// Pulse EXP_nRESET_INT and configure both chips from scratch.
    ///
    /// U15 is configured first (IOCON.ODR and MIRROR before GPINTEN, as
    /// docs/controller.md requires), then U14 with OLAT = 0 before
    /// IODIR = 0 so the outputs are already low when they become outputs.
    pub fn reset_cycle(&mut self) -> HalResult<()> {
        self.out_of_reset = false;
        self.u14.mark_reset();
        self.u15.mark_reset();

        if let Err(err) = self.reset.set_low() {
            log::error!("expanders: driving EXP_nRESET_INT low failed: {err}");
            return Err(HalError::Other(format!("reset line: {err}")));
        }
        thread::sleep(RESET_LOW);
        if let Err(err) = self.reset.set_high() {
            log::error!("expanders: releasing EXP_nRESET_INT failed: {err}");
            return Err(HalError::Other(format!("reset line: {err}")));
        }
        RESET_FORCED.store(false, Ordering::Release);
        thread::sleep(RESET_SETTLE);
        self.out_of_reset = true;

        if !self.bus.present(U15_ADDR) || !self.bus.present(U14_ADDR) {
            if !self.absent_reported {
                self.absent_reported = true;
                log::error!(
                    "expanders: no answer at 0x{U14_ADDR:02x}/0x{U15_ADDR:02x} on the internal bus"
                );
            } else {
                log::debug!("expanders: internal bus still silent");
            }
            self.park();
            return Err(HalError::NotPresent);
        }

        let result = self.configure_both();
        if result.is_err() {
            self.park();
        } else {
            self.absent_reported = false;
        }
        result
    }

    fn configure_both(&mut self) -> HalResult<()> {
        self.u15.configure(
            IOCON_ODR | IOCON_MIRROR,
            U15_GPPU,
            0,
            0,
            0, // INTCON = 0: interrupt on any change
            U15_GPINTEN,
            0,
            U15_IODIR,
        )?;
        self.u14
            .configure(IOCON_ODR, 0, 0, 0, 0, 0, 0, U14_IODIR)?;
        // Clear a pending INTA so EXP_INT is released before anyone arms
        // an edge-triggered handler on it.
        let _ = self.u15.read_intcap();
        log::info!("expanders: U14 and U15 configured, all relays released");
        Ok(())
    }

    /// Drop the reset line and forget the shadows, ignoring the press flag.
    /// Used on every failure path.
    fn park(&mut self) {
        if let Err(err) = self.reset.set_low() {
            log::error!("expanders: parking EXP_nRESET_INT low failed: {err}");
        }
        self.out_of_reset = false;
        self.press_active = false;
        self.u14.mark_reset();
        self.u15.mark_reset();
    }

    /// Make sure both chips are out of reset and configured, then hand out
    /// mutable access.
    pub fn powered(&mut self) -> HalResult<()> {
        if reset_was_forced() {
            log::warn!("expanders: reset line was forced low, reconfiguring");
            return self.reset_cycle();
        }
        if self.out_of_reset && self.u14.is_configured() && self.u15.is_configured() {
            return Ok(());
        }
        self.reset_cycle()
    }

    /// Return to the idle state: U14's latches cleared, then the reset line
    /// low if [`IDLE_IN_RESET`] is set and no press is in progress.
    ///
    /// Clearing OLAT first means the relays are already open before the
    /// chip loses its configuration, so a failing reset line still leaves
    /// a released board as long as I2C works, and a failing I2C bus still
    /// leaves a released board as long as the reset line works.
    pub fn idle(&mut self) -> HalResult<()> {
        if self.press_active {
            return Ok(());
        }
        let mut result = Ok(());
        if self.out_of_reset && self.u14.is_configured() {
            result = self.u14.set_olat(0).and_then(|()| self.u14.verify());
            if let Err(ref err) = result {
                log::error!("expanders: clearing U14 latches failed: {err}");
            }
        }
        if IDLE_IN_RESET || result.is_err() {
            self.park();
        }
        result
    }

    /// Mark that a press is holding the reset line high. While this is set,
    /// [`Self::idle`] does nothing.
    pub fn set_press_active(&mut self, active: bool) {
        self.press_active = active;
    }

    /// True while a press holds the reset line.
    pub fn press_active(&self) -> bool {
        self.press_active
    }

    /// True when the reset line is high and both chips are configured.
    pub fn is_live(&self) -> bool {
        self.out_of_reset && self.u14.is_configured() && self.u15.is_configured()
    }

    /// The relay expander. Call [`Self::powered`] first.
    pub fn u14(&mut self) -> &mut Mcp23017 {
        &mut self.u14
    }

    /// The sense expander. Call [`Self::powered`] first.
    pub fn u15(&mut self) -> &mut Mcp23017 {
        &mut self.u15
    }

    /// The internal bus, for the TMP1075 and for a bus scan.
    pub fn bus(&self) -> &I2cBus {
        &self.bus
    }

    /// Periodic 1 s readback (ADR component 2). A mismatch resets both
    /// chips and is reported as [`HalError::ExpanderFault`].
    pub fn verify(&mut self) -> HalResult<()> {
        if !self.is_live() {
            return Ok(());
        }
        let result = self.u14.verify().and_then(|()| self.u15.verify());
        if result.is_err() {
            let _ = self.reset_cycle();
        }
        result
    }

    /// Run `op` against the expanders with the reset line raised,
    /// reinitialising both chips through a reset pulse if `op` fails.
    ///
    /// This is the "any I2C error on the internal bus reinitialises both
    /// through a reset pulse and aborts the action in progress" rule from
    /// the ADR. The original error is what the caller sees.
    pub fn with_power<T>(
        &mut self,
        op: impl FnOnce(&mut Self) -> HalResult<T>,
    ) -> HalResult<T> {
        self.powered()?;
        match op(self) {
            Ok(value) => Ok(value),
            Err(err) => {
                log::error!("expanders: {err}, reinitialising both chips");
                let _ = self.reset_cycle();
                if !self.press_active {
                    let _ = self.idle();
                }
                Err(err)
            }
        }
    }
}
