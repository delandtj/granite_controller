//! Granite controller firmware library: one module per ADR 0001 component.
//! `main.rs` wires them. Each module is owned by one implementation task;
//! add code inside the module files, not here.

pub mod hw;
pub mod platform;
pub mod http;
pub mod mqtt;
pub mod modbus;
pub use granite_core as core;
