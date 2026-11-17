//! The coredump report (ADR 0001 component 27).
//!
//! A panic or a watchdog writes a coredump into the `coredump`
//! partition (`CONFIG_ESP_COREDUMP_ENABLE_TO_FLASH`, ELF format, 64 KB
//! at 0x3B000 in `partitions.csv`). The image that boots afterwards
//! reads the dump's summary and, on its first broker session, publishes
//! one `sys` / `coredump` event with the task that crashed, its program
//! counter and return address, the exception cause and the first 16 hex
//! digits of the SHA-256 of the image that crashed. The dump itself
//! stays in flash: `GET /api/v1/coredump` hands out the raw partition,
//! so a submerged controller needs no USB to be diagnosed.
//!
//! "On the first broker session" works the way the rollback report in
//! [`super::ota`] does: the summary is read at boot and parked, the
//! MQTT worker's connect hook calls [`report`], and the event is queued
//! while a session is up so it actually goes out.
//!
//! Reported once per crash: the summary is only read when this boot's
//! reset reason is a panic or a watchdog, so an operator who reboots
//! the board afterwards does not get the same report again. The dump is
//! still there to download until the next crash overwrites it.

use std::sync::Mutex;
use std::sync::mpsc::Sender;

use esp_idf_svc::sys::{
    ESP_OK, esp_core_dump_get_summary, esp_core_dump_image_check, esp_core_dump_image_get,
    esp_core_dump_summary_t, esp_partition_find_first, esp_partition_read,
    esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_COREDUMP, esp_partition_t,
    esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
};
use granite_core::hal::BootReason;
use granite_core::msg::sys_event;

/// Hex digits of the crashed image's ELF SHA-256 the event carries.
const SHA_DIGITS: usize = 16;

/// Where the event goes, parked by [`init`] because the MQTT worker's
/// connect hook has no bus of its own.
static EVENTS: Mutex<Option<Sender<granite_core::msg::Event>>> = Mutex::new(None);

/// The report waiting for the first broker session.
static PENDING: Mutex<Option<String>> = Mutex::new(None);

/// Read the summary of a dump this boot's reset reason says is fresh,
/// and park the report for the first broker session.
///
/// `report` is `sys.coredump_report`: with it false nothing is read and
/// nothing is published, and the dump is still downloadable.
pub fn init(reason: BootReason, report: bool, events: Option<Sender<granite_core::msg::Event>>) {
    if let Ok(mut guard) = EVENTS.lock() {
        *guard = events;
    }
    let crashed = matches!(
        reason,
        BootReason::Panic | BootReason::TaskWatchdog | BootReason::IntWatchdog
    );
    if !crashed {
        return;
    }
    if !report {
        log::warn!(
            "coredump: this boot follows a {reason:?} and sys.coredump_report is false, so \
             nothing is published; the dump is still at GET /api/v1/coredump"
        );
        return;
    }
    let check = unsafe { esp_core_dump_image_check() };
    if check != ESP_OK {
        log::warn!(
            "coredump: this boot follows a {reason:?} but there is no usable dump in flash \
             (esp_core_dump_image_check: {check})"
        );
        return;
    }
    let Some(detail) = summary() else {
        return;
    };
    log::error!("coredump: {detail}");
    if let Ok(mut guard) = PENDING.lock() {
        *guard = Some(detail);
    }
}

/// The summary as one line, or `None` when ESP-IDF could not read it.
fn summary() -> Option<String> {
    // About 1.1 KB (the struct carries a 1 KB stack dump), so it goes
    // on the heap rather than this thread's stack.
    let mut summary = Box::new(esp_core_dump_summary_t::default());
    let err = unsafe { esp_core_dump_get_summary(&raw mut *summary) };
    if err != ESP_OK {
        log::warn!("coredump: the dump is there but its summary did not read ({err})");
        return None;
    }
    let task = cstr(&summary.exc_task);
    let sha = cstr_bytes(&summary.app_elf_sha256);
    let sha: String = sha.chars().take(SHA_DIGITS).collect();
    Some(format!(
        "task {task}, pc {:#010x}, ra {:#010x}, cause {}, mtval {:#010x}, image {sha}",
        summary.exc_pc, summary.ex_info.ra, summary.ex_info.mcause, summary.ex_info.mtval,
    ))
}

/// First broker session after a crash: publish the report, once.
pub fn report() {
    let pending = match PENDING.lock() {
        Ok(mut guard) => guard.take(),
        Err(_) => return,
    };
    let Some(detail) = pending else {
        return;
    };
    let Ok(guard) = EVENTS.lock() else {
        return;
    };
    if let Some(tx) = guard.as_ref() {
        let _ = tx.send(sys_event(
            crate::platform::now_ms(),
            "coredump",
            Some(detail),
        ));
    }
}

/// A NUL-terminated `c_char` array as a string. `c_char` and `u8` have
/// the same layout whichever sign the target gives it, so the bytes are
/// read as bytes.
fn cstr(raw: &[core::ffi::c_char]) -> String {
    let bytes = unsafe { core::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), raw.len()) };
    cstr_bytes(bytes)
}

/// The same for a `u8` array.
fn cstr_bytes(raw: &[u8]) -> String {
    let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

// ---------------------------------------------------------------------
// The download
// ---------------------------------------------------------------------

/// The dump in flash, for `GET /api/v1/coredump`.
pub struct Image {
    partition: *const esp_partition_t,
    /// Length of the dump, checksum included.
    len: usize,
}

impl Image {
    /// How many bytes the download is.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when the dump is empty, which [`image`] already rules out.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Read part of the dump. `offset` is relative to the start of the
    /// partition, which is where the dump starts.
    pub fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, String> {
        let want = buf.len().min(self.len.saturating_sub(offset));
        if want == 0 {
            return Ok(0);
        }
        let err = unsafe {
            esp_partition_read(self.partition, offset, buf.as_mut_ptr().cast(), want)
        };
        if err == ESP_OK {
            Ok(want)
        } else {
            Err(format!("flash read at {offset} failed ({err})"))
        }
    }
}

/// The dump in flash, if there is a valid one.
///
/// Whatever this boot's reset reason was: a dump stays readable until
/// the next crash overwrites it, and that is the point of being able to
/// fetch it from a board that has since been rebooted.
pub fn image() -> Option<Image> {
    if unsafe { esp_core_dump_image_check() } != ESP_OK {
        return None;
    }
    let mut addr: usize = 0;
    let mut len: usize = 0;
    if unsafe { esp_core_dump_image_get(&mut addr, &mut len) } != ESP_OK || len == 0 {
        return None;
    }
    let partition = unsafe {
        esp_partition_find_first(
            esp_partition_type_t_ESP_PARTITION_TYPE_DATA,
            esp_partition_subtype_t_ESP_PARTITION_SUBTYPE_DATA_COREDUMP,
            core::ptr::null(),
        )
    };
    if partition.is_null() {
        return None;
    }
    let size = unsafe { (*partition).size } as usize;
    Some(Image {
        partition,
        len: len.min(size),
    })
}
