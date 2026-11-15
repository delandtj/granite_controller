//! Which switches this image knows, and what each of them is (ADR 0006
//! component 24).
//!
//! One static row per (model, firmware) pair whose pages were captured
//! and whose parsers were tested against them. The row carries the port
//! layout, the dialect its pages speak and whether writes are allowed,
//! so nothing outside this file knows that a block switch has nine
//! ports or that port 9 is the SFP+.
//!
//! Lookup is exact on both strings, trimmed. There is no prefix, range
//! or "newer than" rule: the pages are scraped, so a version nobody
//! captured is a version nobody tested. The model is part of the key
//! because a sibling model can report the same firmware string with
//! another port count.
//!
//! The table lives in the image and not in the configuration: "tested
//! against these pages" is a property of the build, so enabling another
//! unit is a controller OTA and never a config override.

use alloc::string::String;

/// The ports a unit has, and which of them are SFP+.
///
/// Ports are 1-based and contiguous, which is how every page of the
/// web UI numbers them (`Port 1` .. `Port N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// How many ports the unit has; the ports are `1..=ports`.
    pub ports: u8,
    /// The SFP+ ports, 1-based. They are the only ones that do 10G and
    /// the only ones that cannot do 10M (`docs/sodola.md`, "Quirks").
    pub sfp: &'static [u8],
}

impl Layout {
    /// True when `port` is a port of this unit.
    pub const fn is_port(&self, port: u8) -> bool {
        port >= 1 && port <= self.ports
    }

    /// True when `port` is an SFP+ cage.
    pub fn is_sfp(&self, port: u8) -> bool {
        self.sfp.contains(&port)
    }

    /// The widest layout in the table: what a port number is checked
    /// against at configuration time, when there is no switch to ask,
    /// and what an unprofiled switch is read with.
    pub fn widest() -> &'static Layout {
        let mut best = &PROFILES[0].layout;
        for profile in PROFILES {
            if profile.layout.ports > best.ports {
                best = &profile.layout;
            }
        }
        best
    }
}

/// Which parser and form set speaks to a unit.
///
/// An enum and not a trait object: there is one variant today and
/// [`super::parse`] and [`super::forms`] *are* that variant. A firmware
/// that changes some pages gets a variant that overrides those page
/// functions and falls through for the rest; that dispatch is added
/// with the second variant, not before (ADR 0006 component 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// The Sodola V200 CGI pages, as `docs/sodola.md` describes them.
    SodolaV200,
}

/// How far a row has been taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// The pages parse and one write-and-revert was done on a live
    /// unit: writes are allowed.
    Verified,
    /// The pages parse and nothing was ever written: monitored only.
    ReadOnly,
}

impl Tier {
    /// True when a row at this tier may be written to.
    pub const fn may_write(self) -> bool {
        matches!(self, Tier::Verified)
    }

    /// Name used in logs and events.
    pub const fn as_str(self) -> &'static str {
        match self {
            Tier::Verified => "verified",
            Tier::ReadOnly => "read_only",
        }
    }
}

/// One switch this image knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// `Device Model` of `info.cgi`, exact.
    pub model: &'static str,
    /// `Firmware Version` of `info.cgi`, exact.
    pub firmware: &'static str,
    /// Port count and which ports are SFP+.
    pub layout: Layout,
    /// Which parser and form set speaks to it.
    pub dialect: Dialect,
    /// Whether writes are allowed.
    pub tier: Tier,
}

impl Profile {
    /// The fixture directory of this row under
    /// `granite-core/tests/fixtures/`: `<model>-<firmware>`, lower
    /// case.
    pub fn fixture_dir(&self) -> String {
        let mut out = String::with_capacity(self.model.len() + self.firmware.len() + 1);
        out.push_str(&self.model.to_lowercase());
        out.push('-');
        out.push_str(&self.firmware.to_lowercase());
        out
    }

    /// True when writes to this unit are allowed.
    pub const fn may_write(&self) -> bool {
        self.tier.may_write()
    }
}

/// The ports of an SL902-SWTGW218AS: 8x 2.5G copper plus the SFP+.
const SL902_LAYOUT: Layout = Layout {
    ports: 9,
    sfp: &[9],
};

/// Every switch this image knows.
///
/// Adding one: capture its pages with `tools/sodola.py get` into
/// `tests/fixtures/<model>-<firmware>` (lower case), add a
/// [`Tier::ReadOnly`] row, fix or fork the page functions until
/// `tests/switch.rs` passes, do one write-and-revert on a live unit,
/// then promote the row to [`Tier::Verified`].
pub static PROFILES: &[Profile] = &[Profile {
    model: "SL902-SWTGW218AS",
    firmware: "V200.1.8",
    layout: SL902_LAYOUT,
    dialect: Dialect::SodolaV200,
    tier: Tier::Verified,
}];

/// The row for a (model, firmware) pair, or `None` when this image does
/// not know that unit.
///
/// Both strings are trimmed and compared exactly: the model as
/// `info.cgi` prints it, the firmware as `info.cgi` prints it.
pub fn lookup(model: &str, firmware: &str) -> Option<&'static Profile> {
    let model = model.trim();
    let firmware = firmware.trim();
    PROFILES
        .iter()
        .find(|p| p.model == model && p.firmware == firmware)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_found_on_both_strings_and_nothing_else_is() {
        let found = lookup("SL902-SWTGW218AS", "V200.1.8").expect("the bench unit");
        assert_eq!(found.layout.ports, 9);
        assert!(found.layout.is_sfp(9) && !found.layout.is_sfp(1));
        assert!(found.may_write(), "the bench unit is verified");
        assert_eq!(found.dialect, Dialect::SodolaV200);

        assert!(
            lookup("SL902-SWTGW218AS", "V200.1.9").is_none(),
            "the right model on a firmware nobody captured"
        );
        assert!(
            lookup("SL905-SWTGW218AS", "V200.1.8").is_none(),
            "a sibling model that reports the same firmware string"
        );
        assert!(lookup("", "").is_none());
    }

    #[test]
    fn the_strings_are_trimmed_but_not_otherwise_touched() {
        assert_eq!(
            lookup("  SL902-SWTGW218AS \n", "\tV200.1.8  "),
            lookup("SL902-SWTGW218AS", "V200.1.8"),
            "the pages come with whitespace around the cell text"
        );
        assert!(
            lookup("sl902-swtgw218as", "V200.1.8").is_none(),
            "the model is matched exactly, not case-folded"
        );
    }

    #[test]
    fn the_widest_layout_covers_every_row() {
        let widest = Layout::widest();
        for profile in PROFILES {
            assert!(
                profile.layout.ports <= widest.ports,
                "{} has more ports than the widest layout",
                profile.model
            );
        }
    }

    #[test]
    fn a_fixture_directory_is_the_row_in_lower_case() {
        assert_eq!(PROFILES[0].fixture_dir(), "sl902-swtgw218as-v200.1.8");
    }
}
