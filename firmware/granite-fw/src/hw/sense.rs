//! Node power LEDs and dry contacts from U15 (ADR 0001 component 4,
//! "Sensing", first bullet).
//!
//! One thread owns the reads. It wakes on EXP_INT (GPIO0, falling edge,
//! open-drain and shared) and otherwise once a second as a backstop, reads
//! INTCAP and then GPIO, and publishes the result into atomics. The trait
//! methods only load those atomics, so a reader never blocks on I2C and
//! never waits for the expander mutex.
//!
//! With `expanders::IDLE_IN_RESET` set, U15 is in reset between polls and
//! cannot raise EXP_INT, so the interrupt only ever fires while a press
//! holds the reset line high and the 1 s poll is doing all the work. See
//! the module docs of [`super::expanders`] for why, and what the other
//! choice costs.
//!
//! Debounce ownership, so it is not done twice: the **core** debounces the
//! power LEDs (`granite_core::node::DEBOUNCE_MS`, 100 ms, applied in
//! `Nodes::observe`), so the LED bits published here are raw. The **dry
//! contacts** are debounced here, 50 ms on top of the 1k/100nF RC, because
//! nothing in the core does it.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

use esp_idf_hal::delay::TickType;
use esp_idf_hal::gpio::{Input, InterruptType, PinDriver};
use esp_idf_hal::task::notification::Notification;
use granite_core::hal::{DryInputs, HalError, HalResult, NodeSense};

use super::expanders::{Shared, U15_DRY_SHIFT, U15_NODE8_BIT, lock};

/// Backstop poll interval (ADR: "plus a 1 s poll as backstop").
pub const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// Dry-contact debounce (ADR: "Dry inputs are debounced 50 ms in firmware
/// on top of the RC").
pub const DRY_DEBOUNCE_MS: u64 = 50;
/// Stack for the sense thread. It does I2C and logging, no TLS, no JSON.
const STACK_SIZE: usize = 4096;

struct SenseState {
    /// Raw LED bits, bit 0 = node 1, 1 = lit. The core debounces these.
    leds: AtomicU8,
    /// Debounced dry contacts, bit 0 = input 1, 1 = closed.
    dry: AtomicU8,
    /// False until U15 has answered at least once.
    valid: AtomicBool,
    /// Monotonic ms of the last successful read, 0 = never. 32 bits
    /// because the RISC-V target has no 64-bit atomics; it wraps after
    /// about 49 days of uptime and is a diagnostic only.
    read_ms: AtomicU32,
}

/// Non-blocking reader for the LED and dry-contact bits.
///
/// Cloneable on purpose: the core wants a `&mut dyn NodeSense` and a
/// `&mut dyn DryInputs` at the same time, and both are just atomic loads.
#[derive(Clone)]
pub struct SenseHandle {
    state: Arc<SenseState>,
}

impl SenseHandle {
    /// Monotonic ms of the last successful U15 read, `None` if never.
    /// Truncated to 32 bits, see [`SenseState::read_ms`].
    pub fn last_read_ms(&self) -> Option<u32> {
        match self.state.read_ms.load(Ordering::Acquire) {
            0 => None,
            ms => Some(ms),
        }
    }

    fn checked(&self) -> HalResult<()> {
        if self.state.valid.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(HalError::NotPresent)
        }
    }
}

impl NodeSense for SenseHandle {
    fn read_leds(&mut self) -> HalResult<u8> {
        self.checked()?;
        Ok(self.state.leds.load(Ordering::Acquire))
    }
}

impl DryInputs for SenseHandle {
    fn read_dry(&mut self) -> HalResult<u8> {
        self.checked()?;
        Ok(self.state.dry.load(Ordering::Acquire))
    }
}

/// 50 ms debounce over a bit field.
struct Debounce {
    stable: u8,
    candidate: u8,
    since_ms: u64,
}

impl Debounce {
    fn new() -> Self {
        Debounce {
            stable: 0,
            candidate: 0,
            since_ms: 0,
        }
    }

    /// Feed a raw sample. Returns the stable value and whether a candidate
    /// is still waiting out its debounce (the caller then polls sooner).
    fn update(&mut self, raw: u8, now_ms: u64) -> (u8, bool) {
        if raw != self.candidate {
            self.candidate = raw;
            self.since_ms = now_ms;
        }
        if self.candidate != self.stable && now_ms.saturating_sub(self.since_ms) >= DRY_DEBOUNCE_MS
        {
            self.stable = self.candidate;
        }
        (self.stable, self.candidate != self.stable)
    }
}

/// Start the sense thread.
///
/// `int_pin` is EXP_INT (GPIO0) already turned into an input driver: the
/// line is open-drain and shared between U15 and the J8 add-ons, so it is
/// configured with a pull-up and a falling-edge interrupt here.
pub fn start(
    exp: Shared,
    mut int_pin: PinDriver<'static, Input>,
) -> anyhow::Result<SenseHandle> {
    let state = Arc::new(SenseState {
        leds: AtomicU8::new(0),
        dry: AtomicU8::new(0),
        valid: AtomicBool::new(false),
        read_ms: AtomicU32::new(0),
    });
    let handle = SenseHandle {
        state: state.clone(),
    };

    int_pin.set_interrupt_type(InterruptType::NegEdge)?;

    thread::Builder::new()
        .name("granite-sense".into())
        .stack_size(STACK_SIZE)
        .spawn(move || {
            // The notification must be created in the task that waits on
            // it, so the ISR subscription happens here too.
            let notification = Notification::new();
            let notifier = notification.notifier();
            // Safety: the callback only touches a FreeRTOS task notify,
            // which is ISR safe, and allocates nothing.
            if let Err(err) = unsafe {
                int_pin.subscribe(move || {
                    notifier.notify_and_yield(NonZeroU32::MIN);
                })
            } {
                log::error!("sense: subscribing to EXP_INT failed: {err}");
            }
            if let Err(err) = int_pin.enable_interrupt() {
                log::error!("sense: enabling the EXP_INT interrupt failed: {err}");
            }

            let mut dry = Debounce::new();
            let mut next_verify_ms = 0u64;
            loop {
                let woken = notification.wait(TickType::from(POLL_INTERVAL).ticks());
                // ESP-IDF disables the pin interrupt on each trigger.
                if woken.is_some()
                    && let Err(err) = int_pin.enable_interrupt()
                {
                    log::error!("sense: re-enabling the EXP_INT interrupt failed: {err}");
                }

                let now_ms = super::clock::now_ms();
                let pending = read_once(&exp, &state, &mut dry, now_ms, woken.is_some());

                // The 1 s expander readback from ADR component 2 rides
                // along on this thread: it already holds the lock here.
                if now_ms >= next_verify_ms {
                    next_verify_ms = now_ms + 1000;
                    let _ = lock(&exp).verify();
                }

                if pending {
                    thread::sleep(Duration::from_millis(DRY_DEBOUNCE_MS));
                    let now_ms = super::clock::now_ms();
                    read_once(&exp, &state, &mut dry, now_ms, false);
                }
            }
        })?;

    Ok(handle)
}

/// One read of U15. Returns true when a dry-contact change is still inside
/// its debounce window and the caller should sample again shortly.
fn read_once(
    exp: &Shared,
    state: &SenseState,
    dry: &mut Debounce,
    now_ms: u64,
    from_interrupt: bool,
) -> bool {
    let mut guard = lock(exp);
    let raw = guard.with_power(|exp| {
        let u15 = exp.u15();
        // INTCAP first (it is what the interrupt captured and reading it
        // releases the shared EXP_INT line), then GPIO for the state now.
        let captured = u15.read_intcap()?;
        let live = u15.read_gpio()?;
        Ok((captured, live))
    });
    // Dropping the reset line again is `idle`'s job and only happens when
    // no press is in progress.
    let idle = guard.idle();
    drop(guard);

    let (captured, live) = match raw {
        Ok(pair) => pair,
        Err(err) => {
            if state.valid.swap(false, Ordering::AcqRel) {
                log::error!("sense: U15 read failed: {err}");
            }
            return false;
        }
    };
    if let Err(err) = idle {
        log::error!("sense: parking the expanders after a read failed: {err}");
    }
    if from_interrupt && captured != live {
        log::debug!("sense: EXP_INT captured 0x{captured:04x}, now 0x{live:04x}");
    }

    state.leds.store(leds_from_gpio(live), Ordering::Release);
    let (stable, pending) = dry.update(dry_from_gpio(live), now_ms);
    state.dry.store(stable, Ordering::Release);
    state.read_ms.store((now_ms as u32).max(1), Ordering::Release);
    if !state.valid.swap(true, Ordering::AcqRel) {
        log::info!("sense: U15 answering, leds 0x{:02x} dry 0x{stable:02x}", leds_from_gpio(live));
    }
    pending
}

/// NODE_ON1-7 on GPA0-6, NODE_ON8 on GPB4, active low (LED lit = low).
pub fn leds_from_gpio(word: u16) -> u8 {
    let mut leds = 0u8;
    for node in 0..7u8 {
        if word & (1 << node) == 0 {
            leds |= 1 << node;
        }
    }
    if word & (1 << U15_NODE8_BIT) == 0 {
        leds |= 1 << 7;
    }
    leds
}

/// DRY_IN1-4 on GPB0-3, active low (contact to GND = closed = low).
pub fn dry_from_gpio(word: u16) -> u8 {
    let mut dry = 0u8;
    for input in 0..4u8 {
        if word & (1 << (U15_DRY_SHIFT + input)) == 0 {
            dry |= 1 << input;
        }
    }
    dry
}
