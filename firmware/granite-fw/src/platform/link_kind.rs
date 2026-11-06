//! The link kind of *this* image: the marker in its read-only data and
//! the one constant that says which link was built (ADR 0006,
//! architecture component 7).
//!
//! The format, the search and the verdict are
//! [`granite_core::link_kind`], host-tested there. This module is the
//! two ends the firmware owns:
//!
//! - [`LINK_TAG`], the 24 bytes linked into the image so an upload can
//!   find them in the stream it is writing;
//! - [`RUNNING`], the kind the cargo features chose, which
//!   [`crate::platform::ota::OtaWriter`] compares an uploaded image
//!   against.

use granite_core::link_kind::{LinkKind, TAG_LEN, read_tag, tag_bytes};

/// The link this image is built for.
#[cfg(feature = "link-canfd")]
pub const RUNNING: LinkKind = LinkKind::CanFd;
/// The link this image is built for.
#[cfg(all(feature = "link-i2c", not(feature = "link-canfd")))]
pub const RUNNING: LinkKind = LinkKind::I2c;

/// The marker of this image.
///
/// `#[used]` keeps it through `--gc-sections` and LTO; the section is a
/// `.rodata` sub-section, which the ESP-IDF linker script places with
/// the rest of the read-only data (`*(.rodata .rodata.*)`), so no
/// linker-script change is needed and the bytes land in the image's
/// DROM segment - inside the file an OTA upload streams.
#[used]
#[unsafe(link_section = ".rodata.granite_link_tag")]
pub static LINK_TAG: [u8; TAG_LEN] = tag_bytes(RUNNING);

/// One line for the boot log: which link this image drives, and whether
/// its own marker survived the build.
///
/// A malformed marker is a build problem (a linker that dropped or
/// rewrote the section), and it matters before anybody tries an upload:
/// an image whose marker is gone cannot be installed over this one.
pub fn summary() -> String {
    match read_tag(&LINK_TAG) {
        Some(kind) if kind == RUNNING => format!("{kind} (marker ok)"),
        Some(kind) => format!("{RUNNING} link, but the marker says {kind} - the build is wrong"),
        None => format!("{RUNNING} (marker MALFORMED: an upload of this image would be refused)"),
    }
}
