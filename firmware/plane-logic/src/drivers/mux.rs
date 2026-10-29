//! TCA9548A segment mux, address 0x70 (power/pp-48/tools/sch/blocks/
//! mux.py: A0-A2 to GND, channel k = unit k + 1).
//!
//! The part has no register pointer: one written byte *is* the control
//! register, bit n enabling channel n, and a read gives it back
//! (TCA9548A datasheet SCPS207, "Control register"). So selecting a
//! channel is a one-byte write and verifying it is a one-byte read.
//!
//! /RESET is not on the bus: it is the core's MUX_RST pin, reached
//! through [`SegmentBus::reset_mux`].

use crate::device::{DevResult, SegmentBus};

/// The mux's I2C address (A0-A2 to GND).
pub const MUX_ADDR: u8 = 0x70;

/// Channels the plane uses: one per unit board (ADR 0005).
pub const CHANNELS: u8 = granite_proto::SEGMENT_CHANNELS as u8;

/// The TCA9548A on the plane's segment bus.
#[derive(Debug, Default, Clone, Copy)]
pub struct Mux {
    /// What was last written, so an unchanged selection costs nothing.
    selected: Option<u8>,
}

impl Mux {
    /// A mux whose selection is unknown: the first [`Mux::select`]
    /// always writes.
    pub const fn new() -> Self {
        Mux { selected: None }
    }

    /// The channel currently selected, as far as this driver knows.
    pub const fn selected(&self) -> Option<u8> {
        self.selected
    }

    /// Forget the selection: after a MUX_RST pulse or a bus error, the
    /// next [`Mux::select`] must write again.
    pub fn invalidate(&mut self) {
        self.selected = None;
    }

    /// Select exactly channel `channel` (0-3), leaving the others off.
    pub fn select<B: SegmentBus>(&mut self, bus: &mut B, channel: u8) -> DevResult<()> {
        if self.selected == Some(channel) {
            return Ok(());
        }
        // One channel at a time: the units all answer the same
        // addresses, so two channels on would collide.
        let r = bus.write(MUX_ADDR, &[1u8 << channel]);
        self.selected = if r.is_ok() { Some(channel) } else { None };
        r
    }

    /// Turn every channel off.
    pub fn deselect<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<()> {
        let r = bus.write(MUX_ADDR, &[0x00]);
        self.selected = None;
        r
    }

    /// Pulse MUX_RST and forget the selection.
    pub fn reset<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<()> {
        self.selected = None;
        bus.reset_mux()
    }

    /// Read the control register back (the enabled-channel bitmap).
    pub fn read_control<B: SegmentBus>(&mut self, bus: &mut B) -> DevResult<u8> {
        let mut byte = [0u8; 1];
        bus.read(MUX_ADDR, &[], &mut byte)?;
        Ok(byte[0])
    }
}
