//! Firmware update and boot safety (ADR 0001 component 11).
//!
//! What carries over from the wfi028t prior art
//! (`~/Electronics/wfi028t-controller/docs/adr/0002-ota-and-console-port.md`,
//! `fw/src/ota.rs`) is the *probation model*, not the mechanism: an image
//! boots on probation and has to prove itself on a concrete readiness
//! ladder inside a deadline, every step is logged, and failing it resets
//! the chip so the rollback bootloader aborts the slot. The partition
//! writing, the ed25519 header and the TCP push port do not carry over:
//! here `esp_ota_*` writes the slot and the ESP-IDF app signature block
//! (`sdkconfig.defaults.signing`) authenticates the image.
//!
//! The ladder (ADR: within `t_validate`, default 10 min):
//!
//!   1. expanders initialised - the hardware layer calls [`mark_expanders`],
//!   2. link and an IP - the network monitor calls [`mark_link`],
//!   3. **either** the broker connected ([`mark_broker`]) **or** an
//!      authenticated HTTPS request served ([`mark_https`]).
//!
//! A step whose provider never registered (no hardware layer in this
//! build, MQTT disabled in the config) is not required: see
//! [`expect_expanders`] and [`expect_broker`]. Steps nobody can satisfy
//! would turn every boot into a rollback.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
use esp_idf_svc::ota::{EspOta, Slot, SlotState};
use esp_idf_svc::sys::{
    EspError, esp, esp_ota_abort, esp_ota_begin, esp_ota_end, esp_ota_get_next_update_partition,
    esp_ota_get_running_partition, esp_ota_get_state_partition, esp_ota_handle_t,
    esp_ota_img_states_t, esp_ota_img_states_t_ESP_OTA_IMG_NEW,
    esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY, esp_ota_set_boot_partition, esp_ota_write,
    esp_partition_t, mbedtls_sha256_context, mbedtls_sha256_finish, mbedtls_sha256_free,
    mbedtls_sha256_init, mbedtls_sha256_starts, mbedtls_sha256_update,
};
use esp_idf_svc::tls::X509;
use granite_core::api::{OtaFinish, OtaSink as ApiOtaSink};
use granite_core::msg::{Event, EventKind};

use super::identity::hex;

/// How often the probation ladder is re-evaluated.
const PROBATION_TICK: Duration = Duration::from_secs(2);
/// Let the last log line reach the console before a probation reset.
const REBOOT_DELAY: Duration = Duration::from_secs(1);
/// Read buffer for an HTTPS pull.
const PULL_CHUNK: usize = 4096;

/// Reported OTA state of the running image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OtaState {
    /// Running from the factory partition.
    Factory,
    /// Marked valid.
    Valid,
    /// On probation: not confirmed yet.
    Pending,
    /// Marked invalid (should not be running).
    Invalid,
    /// `otadata` says nothing, which is what a bench flash leaves behind.
    #[default]
    Unknown,
}

impl OtaState {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            OtaState::Factory => "factory",
            OtaState::Valid => "valid",
            OtaState::Pending => "pending",
            OtaState::Invalid => "invalid",
            OtaState::Unknown => "unknown",
        }
    }

    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(code: u8) -> Self {
        match code {
            0 => OtaState::Factory,
            1 => OtaState::Valid,
            2 => OtaState::Pending,
            3 => OtaState::Invalid,
            _ => OtaState::Unknown,
        }
    }
}

impl core::fmt::Display for OtaState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

static STATE: AtomicU8 = AtomicU8::new(4); // Unknown

/// The OTA state of the running image, for `status`, `/id` and the LED.
pub fn ota_state() -> OtaState {
    OtaState::from_code(STATE.load(Ordering::Relaxed))
}

/// Readiness flags of the probation ladder.
mod ready {
    use std::sync::atomic::AtomicBool;

    pub static EXPANDERS: AtomicBool = AtomicBool::new(false);
    pub static LINK: AtomicBool = AtomicBool::new(false);
    pub static BROKER: AtomicBool = AtomicBool::new(false);
    pub static HTTPS: AtomicBool = AtomicBool::new(false);
    pub static WANT_EXPANDERS: AtomicBool = AtomicBool::new(false);
    pub static WANT_BROKER: AtomicBool = AtomicBool::new(false);
}

/// The hardware layer announces that it will report expander readiness.
/// Without this the ladder does not wait for step 1.
pub fn expect_expanders() {
    ready::WANT_EXPANDERS.store(true, Ordering::Relaxed);
}

/// The MQTT worker announces that a broker is configured, so "broker
/// connected" is a way to pass step 3.
pub fn expect_broker() {
    ready::WANT_BROKER.store(true, Ordering::Relaxed);
}

/// Step 1: both expanders configured.
pub fn mark_expanders() {
    ready::EXPANDERS.store(true, Ordering::Relaxed);
}

/// Step 2: link up and an IP.
pub fn mark_link() {
    ready::LINK.store(true, Ordering::Relaxed);
}

/// Step 3a: the configured broker connected.
pub fn mark_broker() {
    ready::BROKER.store(true, Ordering::Relaxed);
}

/// Step 3b: an authenticated HTTPS request was served.
pub fn mark_https() {
    ready::HTTPS.store(true, Ordering::Relaxed);
}

fn ladder_passed() -> Option<&'static str> {
    if ready::WANT_EXPANDERS.load(Ordering::Relaxed) && !ready::EXPANDERS.load(Ordering::Relaxed) {
        return None;
    }
    if !ready::LINK.load(Ordering::Relaxed) {
        return None;
    }
    let broker = ready::WANT_BROKER.load(Ordering::Relaxed) && ready::BROKER.load(Ordering::Relaxed);
    let https = ready::HTTPS.load(Ordering::Relaxed);
    match (broker, https) {
        (true, _) => Some("expanders, link and the broker"),
        (false, true) => Some("expanders, link and an authenticated https request"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Slot information
// ---------------------------------------------------------------------------

/// What the firmware knows about the three app partitions.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OtaInfo {
    /// Label of the running partition.
    pub running: String,
    /// State of the running partition.
    pub state: String,
    /// Label of the partition the next image would be written to.
    pub next: String,
    /// Label of the partition the bootloader would pick now.
    pub boot: String,
    /// Label of the last slot marked invalid, if any.
    pub last_invalid: Option<String>,
    /// Version string of the running image, from the app descriptor.
    pub version: String,
}

fn slot_label(slot: &Slot) -> String {
    slot.label.as_str().to_string()
}

fn state_name(state: SlotState) -> &'static str {
    match state {
        SlotState::Factory => "factory",
        SlotState::Valid => "valid",
        SlotState::Invalid => "invalid",
        SlotState::Unverified => "unverified",
        SlotState::Unknown => "unknown",
    }
}

/// Read the slot table. `EspOta` is a singleton, so this takes it, reads,
/// and drops it again.
pub fn info() -> Result<OtaInfo, EspError> {
    let ota = EspOta::new()?;
    let running = ota.get_running_slot()?;
    let next = ota.get_update_slot()?;
    let boot = ota.get_boot_slot()?;
    let last_invalid = ota.get_last_invalid_slot()?.map(|s| slot_label(&s));
    Ok(OtaInfo {
        running: slot_label(&running),
        state: state_name(running.state).to_string(),
        next: slot_label(&next),
        boot: slot_label(&boot),
        last_invalid,
        version: running
            .firmware
            .as_ref()
            .map(|f| f.version.as_str().to_string())
            .unwrap_or_default(),
    })
}

/// Raw `esp_ota_img_states_t` of the running partition.
fn running_img_state() -> Option<esp_ota_img_states_t> {
    let partition = unsafe { esp_ota_get_running_partition() };
    if partition.is_null() {
        return None;
    }
    let mut state: esp_ota_img_states_t = 0;
    if unsafe { esp_ota_get_state_partition(partition, &mut state) } == 0 {
        Some(state)
    } else {
        None
    }
}

/// True when the running image has not confirmed itself yet.
///
/// `ESP_OTA_IMG_NEW` counts: the bootloader moves it to
/// `PENDING_VERIFY` on the boot after the one that wrote the slot, and an
/// image that finds itself in either state has not been confirmed.
pub fn is_pending_verify() -> bool {
    // Comparisons rather than a pattern: the bindgen constants are not
    // upper case, and a lowercase constant in a pattern is a binding, not
    // a comparison.
    match running_img_state() {
        Some(state) => {
            state == esp_ota_img_states_t_ESP_OTA_IMG_NEW
                || state == esp_ota_img_states_t_ESP_OTA_IMG_PENDING_VERIFY
        }
        None => false,
    }
}

/// Mark the running image valid (the `ota-mark-valid` console command and
/// the Firmware page button).
pub fn mark_valid() -> Result<(), EspError> {
    let mut ota = EspOta::new()?;
    ota.mark_running_slot_valid()?;
    STATE.store(OtaState::Valid.code(), Ordering::Relaxed);
    log::warn!("ota: running image marked valid");
    Ok(())
}

/// Mark the running image invalid and let ESP-IDF reboot into the previous
/// slot. Only returns on failure.
pub fn mark_invalid_and_reboot() -> EspError {
    match EspOta::new() {
        Ok(mut ota) => ota.mark_running_slot_invalid_and_reboot(),
        Err(e) => e,
    }
}

// ---------------------------------------------------------------------------
// Probation
// ---------------------------------------------------------------------------

/// Decide the boot-time OTA state and, if the image is on probation, start
/// the validation ladder in its own thread.
///
/// `events` gets one `ota` event per step, so the probation shows up in the
/// MQTT log the same way the prior art put it in the capture log.
pub fn start(t_validate_s: u32, events: Option<Sender<Event>>) {
    let state = match info() {
        Ok(info) => {
            log::info!(
                "ota: running {} ({}), next {}, boot {}{}",
                info.running,
                info.state,
                info.next,
                info.boot,
                info.last_invalid
                    .as_ref()
                    .map(|l| format!(", last invalid {l}"))
                    .unwrap_or_default()
            );
            match info.state.as_str() {
                "factory" => OtaState::Factory,
                "valid" => OtaState::Valid,
                "invalid" => OtaState::Invalid,
                "unverified" => OtaState::Pending,
                _ => OtaState::Unknown,
            }
        }
        Err(e) => {
            log::warn!("ota: slot table unreadable ({e}); treating the image as unknown");
            OtaState::Unknown
        }
    };
    STATE.store(state.code(), Ordering::Relaxed);

    // `Unverified` covers ESP_OTA_IMG_NEW as well; only a real
    // pending-verify needs the ladder.
    if state != OtaState::Pending || !is_pending_verify() {
        return;
    }

    let window = if t_validate_s == 0 { 600 } else { t_validate_s };
    log::warn!("ota: this image is on probation, {window} s to confirm");
    emit(&events, "pending_verify", &format!("{window} s to confirm"));

    let spawned = thread::Builder::new()
        .name("ota-probation".into())
        .stack_size(4096)
        .spawn(move || probation(window, events));
    if let Err(e) = spawned {
        log::error!("ota: probation thread could not start ({e}); marking the image valid");
        let _ = mark_valid();
    }
}

fn probation(window_s: u32, events: Option<Sender<Event>>) {
    let deadline = super::now_ms() + u64::from(window_s) * 1000;
    loop {
        if let Some(why) = ladder_passed() {
            match mark_valid() {
                Ok(()) => {
                    log::warn!("ota: image confirmed ({why})");
                    emit(&events, "valid", why);
                }
                Err(e) => {
                    // Could not write otadata: stay pending, which is the
                    // safe direction - the next reset rolls back.
                    log::error!("ota: image works ({why}) but otadata could not be written: {e}");
                    emit(&events, "failed", &format!("otadata write failed: {e}"));
                }
            }
            return;
        }
        if super::now_ms() >= deadline {
            let detail = format!(
                "expanders={} link={} broker={} https={}",
                ready::EXPANDERS.load(Ordering::Relaxed),
                ready::LINK.load(Ordering::Relaxed),
                ready::BROKER.load(Ordering::Relaxed),
                ready::HTTPS.load(Ordering::Relaxed),
            );
            log::error!(
                "ota: not confirmed within {window_s} s ({detail}); marking the image invalid, \
                 the bootloader rolls back"
            );
            emit(&events, "rolled_back", &detail);
            STATE.store(OtaState::Invalid.code(), Ordering::Relaxed);
            thread::sleep(REBOOT_DELAY);
            // Releases the relays first, then hands over to ESP-IDF.
            super::release_outputs();
            let e = mark_invalid_and_reboot();
            // Only reached when ESP-IDF refused, e.g. there is no other
            // app to roll back to. A plain restart leaves the slot
            // pending, which the bootloader aborts on its own.
            log::error!("ota: rollback refused ({e}); restarting anyway");
            super::planned_reboot("ota_probation_failed");
        }
        thread::sleep(PROBATION_TICK);
    }
}

/// One `ota` event. `phase` uses the vocabulary
/// [`EventKind::Ota`] documents.
fn emit(events: &Option<Sender<Event>>, phase: &str, detail: &str) {
    if let Some(tx) = events {
        let _ = tx.send(Event::new(
            super::now_ms(),
            EventKind::Ota {
                phase: phase.to_string(),
                progress: None,
                detail: Some(detail.to_string()),
            },
        ));
    }
}

// ---------------------------------------------------------------------------
// Writing an image
// ---------------------------------------------------------------------------

/// Anything that can stop an update.
#[derive(Debug)]
pub enum OtaError {
    /// ESP-IDF refused (no slot, flash error, image header rejected).
    Esp(EspError),
    /// The image hash did not match what the caller promised.
    Sha {
        /// What the caller said.
        expected: String,
        /// What arrived.
        got: String,
    },
    /// The transfer stopped early or the source failed.
    Transfer(String),
    /// Nothing was written.
    Empty,
}

impl core::fmt::Display for OtaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            OtaError::Esp(e) => write!(f, "esp-idf: {e}"),
            OtaError::Sha { expected, got } => {
                write!(f, "sha256 mismatch: expected {expected}, got {got}")
            }
            OtaError::Transfer(m) => write!(f, "transfer failed: {m}"),
            OtaError::Empty => f.write_str("no image data"),
        }
    }
}

impl std::error::Error for OtaError {}

impl From<EspError> for OtaError {
    fn from(e: EspError) -> Self {
        OtaError::Esp(e)
    }
}

/// Streaming SHA-256 over whatever is written to the slot.
struct Sha256Stream(mbedtls_sha256_context);

impl Sha256Stream {
    fn new() -> Self {
        let mut ctx: mbedtls_sha256_context = unsafe { core::mem::zeroed() };
        unsafe {
            mbedtls_sha256_init(&mut ctx);
            mbedtls_sha256_starts(&mut ctx, 0);
        }
        Sha256Stream(ctx)
    }

    fn update(&mut self, data: &[u8]) {
        unsafe { mbedtls_sha256_update(&mut self.0, data.as_ptr(), data.len()) };
    }

    fn finish(&mut self) -> [u8; 32] {
        let mut out = [0u8; 32];
        unsafe { mbedtls_sha256_finish(&mut self.0, out.as_mut_ptr()) };
        out
    }
}

impl Drop for Sha256Stream {
    fn drop(&mut self) {
        unsafe { mbedtls_sha256_free(&mut self.0) };
    }
}

/// Result of a completed write.
#[derive(Debug, Clone)]
pub struct Written {
    /// Bytes written.
    pub len: usize,
    /// SHA-256 of what was written, lowercase hex.
    pub sha256: String,
    /// Slot the image went into.
    pub slot: String,
}

/// The one thing in the firmware that writes an app partition.
///
/// It implements [`granite_core::api::OtaSink`], which is a long-lived
/// `begin / write* / finish` object the HTTP upload route holds across
/// many socket reads. That rules out `EspOta::initiate_update`, whose
/// `EspOtaUpdate` borrows the `EspOta` it came from and so cannot be
/// stored next to it; `esp_ota_*` is called directly instead. `EspOta`
/// still does everything that needs no borrow: the slot table,
/// `mark_valid`, the rollback.
///
/// The image is hashed while it is written, so a truncated or tampered
/// transfer is caught without a second pass over flash. *Authenticity* is
/// ESP-IDF's job: with `sdkconfig.defaults.signing` in the build,
/// `esp_ota_end` refuses an image whose signature does not verify against
/// the embedded public key.
pub struct OtaWriter {
    partition: *const esp_partition_t,
    handle: Option<esp_ota_handle_t>,
    sha: Option<Sha256Stream>,
    written: u64,
    expected_sha256: Option<String>,
    slot: String,
}

// The only non-Send field is a pointer into the partition table, which
// lives in flash for the lifetime of the firmware and is read-only.
unsafe impl Send for OtaWriter {}

impl Default for OtaWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl OtaWriter {
    /// An idle writer. Nothing touches flash until [`OtaWriter::begin`].
    pub fn new() -> Self {
        OtaWriter {
            partition: core::ptr::null(),
            handle: None,
            sha: None,
            written: 0,
            expected_sha256: None,
            slot: String::new(),
        }
    }

    /// Require this SHA-256 (hex) of the image before the boot partition
    /// is switched. The MQTT/API `ota` pull sets it; a push from the setup
    /// page has nothing to compare against and relies on the image
    /// signature instead.
    pub fn expect_sha256(&mut self, sha256: &str) {
        self.expected_sha256 = Some(sha256.trim().to_ascii_lowercase());
    }

    /// True while an update is open.
    pub fn is_open(&self) -> bool {
        self.handle.is_some()
    }

    /// Bytes written so far.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Open the inactive slot. `total_len` erases only what is needed.
    pub fn begin(&mut self, total_len: Option<u64>) -> Result<(), OtaError> {
        if self.handle.is_some() {
            return Err(OtaError::Transfer(String::from(
                "an update is already in progress",
            )));
        }
        let partition = unsafe { esp_ota_get_next_update_partition(core::ptr::null()) };
        if partition.is_null() {
            return Err(OtaError::Transfer(String::from(
                "no OTA slot to write (is the partition table the granite one?)",
            )));
        }
        self.slot = partition_label(partition);
        let size = match total_len {
            Some(n) if n > 0 => n as usize,
            // OTA_SIZE_UNKNOWN erases the whole slot, which is slower but
            // always correct.
            _ => usize::MAX,
        };
        let mut handle: esp_ota_handle_t = 0;
        esp!(unsafe { esp_ota_begin(partition, size, &mut handle) })?;
        self.partition = partition;
        self.handle = Some(handle);
        self.sha = Some(Sha256Stream::new());
        self.written = 0;
        log::warn!(
            "ota: writing {} ({})",
            self.slot,
            total_len
                .map(|n| format!("{n} bytes"))
                .unwrap_or_else(|| String::from("size unknown"))
        );
        Ok(())
    }

    /// Append a chunk.
    pub fn write(&mut self, chunk: &[u8]) -> Result<(), OtaError> {
        if chunk.is_empty() {
            return Ok(());
        }
        let Some(handle) = self.handle else {
            return Err(OtaError::Transfer(String::from("no update in progress")));
        };
        esp!(unsafe { esp_ota_write(handle, chunk.as_ptr().cast(), chunk.len()) })?;
        if let Some(sha) = self.sha.as_mut() {
            sha.update(chunk);
        }
        let before = self.written;
        self.written += chunk.len() as u64;
        if before / (64 * 1024) != self.written / (64 * 1024) {
            log::info!("ota: {} bytes", self.written);
        }
        Ok(())
    }

    /// Verify the hash, close the update, and point the bootloader at the
    /// new slot. The caller reboots.
    pub fn finish(&mut self) -> Result<Written, OtaError> {
        let Some(handle) = self.handle.take() else {
            return Err(OtaError::Transfer(String::from("no update in progress")));
        };
        let digest = self
            .sha
            .as_mut()
            .map(|sha| hex(&sha.finish()))
            .unwrap_or_default();
        self.sha = None;
        let len = self.written;

        if len == 0 {
            let _ = unsafe { esp_ota_abort(handle) };
            return Err(OtaError::Empty);
        }
        if let Some(expected) = self.expected_sha256.clone()
            && expected != digest
        {
            let _ = unsafe { esp_ota_abort(handle) };
            return Err(OtaError::Sha {
                expected,
                got: digest,
            });
        }

        // esp_ota_end validates the image header and, with signing on, the
        // signature block; only then is the boot partition switched.
        esp!(unsafe { esp_ota_end(handle) })?;
        esp!(unsafe { esp_ota_set_boot_partition(self.partition) })?;
        log::warn!(
            "ota: {} written, {len} bytes, sha256 {digest}, boot partition switched",
            self.slot
        );
        Ok(Written {
            len: len as usize,
            sha256: digest,
            slot: self.slot.clone(),
        })
    }

    /// Give up on the image in progress. `otadata` is left untouched.
    pub fn abort(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = unsafe { esp_ota_abort(handle) };
            log::warn!("ota: update aborted after {} bytes", self.written);
        }
        self.sha = None;
        self.written = 0;
        self.expected_sha256 = None;
    }
}

impl Drop for OtaWriter {
    fn drop(&mut self) {
        self.abort();
    }
}

/// `granite_core::api::OtaSink` over [`OtaWriter`]: the shape the HTTP
/// upload route holds in an `Arc<Mutex<dyn OtaSink + Send>>`.
impl ApiOtaSink for OtaWriter {
    fn begin(&mut self, total_len: Option<u64>) -> Result<(), String> {
        OtaWriter::begin(self, total_len).map_err(|e| e.to_string())
    }

    fn write(&mut self, chunk: &[u8]) -> Result<(), String> {
        OtaWriter::write(self, chunk).map_err(|e| e.to_string())
    }

    fn finish(&mut self) -> Result<OtaFinish, String> {
        let written = OtaWriter::finish(self).map_err(|e| e.to_string())?;
        Ok(OtaFinish {
            slot: written.slot,
            bytes: written.len as u64,
            reboot_required: true,
        })
    }

    fn abort(&mut self) {
        OtaWriter::abort(self);
    }
}

fn partition_label(partition: *const esp_partition_t) -> String {
    if partition.is_null() {
        return String::new();
    }
    let label = unsafe { (*partition).label };
    let end = label.iter().position(|c| *c == 0).unwrap_or(label.len());
    String::from_utf8_lossy(&label[..end]).into_owned()
}

/// Pull an image over HTTPS and install it.
///
/// `ca_pem` is the trust anchor; empty means "use the ESP-IDF global CA
/// store", which is nothing unless the build attached a bundle, so a
/// configured CA is effectively required.
pub fn pull(url: &str, sha256: &str, ca_pem: &str) -> Result<Written, OtaError> {
    // esp-idf-svc wants a 'static X509, and the connection lives only for
    // this call; leak one copy per pull rather than keep a global.
    let ca: Option<X509<'static>> = if ca_pem.trim().is_empty() {
        None
    } else {
        let mut bytes = ca_pem.trim().as_bytes().to_vec();
        bytes.push(0);
        let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        Some(X509::pem(
            core::ffi::CStr::from_bytes_with_nul(leaked)
                .map_err(|e| OtaError::Transfer(format!("ca pem: {e}")))?,
        ))
    };

    let mut conn = EspHttpConnection::new(&HttpConfiguration {
        buffer_size: Some(PULL_CHUNK),
        server_certificate: ca,
        timeout: Some(Duration::from_secs(30)),
        follow_redirects_policy: esp_idf_svc::http::client::FollowRedirectsPolicy::FollowAll,
        ..Default::default()
    })?;
    conn.initiate_request(esp_idf_svc::http::Method::Get, url, &[])?;
    conn.initiate_response()?;
    let status = conn.status();
    if status != 200 {
        return Err(OtaError::Transfer(format!("http {status} for {url}")));
    }
    let len: Option<u64> = conn
        .header("Content-Length")
        .and_then(|v| v.trim().parse().ok());
    log::warn!("ota: pulling {url}");

    let mut writer = OtaWriter::new();
    writer.expect_sha256(sha256);
    writer.begin(len)?;

    let mut buf = [0u8; PULL_CHUNK];
    let outcome = loop {
        match conn.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => {
                if let Err(e) = writer.write(&buf[..n]) {
                    break Err(e);
                }
            }
            Err(e) => break Err(OtaError::Transfer(format!("read: {e}"))),
        }
    };
    if let Err(e) = outcome {
        writer.abort();
        return Err(e);
    }
    if let Some(expected) = len
        && writer.written() != expected
    {
        let got = writer.written();
        writer.abort();
        return Err(OtaError::Transfer(format!(
            "short read: {got} of {expected} bytes"
        )));
    }
    writer.finish()
}

/// True once an update has switched the boot partition and the firmware is
/// on its way to a reboot, so other tasks can stop accepting work.
static REBOOTING: AtomicBool = AtomicBool::new(false);

/// True while a planned OTA reboot is in progress.
pub fn rebooting() -> bool {
    REBOOTING.load(Ordering::Relaxed)
}

/// Reboot into a freshly written image through the planned-reboot path:
/// relays released first, then restart.
pub fn reboot_into_new_image() -> ! {
    REBOOTING.store(true, Ordering::Relaxed);
    super::planned_reboot("ota")
}
