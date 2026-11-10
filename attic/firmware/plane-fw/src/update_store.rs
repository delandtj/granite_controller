//! [`UpdateStore`]: not implemented in this build.
//!
//! ADR 0002 decision 9 updates the plane over the link: FW_BEGIN erases
//! the DFU slot, FW_DATA fills it, FW_COMMIT marks an embassy-boot swap
//! and the bootloader does the rest. None of that exists yet: there is
//! no plane-boot, no partition split (memory.x gives this image the
//! whole 128 KB) and no swap state page, so a DFU write here would
//! overwrite the running image.
//!
//! Every operation therefore refuses with [`DeviceError::Storage`],
//! which the plane logic turns into update state 5 "failed" and a
//! result code the controller reads back: an update is *refused*, not
//! half-done. `image_unconfirmed` and `rolled_back` are false, so INFO
//! reports the image as confirmed, which an SWD-flashed image is.
//!
//! What this needs to become real: plane-boot (embassy-boot-stm32 with
//! the IWDG fed through the swap), the memory.x split of ADR 0002
//! decision 2, and embassy-boot's `FirmwareUpdater` behind these six
//! methods.

use plane_logic::device::{DevResult, DeviceError, UpdateStore};

/// The DFU slot that is not there yet.
pub struct NoUpdates;

impl UpdateStore for NoUpdates {
    fn dfu_capacity(&self) -> u32 {
        0
    }

    fn erase_dfu(&mut self) -> DevResult<()> {
        Err(DeviceError::Storage)
    }

    fn write_dfu(&mut self, _offset: u32, _data: &[u8]) -> DevResult<()> {
        Err(DeviceError::Storage)
    }

    fn crc32_dfu(&mut self, _len: u32) -> DevResult<u32> {
        Err(DeviceError::Storage)
    }

    fn mark_swap(&mut self) -> DevResult<()> {
        Err(DeviceError::Storage)
    }

    fn mark_booted(&mut self) -> DevResult<()> {
        Err(DeviceError::Storage)
    }

    fn image_unconfirmed(&self) -> bool {
        false
    }

    fn rolled_back(&self) -> bool {
        false
    }
}
