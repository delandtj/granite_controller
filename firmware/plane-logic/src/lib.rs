//! Hardware-free core of the Granite plane firmware.
//!
//! See `firmware/docs/adr/0002-plane-core-firmware.md`: that ADR is the
//! authority for the behaviour, and every module here points at the
//! decision it implements. This crate holds everything that can be
//! reasoned about and tested without a board, so the same code runs on
//! the STM32G070 (`plane-fw`), on the host (its own tests) and inside
//! granite-sim's fake plane.
//!
//! - [`device`]: the small synchronous traits the firmware fills in
//!   (segment bus, core pins, backup registers, config pages, DFU slot).
//!   Nothing else in the crate touches hardware.
//! - [`drivers`]: TCA9548A, TCA6424A, INA238 and ADC121C027 over
//!   [`device::SegmentBus`], including the unit board's expander line
//!   map.
//! - [`model`]: the per-slot and per-unit model the poll cycle fills and
//!   the link reads, in the wire's own units.
//! - [`backup`] / [`config`]: the intended state in the TAMP registers,
//!   and the config in the two CONFIG pages.
//! - [`discovery`] / [`poll`]: finding the units, keeping them, and the
//!   20 Hz / 2 Hz / 1 Hz schedule.
//! - [`sequencer`]: the slot actions, their timing and the PRESS_TRIG
//!   toggling that bounds a press.
//! - [`link`] / [`update`] / [`events`] / [`log`]: the controller link,
//!   the firmware update, the event ring and the log ring.
//! - [`plane`]: [`Plane`] itself, which is all of the above as one state
//!   machine.
//!
//! The firmware glue is thin by design:
//!
//! ```ignore
//! let mut plane = Plane::new(Setup { address, serial, .. });
//! plane.boot(now_ms(), &mut devices, reset_cause);
//! loop {
//!     plane.tick(now_ms(), &mut devices);          // bus task, 20 Hz
//!     int_out.set_level(plane.event_pending());
//!     if plane.may_feed_watchdog() { iwdg.pet(); }
//!     if plane.reset_requested() { SCB::sys_reset(); }
//! }
//! // link task:
//! if let Some(answer) = plane.handle_frame(&request, &mut out) { i2c.respond(answer) }
//! ```
//!
//! The crate is `no_std`, allocates nothing, contains no `unsafe` and no
//! floating point: it builds for `thumbv6m-none-eabi`, which has neither
//! an FPU nor compare-and-swap. The `sim` feature adds the simulated
//! devices of [`sim`] and is the only thing here that may use `std`.

#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
extern crate std;

// The simulated devices keep growable buffers (a DFU image, the list of
// PC6 toggles); nothing outside the `sim` feature allocates.
#[cfg(feature = "sim")]
extern crate alloc;

pub mod backup;
pub mod config;
pub mod crc32;
pub mod device;
pub mod discovery;
pub mod drivers;
pub mod events;
pub mod link;
pub mod log;
pub mod model;
pub mod plane;
pub mod poll;
pub mod sequencer;
pub mod update;

#[cfg(feature = "sim")]
pub mod sim;

pub use backup::{BackupSet, Intended};
pub use config::PlaneConfig;
pub use device::{
    BackupRegs, ConfigStore, CorePins, DevResult, DeviceError, DeviceSet, Devices, SegmentBus,
    UpdateStore,
};
pub use model::{SlotModel, UnitModel};
pub use plane::{LedPattern, Plane, Setup, reset_cause};
pub use sequencer::Button;
