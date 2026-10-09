//! The 16 photoMOS relays on U14, with the hardware press deadline
//! (ADR 0001 component 3, "Node actuator", firmware side).
//!
//! The core owns the action state machine; this module owns exactly one
//! invariant: **no relay stays closed longer than the deadline, whatever
//! the firmware does next.** Three independent mechanisms back it up, in
//! the order they bite:
//!
//! 1. [`NodeSwitches::release`] and [`NodeSwitches::release_all`] on the
//!    normal path.
//! 2. An `esp_timer` one-shot armed before every assert. Its callback runs
//!    in the high-priority esp_timer task and does nothing but drive
//!    GPIO10 low through `gpio_set_level`: no I2C, no allocation, no lock.
//!    That resets U14 and opens all 16 outputs.
//! 3. A `std::panic` hook that drives GPIO10 low before anything else.
//!    `CONFIG_ESP_TASK_WDT_PANIC=y` (firmware/granite-fw/sdkconfig.defaults)
//!    means a task-watchdog timeout panics, so the watchdog path goes
//!    through this hook too. Below that, the pull-down on EXP_nRESET_INT
//!    covers chip reset and high-Z.
//!
//! If the deadline ever fires, the next call into this module reports
//! [`HalError::ExpanderFault`] once and reinitialises U14, so the action
//! that was in flight fails instead of silently continuing.

use std::ffi::c_void;
use std::panic;
use std::ptr;
use std::sync::Once;
use std::sync::atomic::{AtomicBool, Ordering};

use esp_idf_sys::{
    ESP_ERR_INVALID_STATE, EspError, esp, esp_timer_create, esp_timer_create_args_t,
    esp_timer_dispatch_t_ESP_TIMER_TASK, esp_timer_handle_t, esp_timer_start_once, esp_timer_stop,
};
use granite_core::actuator::DEADLINE_MAX_MS;
use granite_core::hal::{HalError, HalResult, NodeSwitches, Switch};
use granite_core::{NodeId, node_index};

use super::expanders::{Shared, force_reset_low, lock};

/// Set by the deadline callback. Read and cleared by the next call into
/// [`Relays`], which then reports the fault and reinitialises U14.
static DEADLINE_FIRED: AtomicBool = AtomicBool::new(false);

static PANIC_HOOK: Once = Once::new();

/// Install the panic hook that releases the relays.
///
/// Idempotent. Called by [`crate::hw::init`]; chains to whatever hook was
/// installed before, so the ESP-IDF backtrace still prints.
pub fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            // First thing, before any formatting or allocation: open every
            // relay by resetting U14.
            force_reset_low();
            previous(info);
        }));
        log::info!("relays: panic hook installed, it drops EXP_nRESET_INT first");
    });
}

/// The deadline timer callback: the whole point is that this does almost
/// nothing.
unsafe extern "C" fn deadline_expired(_arg: *mut c_void) {
    force_reset_low();
    DEADLINE_FIRED.store(true, Ordering::Release);
}

struct Deadline {
    handle: esp_timer_handle_t,
}

// The handle is only ever passed back to esp_timer, which is thread safe.
unsafe impl Send for Deadline {}

impl Deadline {
    fn new() -> Result<Self, EspError> {
        let args = esp_timer_create_args_t {
            callback: Some(deadline_expired),
            arg: ptr::null_mut(),
            dispatch_method: esp_timer_dispatch_t_ESP_TIMER_TASK,
            name: c"granite-press".as_ptr(),
            skip_unhandled_events: false,
        };
        let mut handle: esp_timer_handle_t = ptr::null_mut();
        esp!(unsafe { esp_timer_create(&args, &mut handle) })?;
        Ok(Deadline { handle })
    }

    fn arm(&self, ms: u32) -> Result<(), EspError> {
        self.disarm()?;
        let us = u64::from(ms.min(DEADLINE_MAX_MS)) * 1000;
        esp!(unsafe { esp_timer_start_once(self.handle, us) })
    }

    fn disarm(&self) -> Result<(), EspError> {
        match esp!(unsafe { esp_timer_stop(self.handle) }) {
            // Not running: nothing to stop.
            Err(err) if err.code() == ESP_ERR_INVALID_STATE => Ok(()),
            other => other,
        }
    }
}

/// [`NodeSwitches`] over U14.
pub struct Relays {
    exp: Shared,
    deadline: Deadline,
}

impl Relays {
    /// Build the relay driver over the shared expanders.
    pub fn new(exp: Shared) -> Result<Self, EspError> {
        install_panic_hook();
        Ok(Relays {
            exp,
            deadline: Deadline::new()?,
        })
    }

    /// Bit of U14's output word for one switch: GPA0-7 = PWR1-8 (bits
    /// 0-7), GPB0-7 = RST1-8 (bits 8-15).
    fn bit_for(node: NodeId, sw: Switch) -> HalResult<u8> {
        let index = node_index(node).ok_or(HalError::OutOfRange)? as u8;
        Ok(match sw {
            Switch::Pwr => index,
            Switch::Rst => index + 8,
        })
    }

    /// Consume the deadline-fired flag. The first caller after a deadline
    /// gets the fault; U14 is reinitialised on the next assert because
    /// `force_reset_low` left the reset-forced flag behind.
    fn take_fault(&self) -> HalResult<()> {
        if DEADLINE_FIRED.swap(false, Ordering::AcqRel) {
            log::error!("relays: press deadline fired, U14 was reset to release the relays");
            let mut exp = lock(&self.exp);
            exp.set_press_active(false);
            let _ = exp.reset_cycle();
            let _ = exp.idle();
            return Err(HalError::ExpanderFault);
        }
        Ok(())
    }
}

impl NodeSwitches for Relays {
    fn assert(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        let bit = Self::bit_for(node, sw)?;
        self.take_fault()?;

        let mut exp = lock(&self.exp);
        // Taken before the write so the reset line stays high for the
        // whole press even if a sense read lands in between.
        exp.set_press_active(true);
        let result = exp.with_power(|exp| exp.u14().set_bit(bit, true));
        if result.is_err() {
            exp.set_press_active(false);
            let _ = exp.idle();
            let _ = self.deadline.disarm();
        } else {
            log::info!("relays: node {node} {sw} asserted");
        }
        result
    }

    fn release(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        let bit = Self::bit_for(node, sw)?;
        let fault = self.take_fault();

        let mut exp = lock(&self.exp);
        let result = exp.with_power(|exp| exp.u14().set_bit(bit, false));
        if result.is_ok() {
            log::info!("relays: node {node} {sw} released");
        }
        fault.and(result)
    }

    fn release_all(&mut self) -> HalResult<()> {
        // Disarm first: the timer must not fire into a board that is
        // already parked, which would leave a fault flag behind for an
        // action that completed normally.
        let disarm = self
            .deadline
            .disarm()
            .map_err(|err| {
                log::error!("relays: disarming the deadline failed: {err}");
                HalError::Other(format!("esp_timer_stop: {err}"))
            });
        let fault = self.take_fault();

        let mut exp = lock(&self.exp);
        exp.set_press_active(false);
        // `idle` clears U14's latches, verifies them and then drops
        // EXP_nRESET_INT, so the relays are open whichever half works.
        let idle = exp.idle();
        disarm.and(fault).and(idle)
    }

    fn arm_deadline(&mut self, ms: u32) -> HalResult<()> {
        self.deadline.arm(ms).map_err(|err| {
            log::error!("relays: arming the {ms} ms deadline failed: {err}");
            HalError::Other(format!("esp_timer_start_once: {err}"))
        })
    }

    fn disarm_deadline(&mut self) -> HalResult<()> {
        self.deadline.disarm().map_err(|err| {
            log::error!("relays: disarming the deadline failed: {err}");
            HalError::Other(format!("esp_timer_stop: {err}"))
        })
    }
}
