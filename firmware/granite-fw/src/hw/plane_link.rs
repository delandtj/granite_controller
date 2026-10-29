//! The plane link - **a stub until the rev D board exists**.
//!
//! Controller ADR 0004 decision 9 puts the plane link on the HP I2C
//! peripheral at 100 kHz with a 5-pin JST PH to the planes' J2 and
//! EXP_INT on GPIO0. That board is not built, and a driver written
//! against hardware nobody has measured is a driver nobody can trust:
//! the clock-stretch behaviour through the ISO1640 is one of the two
//! risks ADR 0004 lists, and it is a bench question.
//!
//! So this is the honest placeholder: every address answers
//! [`LinkError::Nak`], which is exactly how the plane client reads "no
//! plane is strapped to that address" (ADR 0004 decision 5). The
//! firmware therefore comes up with no planes, no nodes, and commands
//! that are refused as absent - rather than with a model built on reads
//! that never happened. The real driver replaces this file; nothing
//! above it changes, because the client only ever sees
//! [`granite_core::hal::PlaneLink`].

use esp_idf_hal::gpio::{Gpio0, Input, PinDriver, Pull};
use granite_core::hal::{LinkError, PlaneLink};

/// The plane link that is not there yet.
pub struct StubPlaneLink {
    /// EXP_INT, held as an input so nothing else claims the pin and the
    /// line can be read once there is something on the other end.
    int: Option<PinDriver<'static, Input>>,
    /// How many transactions were refused, for the log.
    naks: u32,
}

impl StubPlaneLink {
    /// Claim EXP_INT and refuse everything else.
    pub fn new(int: Gpio0<'static>) -> Self {
        let int = match PinDriver::input(int, Pull::Up) {
            Ok(pin) => Some(pin),
            Err(e) => {
                log::warn!("plane link: EXP_INT could not be claimed: {e}");
                None
            }
        };
        log::warn!(
            "plane link: stub driver - no plane transaction is performed, every address \
             answers NAK until the rev D board and its HP I2C driver exist"
        );
        StubPlaneLink { int, naks: 0 }
    }

    /// How many transactions have been refused.
    pub const fn naks(&self) -> u32 {
        self.naks
    }
}

impl PlaneLink for StubPlaneLink {
    fn write_read(&mut self, _addr: u8, _req: &[u8], _resp: &mut [u8]) -> Result<usize, LinkError> {
        self.naks = self.naks.saturating_add(1);
        Err(LinkError::Nak)
    }

    fn write(&mut self, _addr: u8, _frame: &[u8]) -> Result<(), LinkError> {
        self.naks = self.naks.saturating_add(1);
        Err(LinkError::Nak)
    }

    fn int_asserted(&mut self) -> bool {
        // Open drain with a pull-up: low means someone has events. With
        // nothing on the line it reads high, which is "no events".
        self.int.as_ref().is_some_and(|pin| pin.is_low())
    }
}
