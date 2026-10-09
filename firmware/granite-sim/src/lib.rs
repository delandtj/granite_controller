//! Host simulator for the Granite controller (ADR 0001, "Crates").
//!
//! The simulator runs the real [`granite_core`] - actuator, node state
//! machine, rule engine, dispatcher and the HTTP route table - against
//! fake hardware driven by a scenario file, and serves the real setup
//! page over HTTPS. The point is that the browser experience and the API
//! contract can be exercised, and regression-tested, without a board.
//!
//! - [`scenario`]: the TOML file that scripts the fake hardware.
//! - [`fakes`]: the fake U14/U15, probes, dry contacts, VIN and board
//!   temperature, including how a motherboard answers a button press.
//! - [`sim`]: the simulated controller: state, tick loop, side effects.
//! - [`platform`]: the [`granite_core::api`] traits the board's platform
//!   layer implements, implemented here against the simulation.
//! - [`http`]: axum in front of [`granite_core::api::handle`].
//! - [`keys`]: the fleet recovery key tools (`keygen`, `recover`).

pub mod assets;
pub mod fakes;
pub mod http;
pub mod keys;
pub mod platform;
pub mod scenario;
pub mod sim;

pub use scenario::Scenario;
pub use sim::Sim;
