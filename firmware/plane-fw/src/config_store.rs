//! [`ConfigStore`] over two flash pages (ADR 0002 decision 2 and 10).
//!
//! The plane runs on its config without a controller, so it has to
//! survive a power loss: two 2 KB pages, each holding a blob with a
//! sequence number and a CRC-32, written alternately by
//! `plane_logic::config`. This module only erases and writes a page.
//!
//! Where the pages are is temporary. The slot plan of ADR 0002 puts
//! CONFIG at 0x08004800, right after the bootloader's swap state, but
//! plane-boot does not exist yet and this image owns the whole flash
//! (memory-*.x), so the two copies live at 0x0801F000 / 0x0801F800
//! instead - out of reach of an image that is nowhere near 124 KB. They
//! move when the bootloader lands, and the first boot after the move
//! reads blank pages and falls back to the defaults, which the
//! controller then re-writes (it compares the config hash in INFO).
//!
//! The same two pages on both parts, on purpose. On the G070CB they are
//! the last two of its 128 KB. On the G0B1CC (256 KB, `link-fdcan`)
//! they are the last two of the **first** 128 KB, which is bank 1
//! whichever way that part's DUAL_BANK option bit is set: a page erase
//! there names the same physical page in single-bank numbering (page 62
//! of 128) and in dual-bank numbering (page 62 of bank 1, BKER = 0), so
//! the store cannot land on the wrong page because a part arrived
//! configured the other way. The last two pages of the G0B1's 256 KB
//! would not have that property.

use embassy_stm32::flash::{Blocking, Flash};
use plane_logic::config::{BLOB_BYTES, COPIES};
use plane_logic::device::{DevResult, DeviceError, ConfigStore};

/// Flash page size on both parts: 2 KB.
const PAGE: u32 = 2048;

/// Write granularity on both parts: one 64-bit double word.
const WRITE_ALIGN: usize = 8;

/// Offset of copy 0 from the start of flash: the last two pages of the
/// first 128 KB (see the module docs for why not "of the part").
const FIRST_COPY: u32 = 128 * 1024 - 2 * PAGE;

/// The two CONFIG pages.
pub struct Config {
    flash: Flash<'static, Blocking>,
}

impl Config {
    /// Take the flash controller.
    pub fn new(flash: embassy_stm32::Peri<'static, embassy_stm32::peripherals::FLASH>) -> Self {
        Config {
            flash: Flash::new_blocking(flash),
        }
    }

    /// Where a copy starts.
    fn offset(copy: u8) -> Option<u32> {
        (copy < COPIES).then(|| FIRST_COPY + u32::from(copy) * PAGE)
    }
}

impl ConfigStore for Config {
    fn read(&mut self, copy: u8, out: &mut [u8]) -> DevResult<()> {
        let offset = Self::offset(copy).ok_or(DeviceError::OutOfRange)?;
        if out.len() > PAGE as usize {
            return Err(DeviceError::OutOfRange);
        }
        self.flash
            .blocking_read(offset, out)
            .map_err(|_| DeviceError::Storage)
    }

    fn write(&mut self, copy: u8, data: &[u8]) -> DevResult<()> {
        let offset = Self::offset(copy).ok_or(DeviceError::OutOfRange)?;
        if data.len() > BLOB_BYTES {
            return Err(DeviceError::OutOfRange);
        }
        // The blob is as long as its records; the flash writes whole
        // double words, so it goes out in a padded buffer. 0xFF is what
        // an erased page reads as, so the padding is indistinguishable
        // from "nothing was written here".
        let mut padded = [0xFFu8; BLOB_BYTES];
        padded[..data.len()].copy_from_slice(data);
        let len = data.len().next_multiple_of(WRITE_ALIGN);

        self.flash
            .blocking_erase(offset, offset + PAGE)
            .map_err(|_| DeviceError::Storage)?;
        self.flash
            .blocking_write(offset, &padded[..len])
            .map_err(|_| DeviceError::Storage)
    }
}
