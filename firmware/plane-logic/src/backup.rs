//! The intended state and its home in the TAMP backup registers
//! (ADR 0002 decision 3).
//!
//! The intended state is everything a reset must not change: which slots
//! the operator set OFF, which modules are held off (RUNOFF), which
//! slots are disarmed, and which units are commanded off (U_OFF). It is
//! three bits per slot and one per unit, so the whole plane fits in one
//! `u32` and the register set is five words:
//!
//! | Word | Content |
//! |---|---|
//! | 0 | magic [`MAGIC`] |
//! | 1 | the intended bits |
//! | 2 | boot count |
//! | 3 | flags ([`FLAG_COMMANDED_REBOOT`]) |
//! | 4 | CRC-32 over words 0-3 |
//!
//! Valid (magic and CRC agree) means a reset that kept VDD: the
//! expanders held their outputs through it, so an OFF slot never came
//! on and the plane adopts the set. Invalid means the core lost power:
//! hardware defaults, every slot ON, and the "intended state lost" flag
//! in STATUS for the controller to re-apply what it wanted.
//!
//! The set is rewritten on **every** intended-state change, not only
//! before a commanded reset, so a crash in the middle of a command
//! loses at most the command that was running.

use crate::crc32;
use crate::device::BackupRegs;
use granite_proto::{SLOT_COUNT, UNIT_COUNT};

/// Words of the backup set.
pub const BACKUP_WORDS: usize = 5;

/// Word 0: "PLN1", so a set from another firmware is not adopted.
pub const MAGIC: u32 = 0x504C_4E31;

/// Word 3 bit 0: the reset that is coming was asked for (CTRL 0x4C, or
/// the reset that performs a firmware swap), not a crash.
pub const FLAG_COMMANDED_REBOOT: u32 = 1 << 0;

/// Bit of slot `slot`'s OFF in the intended word.
const fn bit_off(slot: usize) -> u32 {
    1 << slot
}

/// Bit of slot `slot`'s RUNOFF.
const fn bit_runoff(slot: usize) -> u32 {
    1 << (SLOT_COUNT + slot)
}

/// Bit of slot `slot`'s DISARM.
const fn bit_disarm(slot: usize) -> u32 {
    1 << (2 * SLOT_COUNT + slot)
}

/// Bit of unit `unit`'s U_OFF.
const fn bit_u_off(unit: usize) -> u32 {
    1 << (3 * SLOT_COUNT + unit)
}

/// What the plane means to be doing, independent of what the hardware
/// currently shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Intended {
    /// Packed bits; see the module docs.
    bits: u32,
}

impl Intended {
    /// The hardware defaults: every slot ON, every module running, no
    /// slot disarmed, every unit on.
    pub const fn hardware_defaults() -> Self {
        Intended { bits: 0 }
    }

    /// The packed word, as it goes into the backup register.
    pub const fn bits(self) -> u32 {
        self.bits
    }

    /// An intended state from a packed word.
    pub const fn from_bits(bits: u32) -> Self {
        Intended { bits }
    }

    /// True while slot `slot` is meant to be OFF.
    pub const fn off(self, slot: u8) -> bool {
        self.bits & bit_off(slot as usize) != 0
    }

    /// Set or clear slot `slot`'s OFF.
    pub fn set_off(&mut self, slot: u8, off: bool) {
        self.set(bit_off(slot as usize), off);
    }

    /// True while slot `slot`'s module is meant to be held off.
    pub const fn runoff(self, slot: u8) -> bool {
        self.bits & bit_runoff(slot as usize) != 0
    }

    /// Set or clear slot `slot`'s RUNOFF.
    pub fn set_runoff(&mut self, slot: u8, on: bool) {
        self.set(bit_runoff(slot as usize), on);
    }

    /// True while slot `slot` is meant to be disarmed.
    pub const fn disarm(self, slot: u8) -> bool {
        self.bits & bit_disarm(slot as usize) != 0
    }

    /// Set or clear slot `slot`'s DISARM.
    pub fn set_disarm(&mut self, slot: u8, on: bool) {
        self.set(bit_disarm(slot as usize), on);
    }

    /// True while unit `unit` is meant to be switched off.
    pub const fn u_off(self, unit: u8) -> bool {
        self.bits & bit_u_off(unit as usize) != 0
    }

    /// Set or clear unit `unit`'s U_OFF.
    pub fn set_u_off(&mut self, unit: u8, on: bool) {
        self.set(bit_u_off(unit as usize), on);
    }

    /// True when no slot and no unit is held in a non-default state.
    pub const fn is_default(self) -> bool {
        self.bits == 0
    }

    fn set(&mut self, bit: u32, on: bool) {
        if on {
            self.bits |= bit;
        } else {
            self.bits &= !bit;
        }
    }
}

/// The whole backup set as the plane holds it in RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackupSet {
    /// The intended state.
    pub intended: Intended,
    /// How often this plane has booted.
    pub boot_count: u32,
    /// [`FLAG_COMMANDED_REBOOT`] and room for more.
    pub flags: u32,
}

impl BackupSet {
    /// CRC-32 over words 0-3 of the set.
    fn checksum(&self) -> u32 {
        let mut words = [0u8; 16];
        words[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        words[4..8].copy_from_slice(&self.intended.bits().to_le_bytes());
        words[8..12].copy_from_slice(&self.boot_count.to_le_bytes());
        words[12..16].copy_from_slice(&self.flags.to_le_bytes());
        crc32::crc32(&words)
    }

    /// Read the set, or `None` when the magic or the CRC does not agree
    /// (a core power loss).
    pub fn load<K: BackupRegs>(regs: &K) -> Option<Self> {
        if regs.read(0) != MAGIC {
            return None;
        }
        let set = BackupSet {
            intended: Intended::from_bits(regs.read(1)),
            boot_count: regs.read(2),
            flags: regs.read(3),
        };
        if set.checksum() == regs.read(4) {
            Some(set)
        } else {
            None
        }
    }

    /// Write the set with its magic and CRC.
    pub fn store<K: BackupRegs>(&self, regs: &mut K) {
        regs.write(0, MAGIC);
        regs.write(1, self.intended.bits());
        regs.write(2, self.boot_count);
        regs.write(3, self.flags);
        regs.write(4, self.checksum());
    }

    /// True while the last reset was asked for.
    pub const fn commanded_reboot(&self) -> bool {
        self.flags & FLAG_COMMANDED_REBOOT != 0
    }

    /// Mark the next reset as commanded, or clear the mark.
    pub fn set_commanded_reboot(&mut self, on: bool) {
        if on {
            self.flags |= FLAG_COMMANDED_REBOOT;
        } else {
            self.flags &= !FLAG_COMMANDED_REBOOT;
        }
    }
}

/// Every bit the packing uses must fit one word.
const _: () = assert!(3 * SLOT_COUNT + UNIT_COUNT <= 32);

#[cfg(test)]
mod tests {
    use super::*;

    /// Backup registers as RAM, enough to test the packing.
    #[derive(Default)]
    struct Regs([u32; BACKUP_WORDS]);

    impl BackupRegs for Regs {
        fn read(&self, index: usize) -> u32 {
            self.0[index]
        }

        fn write(&mut self, index: usize, value: u32) {
            self.0[index] = value;
        }
    }

    #[test]
    fn every_field_has_its_own_bit() {
        let mut i = Intended::hardware_defaults();
        assert!(i.is_default());
        i.set_off(3, true);
        i.set_runoff(3, true);
        i.set_disarm(7, true);
        i.set_u_off(2, true);
        assert!(i.off(3) && i.runoff(3) && !i.disarm(3));
        assert!(i.disarm(7) && !i.off(7));
        assert!(i.u_off(2) && !i.u_off(1));
        i.set_off(3, false);
        assert!(!i.off(3) && i.runoff(3));
        assert!(!i.is_default());
    }

    #[test]
    fn a_stored_set_comes_back() {
        let mut regs = Regs::default();
        let mut set = BackupSet {
            intended: Intended::hardware_defaults(),
            boot_count: 41,
            flags: 0,
        };
        set.intended.set_off(5, true);
        set.set_commanded_reboot(true);
        set.store(&mut regs);
        assert_eq!(BackupSet::load(&regs), Some(set));
        assert!(BackupSet::load(&regs).unwrap().commanded_reboot());
    }

    #[test]
    fn a_wiped_or_corrupt_set_is_refused() {
        let regs = Regs::default();
        assert_eq!(BackupSet::load(&regs), None); // no magic: power loss
        let mut regs = Regs::default();
        BackupSet::default().store(&mut regs);
        regs.0[1] ^= 0x10; // a bit flip past the CRC
        assert_eq!(BackupSet::load(&regs), None);
    }
}
