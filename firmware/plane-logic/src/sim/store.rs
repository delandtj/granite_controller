//! The simulated core pins, backup registers, config pages and DFU
//! slot.
//!
//! [`FakePins`] is the interesting one: it records every PC6 toggle with
//! its timestamp, which is how a test checks that PRESS_TRIG moves only
//! while a press runs, and it derives PRESS_EN the way the 74LVC1G123
//! does (high for [`PRESS_EN_MS`] after the last edge).

use crate::backup::BACKUP_WORDS;
use crate::config::BLOB_BYTES;
use crate::crc32;
use crate::device::{BackupRegs, ConfigStore, CorePins, DevResult, DeviceError, UpdateStore};

/// How long PRESS_EN stays high after an edge on PC6 (the ~1 s of the
/// retriggerable monostable, docs/press-deadline-line.md).
pub const PRESS_EN_MS: u64 = 1_000;

/// The core's pins and sensors.
#[derive(Debug, Default)]
pub struct FakePins {
    /// Every PC6 toggle, with the millisecond it happened at.
    pub press_toggles: alloc::vec::Vec<u64>,
    /// How often EXP_RSTN was pulsed.
    pub expander_resets: u32,
    /// The core's input \[10 mV\].
    pub core_volts: u16,
    /// The MCU's temperature \[0.1 C\].
    pub core_temperature: i16,
    /// The time [`CorePins::toggle_press_trigger`] should record; the
    /// harness keeps it in step with the plane's clock.
    pub now_ms: u64,
}

impl FakePins {
    /// Pins at 48 V and 25 C.
    pub fn new() -> Self {
        FakePins {
            core_volts: 4_800,
            core_temperature: 250,
            ..FakePins::default()
        }
    }

    /// True while PRESS_EN is high: an edge inside the last second.
    pub fn press_en(&self, now_ms: u64) -> bool {
        self.press_toggles
            .last()
            .is_some_and(|last| now_ms.saturating_sub(*last) <= PRESS_EN_MS)
    }

    /// Toggles inside a window, for a test.
    pub fn toggles_between(&self, from_ms: u64, to_ms: u64) -> usize {
        self.press_toggles
            .iter()
            .filter(|t| **t >= from_ms && **t <= to_ms)
            .count()
    }
}

impl CorePins for FakePins {
    fn toggle_press_trigger(&mut self) {
        self.press_toggles.push(self.now_ms);
    }

    fn pulse_expander_reset(&mut self) {
        self.expander_resets += 1;
    }

    fn core_input_10mv(&mut self) -> Option<u16> {
        Some(self.core_volts)
    }

    fn core_temperature_decidegrees(&mut self) -> Option<i16> {
        Some(self.core_temperature)
    }
}

/// The TAMP backup registers.
#[derive(Debug, Default, Clone, Copy)]
pub struct FakeBackup {
    /// The words.
    pub words: [u32; BACKUP_WORDS],
}

impl FakeBackup {
    /// Registers as a cold start leaves them.
    pub const fn new() -> Self {
        FakeBackup {
            words: [0; BACKUP_WORDS],
        }
    }

    /// Wipe them, which is what a core power loss does.
    pub fn wipe(&mut self) {
        self.words = [0; BACKUP_WORDS];
    }
}

impl BackupRegs for FakeBackup {
    fn read(&self, index: usize) -> u32 {
        self.words.get(index).copied().unwrap_or(0)
    }

    fn write(&mut self, index: usize, value: u32) {
        if let Some(word) = self.words.get_mut(index) {
            *word = value;
        }
    }
}

/// The two CONFIG pages.
#[derive(Debug, Clone, Copy)]
pub struct FakeConfigStore {
    /// The pages, as flash leaves them after an erase.
    pub pages: [[u8; BLOB_BYTES]; 2],
    /// Writes fail: a worn or protected page.
    pub read_only: bool,
}

impl Default for FakeConfigStore {
    fn default() -> Self {
        FakeConfigStore::new()
    }
}

impl FakeConfigStore {
    /// Two erased pages.
    pub const fn new() -> Self {
        FakeConfigStore {
            pages: [[0xFF; BLOB_BYTES]; 2],
            read_only: false,
        }
    }
}

impl ConfigStore for FakeConfigStore {
    fn read(&mut self, copy: u8, out: &mut [u8]) -> DevResult<()> {
        let page = self
            .pages
            .get(copy as usize)
            .ok_or(DeviceError::OutOfRange)?;
        let n = out.len().min(BLOB_BYTES);
        out[..n].copy_from_slice(&page[..n]);
        Ok(())
    }

    fn write(&mut self, copy: u8, data: &[u8]) -> DevResult<()> {
        if self.read_only {
            return Err(DeviceError::Storage);
        }
        let page = self
            .pages
            .get_mut(copy as usize)
            .ok_or(DeviceError::OutOfRange)?;
        if data.len() > BLOB_BYTES {
            return Err(DeviceError::OutOfRange);
        }
        *page = [0xFF; BLOB_BYTES];
        page[..data.len()].copy_from_slice(data);
        Ok(())
    }
}

/// Bytes the simulated DFU slot holds (the 54 KB of ADR 0002 decision
/// 2, kept small so a test image is quick to build).
pub const FAKE_DFU_BYTES: usize = 4 * 1024;

/// The DFU slot and the A/B markers.
#[derive(Debug, Clone)]
pub struct FakeUpdateStore {
    /// The slot.
    pub dfu: alloc::vec::Vec<u8>,
    /// The swap is marked: the next boot runs the DFU image.
    pub swap_marked: bool,
    /// The running image has been marked booted.
    pub booted: bool,
    /// The running image is unconfirmed.
    pub unconfirmed: bool,
    /// The bootloader rolled back.
    pub rolled_back: bool,
    /// Writes fail.
    pub broken: bool,
}

impl Default for FakeUpdateStore {
    fn default() -> Self {
        FakeUpdateStore::new()
    }
}

impl FakeUpdateStore {
    /// An erased slot and a confirmed image.
    pub fn new() -> Self {
        FakeUpdateStore {
            dfu: alloc::vec![0xFF; FAKE_DFU_BYTES],
            swap_marked: false,
            booted: true,
            unconfirmed: false,
            rolled_back: false,
            broken: false,
        }
    }

    /// A store whose running image is on probation, as the first boot
    /// after a swap has it.
    pub fn on_probation() -> Self {
        FakeUpdateStore {
            booted: false,
            unconfirmed: true,
            ..FakeUpdateStore::new()
        }
    }

    /// What the slot holds for the first `len` bytes.
    pub fn image(&self, len: u32) -> &[u8] {
        &self.dfu[..len as usize]
    }
}

impl UpdateStore for FakeUpdateStore {
    fn dfu_capacity(&self) -> u32 {
        self.dfu.len() as u32
    }

    fn erase_dfu(&mut self) -> DevResult<()> {
        if self.broken {
            return Err(DeviceError::Storage);
        }
        self.dfu.fill(0xFF);
        self.swap_marked = false;
        Ok(())
    }

    fn write_dfu(&mut self, offset: u32, data: &[u8]) -> DevResult<()> {
        if self.broken {
            return Err(DeviceError::Storage);
        }
        let at = offset as usize;
        if at + data.len() > self.dfu.len() {
            return Err(DeviceError::OutOfRange);
        }
        self.dfu[at..at + data.len()].copy_from_slice(data);
        Ok(())
    }

    fn crc32_dfu(&mut self, len: u32) -> DevResult<u32> {
        if len as usize > self.dfu.len() {
            return Err(DeviceError::OutOfRange);
        }
        Ok(crc32::crc32(&self.dfu[..len as usize]))
    }

    fn mark_swap(&mut self) -> DevResult<()> {
        if self.broken {
            return Err(DeviceError::Storage);
        }
        self.swap_marked = true;
        Ok(())
    }

    fn mark_booted(&mut self) -> DevResult<()> {
        if self.broken {
            return Err(DeviceError::Storage);
        }
        self.booted = true;
        self.unconfirmed = false;
        Ok(())
    }

    fn image_unconfirmed(&self) -> bool {
        self.unconfirmed
    }

    fn rolled_back(&self) -> bool {
        self.rolled_back
    }
}
