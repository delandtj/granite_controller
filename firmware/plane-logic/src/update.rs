//! The firmware update over the link (ADR 0002 decision 9, ADR 0003 FW
//! records 0x90-0x94).
//!
//! FW begin erases the DFU slot, FW data records fill it, FW commit
//! checks the CRC-32 over what was written and marks the swap, a CTRL
//! reboot performs it, and FW confirm ends the new image's probation.
//! STATUS 0x18 carries the state and the offset the plane expects next,
//! which is how the controller paces itself without a long clock
//! stretch.
//!
//! The probation ladder is the controller's own (firmware ADR 0001): the
//! new image runs unconfirmed, and without a confirm inside
//! [`crate::plane::PROBATION_MS`] the plane asks for a reset and lets the
//! bootloader swap back.

use granite_proto::{FwBegin, FwData, ImageState, UpdateState, Version};

use crate::device::UpdateStore;

/// The firmware update state machine.
#[derive(Debug, Clone, Copy)]
pub struct Update {
    /// What the plane is doing with the image (STATUS 0x18).
    state: UpdateState,
    /// The offset the plane expects next.
    next_offset: u32,
    /// The size FW begin promised.
    size: u32,
    /// The CRC-32 FW begin promised.
    crc32: u32,
    /// The version FW begin carried.
    version: Version,
    /// When the running image's probation runs out.
    probation_until_ms: Option<u64>,
    /// The running image is unconfirmed.
    unconfirmed: bool,
    /// The bootloader rolled back to the previous image.
    rolled_back: bool,
}

impl Default for Update {
    fn default() -> Self {
        Update::new()
    }
}

impl Update {
    /// An idle update with no image in flight.
    pub const fn new() -> Self {
        Update {
            state: UpdateState::Idle,
            next_offset: 0,
            size: 0,
            crc32: 0,
            version: Version::new(0, 0, 0),
            probation_until_ms: None,
            unconfirmed: false,
            rolled_back: false,
        }
    }

    /// What STATUS 0x18 reports.
    pub const fn progress(&self) -> granite_proto::UpdateProgress {
        granite_proto::UpdateProgress::new(self.state, self.next_offset)
    }

    /// The state alone.
    pub const fn state(&self) -> UpdateState {
        self.state
    }

    /// The version the image in flight claims.
    pub const fn version(&self) -> Version {
        self.version
    }

    /// True while a transfer is going on (STATUS 0x13, and the fast LED
    /// blink).
    pub const fn in_progress(&self) -> bool {
        !matches!(self.state, UpdateState::Idle | UpdateState::Failed)
    }

    /// True while the running image is unconfirmed.
    pub const fn unconfirmed(&self) -> bool {
        self.unconfirmed
    }

    /// True when the bootloader rolled back.
    pub const fn rolled_back(&self) -> bool {
        self.rolled_back
    }

    /// The image state of INFO 0x0A.
    pub const fn image_state(&self) -> ImageState {
        if self.rolled_back {
            ImageState::RolledBack
        } else if self.unconfirmed {
            ImageState::Probation
        } else {
            ImageState::Confirmed
        }
    }

    /// Read the store at boot and start the probation clock.
    pub fn boot<U: UpdateStore>(&mut self, now_ms: u64, store: &U) {
        self.unconfirmed = store.image_unconfirmed();
        self.rolled_back = store.rolled_back();
        self.probation_until_ms = if self.unconfirmed {
            Some(now_ms + crate::plane::PROBATION_MS)
        } else {
            None
        };
    }

    /// FW 0x90 begin: erase the DFU slot.
    pub fn begin<U: UpdateStore>(&mut self, store: &mut U, begin: FwBegin) -> bool {
        if begin.size == 0 || begin.size > store.dfu_capacity() {
            self.state = UpdateState::Failed;
            return false;
        }
        self.state = UpdateState::Erasing;
        if store.erase_dfu().is_err() {
            self.state = UpdateState::Failed;
            return false;
        }
        self.size = begin.size;
        self.crc32 = begin.crc32;
        self.version = begin.version;
        self.next_offset = 0;
        self.state = UpdateState::Receiving;
        true
    }

    /// FW 0x91 data.
    ///
    /// A record the plane already has (an offset below what it expects)
    /// is a retransmission and is accepted without writing again; one
    /// past what it expects would leave a hole and is refused, leaving
    /// the offset in STATUS for the controller to go back to.
    pub fn data<U: UpdateStore>(&mut self, store: &mut U, data: FwData<'_>) -> bool {
        if !matches!(self.state, UpdateState::Receiving) {
            return false;
        }
        if data.offset < self.next_offset {
            return true;
        }
        if data.offset > self.next_offset || data.next_offset() > self.size {
            return false;
        }
        if store.write_dfu(data.offset, data.data).is_err() {
            self.state = UpdateState::Failed;
            return false;
        }
        self.next_offset = data.next_offset();
        true
    }

    /// FW 0x92 commit: check the CRC over the DFU slot and mark the
    /// swap.
    pub fn commit<U: UpdateStore>(&mut self, store: &mut U, crc32: u32) -> bool {
        if !matches!(self.state, UpdateState::Receiving) || self.next_offset != self.size {
            self.state = UpdateState::Failed;
            return false;
        }
        self.state = UpdateState::Verifying;
        let Ok(found) = store.crc32_dfu(self.size) else {
            self.state = UpdateState::Failed;
            return false;
        };
        if found != crc32 || found != self.crc32 {
            self.state = UpdateState::Failed;
            return false;
        }
        if store.mark_swap().is_err() {
            self.state = UpdateState::Failed;
            return false;
        }
        self.state = UpdateState::Ready;
        true
    }

    /// FW 0x93 confirm: the running image is good.
    pub fn confirm<U: UpdateStore>(&mut self, store: &mut U) -> bool {
        if store.mark_booted().is_err() {
            return false;
        }
        self.unconfirmed = false;
        self.rolled_back = false;
        self.probation_until_ms = None;
        true
    }

    /// FW 0x94 abort: forget the transfer. The DFU slot keeps whatever
    /// was written; the next begin erases it.
    pub fn abort(&mut self) {
        self.state = UpdateState::Idle;
        self.next_offset = 0;
        self.size = 0;
    }

    /// Housekeeping: a probation that ran out asks for a reset.
    pub fn tick<U: UpdateStore>(&mut self, now_ms: u64, store: &U, reset_request: &mut bool) {
        let _ = store;
        if let Some(until) = self.probation_until_ms
            && now_ms >= until
        {
            // No confirm inside the window: reset, and the bootloader
            // swaps back to the image that worked.
            self.probation_until_ms = None;
            *reset_request = true;
        }
    }

    /// When the probation runs out, for a test or a log line.
    pub const fn probation_until_ms(&self) -> Option<u64> {
        self.probation_until_ms
    }
}
