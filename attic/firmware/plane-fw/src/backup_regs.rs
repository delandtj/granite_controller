//! [`BackupRegs`] over the TAMP backup registers (ADR 0002 decision 3).
//!
//! The G0's five `TAMP_BKPxR` words survive every reset that keeps VDD
//! (software, watchdog, NRST) and are lost with VDD, which is exactly
//! the split the ADR wants: a commanded reboot or a watchdog bite comes
//! back with the intended state and adopts it, a core power loss comes
//! back at hardware defaults with "intended state lost" set.
//!
//! embassy-stm32 does not reach them (its RTC v3 backup-register
//! accessors are stubs, because on this family the words live in the
//! TAMP peripheral and not in RTC), so this goes through the PAC. Two
//! things have to be on first: the RTC/TAMP APB clock, and the backup
//! domain write protection has to be off.

use embassy_stm32::pac;
use embassy_stm32::pac::tamp::regs::Bkpr;
use plane_logic::backup::BACKUP_WORDS;
use plane_logic::device::BackupRegs;

/// The TAMP backup registers.
pub struct Backup;

impl Backup {
    /// Enable the APB clock to RTC / TAMP and unlock the backup domain
    /// for writes. Called once, before the plane boots.
    pub fn init() -> Self {
        pac::RCC.apbenr1().modify(|w| w.set_rtcapben(true));
        pac::PWR.cr1().modify(|w| w.set_dbp(true));
        Backup
    }
}

impl BackupRegs for Backup {
    fn read(&self, index: usize) -> u32 {
        if index >= BACKUP_WORDS {
            return 0;
        }
        pac::TAMP.bkpr(index).read().0
    }

    fn write(&mut self, index: usize, value: u32) {
        if index >= BACKUP_WORDS {
            return;
        }
        pac::TAMP.bkpr(index).write_value(Bkpr(value));
    }
}
