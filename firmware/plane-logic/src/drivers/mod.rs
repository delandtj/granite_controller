//! The four parts on the plane's segment bus, each written against
//! [`crate::device::SegmentBus`] (ADR 0002, "Drivers in plane-logic:
//! TCA9548A, TCA6424A, INA238, ADC121C027").
//!
//! Every driver is a plain struct with no interior state beyond the
//! shadow registers it needs, takes the bus as an argument, and returns
//! [`crate::device::DeviceError`]. None of them knows about the plane
//! model, the poll schedule or the link: the mapping from a register to
//! a wire field is in [`crate::model`].

pub mod expander;
pub mod idadc;
pub mod ina238;
pub mod mux;

pub use expander::{EXP_ADDR, Expander, In, Out};
pub use idadc::{ADC_SLOT_A, ADC_SLOT_B, IdAdc};
pub use ina238::{INA_SLOT_A, INA_SLOT_B, INA_UNIT, Ina238, Sample};
pub use mux::{MUX_ADDR, Mux};
