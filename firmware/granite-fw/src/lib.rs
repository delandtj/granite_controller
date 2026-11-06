//! Granite controller firmware library: one module per ADR 0001 component.
//! `main.rs` wires them. Each module is owned by one implementation task;
//! add code inside the module files, not here.
//!
//! # One link per image
//!
//! ADR 0006 (architecture component 7) gives each image exactly one
//! plane link, chosen at build time: `link-canfd` for the FD generation
//! (MCP2518FD on the W5500's SPI bus, the transport of firmware ADR
//! 0005) or `link-i2c` for rev D (today still the NAK stub). Both sit
//! behind [`granite_core::hal::PlaneLink`], so nothing above the link
//! layer knows which one was built - and that is exactly why the choice
//! must be unambiguous here rather than discovered at run time.
//!
//! The image carries its kind ([`platform::link_kind`]) and the OTA
//! upload refuses an image of the other one.

#[cfg(all(feature = "link-canfd", feature = "link-i2c"))]
compile_error!(
    "granite-fw: link-canfd and link-i2c are both set. One link per image (ADR 0006 \
     architecture component 7): build the FD image with the default features, or the rev D \
     image with --no-default-features --features link-i2c."
);

#[cfg(not(any(feature = "link-canfd", feature = "link-i2c")))]
compile_error!(
    "granite-fw: no plane link selected. One link per image (ADR 0006 architecture \
     component 7): --features link-canfd (the default) or --features link-i2c."
);

pub mod hw;
pub mod platform;
pub mod http;
pub mod mqtt;
pub mod modbus;
pub use granite_core as core;
