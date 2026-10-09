//! Hardware abstraction: the only surface the core uses to touch a board.
//!
//! Every trait here is synchronous and takes `&mut self`. There is no
//! `async` and no `async_trait`: the firmware drives the core from plain
//! FreeRTOS tasks, and the host tests drive it from a loop, so a blocking
//! call plus a monotonic clock is all the core needs. Nothing in this
//! module depends on ESP-IDF.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::NodeId;

/// Sentinel for "no reading" in a centi-degree temperature. Matches the
/// Modbus convention in the ADR (0x8000 in a signed 16-bit register).
pub const TEMP_MISSING: i16 = i16::MIN;

/// The two momentary switches wired per node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Switch {
    /// Power button.
    Pwr,
    /// Reset button.
    Rst,
}

impl Switch {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Switch::Pwr => "pwr",
            Switch::Rst => "rst",
        }
    }
}

impl fmt::Display for Switch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Anything a HAL implementation can fail with.
///
/// The variants are deliberately coarse: the core only distinguishes
/// "the expander lied to us" (which fails an action and triggers a reset
/// pulse in the firmware) from the rest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HalError {
    /// I2C / SPI / 1-wire transport error.
    Bus,
    /// A register readback did not match the shadow, or the press deadline
    /// fired. This is the class the ADR calls `ExpanderFault`.
    ExpanderFault,
    /// Addressed device or probe is not on the bus.
    NotPresent,
    /// Argument out of range (bad node id, bad duration).
    OutOfRange,
    /// The operation did not complete in time.
    Timeout,
    /// Persistent storage failed (NVS read/write/erase).
    Storage,
    /// Anything else, with context for the log.
    Other(String),
}

impl fmt::Display for HalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HalError::Bus => f.write_str("bus error"),
            HalError::ExpanderFault => f.write_str("expander fault"),
            HalError::NotPresent => f.write_str("not present"),
            HalError::OutOfRange => f.write_str("out of range"),
            HalError::Timeout => f.write_str("timeout"),
            HalError::Storage => f.write_str("storage error"),
            HalError::Other(m) => write!(f, "{m}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for HalError {}

/// Result of a HAL call.
pub type HalResult<T> = Result<T, HalError>;

/// The 16 photoMOS relays on U14: one PWR and one RST per node.
///
/// Implementations hold U14 in reset (GPIO10 low) whenever no press is in
/// progress, so `assert` implies "take U14 out of reset first" and
/// `release_all` implies "drop it back into reset".
pub trait NodeSwitches {
    /// Close the relay for `sw` on `node` (press the button).
    fn assert(&mut self, node: NodeId, sw: Switch) -> HalResult<()>;

    /// Open the relay for `sw` on `node` (release the button).
    fn release(&mut self, node: NodeId, sw: Switch) -> HalResult<()>;

    /// Open every relay. Must be safe to call at any time, including from
    /// a fault path, and must not depend on the I2C bus being healthy
    /// (the reset line alone does it).
    fn release_all(&mut self) -> HalResult<()>;

    /// Arm the hardware press deadline: after `ms` the implementation
    /// drops the expander reset line without any further software
    /// involvement. The core computes the value (see
    /// [`crate::actuator::deadline_for`]) and also enforces it in
    /// software as a backstop.
    ///
    /// The default implementation is a no-op so host fakes and simulators
    /// need not model the timer; a real board must override it.
    fn arm_deadline(&mut self, ms: u32) -> HalResult<()> {
        let _ = ms;
        Ok(())
    }

    /// Cancel an armed deadline. Default: no-op, see [`Self::arm_deadline`].
    fn disarm_deadline(&mut self) -> HalResult<()> {
        Ok(())
    }
}

/// The 8 node power LEDs read through U15.
pub trait NodeSense {
    /// Bit `n` (0-based) is node `n + 1`; true = LED lit = node powered.
    fn read_leds(&mut self) -> HalResult<u8>;
}

/// The 4 dry-contact inputs read through U15.
pub trait DryInputs {
    /// Bit `n` (0-based) is input `n + 1`; true = contact closed.
    fn read_dry(&mut self) -> HalResult<u8>;
}

/// A DS18B20 ROM id.
pub type RomId = u64;

/// Format a ROM id the way it appears in config and on the wire.
pub fn rom_id_hex(rom: RomId) -> String {
    use core::fmt::Write as _;
    let mut s = String::new();
    let _ = write!(s, "{rom:016x}");
    s
}

/// Parse a ROM id from its hex form.
pub fn rom_id_from_hex(s: &str) -> Option<RomId> {
    let t = s.trim().trim_start_matches("0x");
    if t.is_empty() || t.len() > 16 {
        return None;
    }
    RomId::from_str_radix(t, 16).ok()
}

/// The 1-wire probe bus.
pub trait Probes {
    /// Enumerate the ROM ids currently on the bus.
    fn scan(&mut self) -> HalResult<Vec<RomId>>;

    /// Read one probe, in centi-degrees Celsius.
    fn read(&mut self, rom: RomId) -> HalResult<i16>;
}

/// The on-board TMP1075.
pub trait BoardTemp {
    /// Board temperature in centi-degrees Celsius.
    fn read_centi_c(&mut self) -> HalResult<i16>;
}

/// The VIN divider on the ADC.
pub trait BusVoltage {
    /// Bus voltage in millivolts, divider and per-board trim applied.
    fn read_mv(&mut self) -> HalResult<u32>;
}

/// What the status LED should be doing. The ADR fixes the meanings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedPattern {
    /// Dark.
    Off,
    /// Solid on: a relay press is in progress.
    Press,
    /// 1 Hz: healthy.
    Heartbeat,
    /// 4 Hz: an OTA image is pending validation.
    OtaPending,
    /// 0.25 Hz: no Ethernet link.
    NoLink,
}

/// The single status LED.
pub trait StatusLed {
    /// Set the pattern; the implementation owns the blinking.
    fn set(&mut self, pattern: LedPattern) -> HalResult<()>;
}

/// Monotonic time. Never wall clock: the core compares and subtracts.
pub trait Clock {
    /// Milliseconds since boot, monotonic, never decreasing.
    fn now_ms(&self) -> u64;
}

/// Namespaced blob storage (NVS on the board).
pub trait Persist {
    /// Read a blob, `None` if the key is absent.
    fn get(&mut self, namespace: &str, key: &str) -> HalResult<Option<Vec<u8>>>;

    /// Write a blob.
    fn set(&mut self, namespace: &str, key: &str, value: &[u8]) -> HalResult<()>;

    /// Remove a key; absent is not an error.
    fn remove(&mut self, namespace: &str, key: &str) -> HalResult<()>;

    /// Erase a whole namespace (factory reset keeps `factory`).
    fn erase_namespace(&mut self, namespace: &str) -> HalResult<()>;
}

/// Why the controller is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootReason {
    /// Power applied.
    PowerOn,
    /// Software reset (reboot command, OTA).
    Software,
    /// Task watchdog.
    TaskWatchdog,
    /// Interrupt / RTC watchdog.
    IntWatchdog,
    /// Brown-out detector.
    BrownOut,
    /// Panic or exception, usually with a core dump.
    Panic,
    /// Could not be determined.
    Unknown,
}

impl BootReason {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            BootReason::PowerOn => "power_on",
            BootReason::Software => "software",
            BootReason::TaskWatchdog => "task_watchdog",
            BootReason::IntWatchdog => "int_watchdog",
            BootReason::BrownOut => "brown_out",
            BootReason::Panic => "panic",
            BootReason::Unknown => "unknown",
        }
    }
}

impl fmt::Display for BootReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// The sink a firmware image is streamed into is
// [`crate::api::OtaSink`]: it is part of the HTTP API's contract
// (begin / write / finish / abort), not of the hardware abstraction, and
// one public `OtaSink` is enough.

/// Restarting the controller.
pub trait Reboot {
    /// Ask for a restart. Implementations may return (the caller then
    /// stops touching hardware) or never return.
    fn request_reboot(&mut self) -> HalResult<()>;

    /// Why this boot happened.
    fn boot_reason(&self) -> BootReason;
}
