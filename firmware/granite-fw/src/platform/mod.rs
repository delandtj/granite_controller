//! Platform layer (ADR 0001 components 6, 10, 11, 12, 13): NVS store,
//! identity and recovery, network with commit-confirm, SNTP, console on USB
//! Serial/JTAG, OTA with probation, log ring.
//!
//! [`Platform`] is what `main.rs` builds once and hands around. Everything
//! in it is either `Send + Sync` on its own or behind a `Mutex`, so the
//! HTTP, MQTT, Modbus and console tasks can all hold the same `Arc`.
//!
//! Boot order matters and is fixed in `main.rs`, not here: the expander
//! reset lines go low before this module is called, because nothing in the
//! platform layer is worth a closed relay.

pub mod api_impl;
pub mod console;
pub mod identity;
pub mod logring;
pub mod net;
pub mod ota;
pub mod store;
#[cfg(feature = "wifi-dev")]
pub mod wifi_dev;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::netif::NetifStack;
use esp_idf_svc::sys::{
    esp_reset_reason, esp_reset_reason_t_ESP_RST_BROWNOUT, esp_reset_reason_t_ESP_RST_INT_WDT,
    esp_reset_reason_t_ESP_RST_PANIC, esp_reset_reason_t_ESP_RST_POWERON,
    esp_reset_reason_t_ESP_RST_SW, esp_reset_reason_t_ESP_RST_TASK_WDT,
    esp_reset_reason_t_ESP_RST_WDT, esp_restart, esp_timer_get_time,
    mbedtls_md_type_t_MBEDTLS_MD_SHA256, mbedtls_pkcs5_pbkdf2_hmac_ext,
};
use granite_core::api::MqttStatus;
use granite_core::config::{Config, LogLevel, Section, Secrets};
use granite_core::hal::{BootReason, Clock, HalResult, LinkError, PlaneLink, Reboot};
use granite_core::msg::Event;
use granite_core::observed::Observed;

use identity::Identity;
use net::{Net, NetHw};
use store::Store;

/// Firmware version, from `Cargo.toml`.
pub const FW_VERSION: &str = env!("CARGO_PKG_VERSION");

/// PBKDF2 iterations for the admin password (ADR component 8).
pub const PBKDF2_ITERS: u32 = 20_000;
/// Salt length in bytes.
pub const PBKDF2_SALT: usize = 16;
/// Derived key length in bytes.
pub const PBKDF2_LEN: usize = 32;

/// Monotonic milliseconds since boot. The one clock the core compares with.
pub fn now_ms() -> u64 {
    (unsafe { esp_timer_get_time() } / 1000) as u64
}

/// [`Clock`] for the core.
#[derive(Debug, Clone, Copy, Default)]
pub struct EspClock;

impl Clock for EspClock {
    fn now_ms(&self) -> u64 {
        now_ms()
    }
}

/// Why this boot happened.
pub fn boot_reason() -> BootReason {
    match unsafe { esp_reset_reason() } {
        r if r == esp_reset_reason_t_ESP_RST_POWERON => BootReason::PowerOn,
        r if r == esp_reset_reason_t_ESP_RST_SW => BootReason::Software,
        r if r == esp_reset_reason_t_ESP_RST_TASK_WDT => BootReason::TaskWatchdog,
        r if r == esp_reset_reason_t_ESP_RST_INT_WDT || r == esp_reset_reason_t_ESP_RST_WDT => {
            BootReason::IntWatchdog
        }
        r if r == esp_reset_reason_t_ESP_RST_BROWNOUT => BootReason::BrownOut,
        r if r == esp_reset_reason_t_ESP_RST_PANIC => BootReason::Panic,
        _ => BootReason::Unknown,
    }
}

/// [`Reboot`] for the core. Always goes through [`planned_reboot`].
#[derive(Debug, Clone, Copy, Default)]
pub struct EspReboot;

impl Reboot for EspReboot {
    fn request_reboot(&mut self) -> HalResult<()> {
        planned_reboot("command")
    }

    fn boot_reason(&self) -> BootReason {
        boot_reason()
    }
}

/// Wall clock as an ISO-8601 UTC string, for `status`. Before the first
/// SNTP sync this reads back the firmware build time.
pub fn wall_clock_string() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let suffix = if net::time_synced() { "Z (sntp)" } else { "Z (build stamp)" };
    format!("{} unix {secs}{suffix}", iso8601(secs))
}

fn iso8601(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Howard Hinnant's days-to-civil, matching `build.rs`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Is the running image signature-checked? True when the build had
/// `sdkconfig.defaults.signing` in `ESP_IDF_SDKCONFIG_DEFAULTS`.
pub const SIGNED_BUILD: bool = cfg!(esp_idf_secure_signed_on_update_no_secure_boot);

/// One line about image signing, for `status` and `/id`.
pub fn signing_summary() -> String {
    if SIGNED_BUILD {
        format!("signed images required, key id {}", app_elf_sha256_short())
    } else {
        String::from("unsigned build (see firmware/README.md, \"Signing key\")")
    }
}

/// First 8 hex digits of the running app's ELF SHA-256. ESP-IDF computes
/// it at build time; it is the "key id" the host push tool matches against
/// so an operator can tell which image is running.
pub fn app_elf_sha256_short() -> String {
    let mut buf = [0u8; 17];
    unsafe { esp_idf_svc::sys::esp_app_get_elf_sha256(buf.as_mut_ptr(), buf.len()) };
    let bytes: Vec<u8> = buf
        .iter()
        .take_while(|c| **c != 0)
        .copied()
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

// ---------------------------------------------------------------------------
// Planned reboot
// ---------------------------------------------------------------------------

type ReleaseHook = Box<dyn Fn() + Send + Sync + 'static>;

static RELEASE_HOOK: OnceLock<ReleaseHook> = OnceLock::new();
static REBOOT_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Register the callback that puts the outputs in a safe state.
///
/// Nothing registers one on rev D: the controller drives no node output
/// at all any more (ADR 0004 decision 1), and a plane holds its own
/// state through a controller reboot. The hook stays because the reboot
/// and rollback paths call [`release_outputs`], and a future add-on with
/// an output of its own is where it earns its keep again.
pub fn set_release_hook<F>(hook: F)
where
    F: Fn() + Send + Sync + 'static,
{
    if RELEASE_HOOK.set(Box::new(hook)).is_err() {
        log::warn!("release hook was already registered; keeping the first one");
    }
}

/// Put the outputs in a safe state now, if a hook is registered.
pub fn release_outputs() {
    if let Some(hook) = RELEASE_HOOK.get() {
        hook();
    }
}

/// Put the calling thread under the task watchdog
/// (`CONFIG_ESP_TASK_WDT_TIMEOUT_S`, which panics). From then on it must
/// call [`feed_task_watchdog`] more often than that, or the board
/// resets. For the threads whose silent hang would be dangerous: the
/// dispatcher (the plane client and the rules) and the sense task
/// (ADR 0001, "Tasks and data flow").
pub fn watch_this_thread(name: &str) {
    let err = unsafe { esp_idf_svc::sys::esp_task_wdt_add(core::ptr::null_mut()) };
    if err == esp_idf_svc::sys::ESP_OK {
        log::info!("{name}: under the task watchdog");
    } else {
        log::error!("{name}: cannot join the task watchdog ({err})");
    }
}

/// Tell the task watchdog the calling thread is alive.
pub fn feed_task_watchdog() {
    unsafe {
        esp_idf_svc::sys::esp_task_wdt_reset();
    }
}

/// Longest a planned reboot waits for MQTT to get its last messages out.
const REBOOT_FLUSH: Duration = Duration::from_secs(3);

/// The one way the firmware restarts itself.
///
/// Releases every relay first, gives MQTT up to [`REBOOT_FLUSH`] to
/// deliver what is queued, lets the last log line reach the console, then
/// calls `esp_restart`. Never returns.
pub fn planned_reboot(reason: &str) -> ! {
    if !REBOOT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        log::warn!("planned reboot ({reason})");
    }
    release_outputs();
    // The event and the ack that explain this reboot (an OTA `verified`,
    // the `reboot` command's ack) are still in the MQTT worker's queue.
    if !crate::mqtt::flush(REBOOT_FLUSH) && crate::mqtt::connected() {
        log::warn!("planned reboot: MQTT did not flush within {REBOOT_FLUSH:?}");
    }
    // Long enough for a 115200-equivalent USB flush, short enough that a
    // watchdog does not beat us to it.
    std::thread::sleep(Duration::from_millis(250));
    unsafe { esp_restart() }
}

// ---------------------------------------------------------------------------
// Event bus
// ---------------------------------------------------------------------------

/// A one-to-many [`Event`] fan-out.
///
/// `std` has no broadcast channel and the ADR's data flow needs one
/// (events go to MQTT, to the log and later to the HTTP event stream), so
/// this is a list of senders with a non-blocking send. A subscriber that
/// stops reading is dropped, not waited for.
///
/// Events published before the MQTT worker subscribes (the config
/// fallbacks and the OTA `pending_verify` of a boot) are kept, up to
/// [`BOOT_BACKLOG`], and handed to the first [`EventBus::subscribe_replay`].
pub struct EventBus {
    inner: Mutex<BusInner>,
}

struct BusInner {
    subs: Vec<Sender<Event>>,
    /// Everything published so far, until the first replay takes it.
    backlog: Option<Vec<Event>>,
    /// Boot events that did not fit in the backlog.
    dropped: u32,
}

/// Events kept for the first replaying subscriber; later ones are counted
/// and dropped.
pub const BOOT_BACKLOG: usize = 32;

impl Default for EventBus {
    fn default() -> Self {
        EventBus {
            inner: Mutex::new(BusInner {
                subs: Vec::new(),
                backlog: Some(Vec::new()),
                dropped: 0,
            }),
        }
    }
}

impl EventBus {
    /// Empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a subscriber.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<Event> {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut inner) = self.inner.lock() {
            inner.subs.push(tx);
        }
        rx
    }

    /// Add a subscriber that first gets everything published since boot.
    /// Only the first caller gets the backlog (the MQTT worker); later
    /// callers get a plain subscription.
    pub fn subscribe_replay(&self) -> std::sync::mpsc::Receiver<Event> {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut inner) = self.inner.lock() {
            for event in inner.backlog.take().unwrap_or_default() {
                let _ = tx.send(event);
            }
            inner.subs.push(tx);
        }
        rx
    }

    /// A sender that publishes into the bus. Handy for a component that
    /// only ever emits, like the OTA probation thread.
    pub fn sender(self: &Arc<Self>) -> Sender<Event> {
        let (tx, rx) = std::sync::mpsc::channel();
        let bus = Arc::clone(self);
        std::thread::Builder::new()
            .name("event-relay".into())
            .stack_size(3072)
            .spawn(move || {
                while let Ok(ev) = rx.recv() {
                    bus.publish(ev);
                }
            })
            .ok();
        tx
    }

    /// Hand `event` to every live subscriber.
    pub fn publish(&self, event: Event) {
        if let Ok(mut guard) = self.inner.lock() {
            // Through a plain reference, so the fields borrow separately.
            let inner = &mut *guard;
            if let Some(backlog) = inner.backlog.as_mut() {
                if backlog.len() < BOOT_BACKLOG {
                    backlog.push(event.clone());
                } else {
                    if inner.dropped == 0 {
                        log::warn!("event bus: boot backlog full, later boot events are not replayed");
                    }
                    inner.dropped += 1;
                }
            }
            inner.subs.retain(|tx| tx.send(event.clone()).is_ok());
        }
    }
}

// ---------------------------------------------------------------------------
// Command channel
// ---------------------------------------------------------------------------

/// The channel MQTT, the HTTP API and Modbus all push commands down; the
/// dispatcher thread is the only consumer (ADR, "Tasks and data flow").
///
/// It is [`crate::http::CommandChannel`], defined next to the HTTP server
/// because that is where the ack timeout matters, and reused here so the
/// firmware has exactly one command path.
pub use crate::http::{CommandChannel, CommandRequest};

// ---------------------------------------------------------------------------
// A plane link that is not there
// ---------------------------------------------------------------------------

/// Stand-in for the plane link when the hardware layer did not come up.
///
/// **This is a stub.** Every address answers [`LinkError::Nak`], which
/// the plane client reads as "no plane is strapped there" (ADR 0004
/// decision 5): the controller then has no planes, no nodes, and every
/// node command is refused as absent, which is the truth. `main.rs` uses
/// the hardware layer's link instead whenever `hw::init` succeeded.
#[derive(Debug, Default)]
pub struct NoPlaneLink;

impl PlaneLink for NoPlaneLink {
    fn write_read(&mut self, addr: u8, _req: &[u8], _resp: &mut [u8]) -> Result<usize, LinkError> {
        log::debug!("no plane link: read of {addr:#04x} refused");
        Err(LinkError::Nak)
    }

    fn write(&mut self, addr: u8, _frame: &[u8]) -> Result<(), LinkError> {
        log::debug!("no plane link: write to {addr:#04x} refused");
        Err(LinkError::Nak)
    }

    fn int_asserted(&mut self) -> bool {
        false
    }
}

/// A random 32-bit value from the hardware RNG.
///
/// The plane client seeds its per-plane command sequence numbers with
/// this, so a controller that reboots mid-command cannot collide with
/// the sequence memory a plane still holds (ADR 0003).
pub fn random_u32() -> u32 {
    unsafe { esp_idf_svc::sys::esp_random() }
}

// ---------------------------------------------------------------------------
// Platform
// ---------------------------------------------------------------------------

/// Everything the platform layer owns, shared behind one `Arc`.
pub struct Platform {
    /// NVS. Behind a mutex: the dispatcher, the console and the HTTP API
    /// all write config sections.
    pub store: Mutex<Store>,
    /// Identity, fixed for the life of the boot.
    pub identity: Identity,
    /// Network thread handle.
    pub net: Net,
    /// The live configuration.
    ///
    /// The HTTP task holds the **write** guard for a whole request
    /// (`granite_core::api::ApiCtx` takes `&mut Config`), so anything that
    /// a request can wait on must never block on this lock. The dispatcher
    /// therefore keeps its own working copy and only ever uses
    /// `try_read`/`try_write` here; see `main::dispatcher`.
    pub config: Arc<RwLock<Config>>,
    /// The sensor snapshot every publisher works from.
    pub observed: Arc<RwLock<Observed>>,
    /// Event fan-out.
    pub events: Arc<EventBus>,
    /// Where a transport sends a command.
    pub commands: Arc<CommandChannel>,
    /// Broker state, written by the MQTT worker. Kept out of
    /// [`Platform::config`] on purpose: the status page asks for it while
    /// the HTTP task holds the config guard.
    pub mqtt: Mutex<MqttStatus>,
    /// Sections that were staged at boot and are waiting for a confirm.
    pub staged_at_boot: Vec<Section>,
}

impl Platform {
    /// The device id, which is also the hostname and the MQTT client id
    /// default.
    pub fn device_id(&self) -> &str {
        &self.identity.device_id
    }

    /// A copy of the live config. Blocks while an HTTP request holds the
    /// write guard, so the dispatcher thread must not call this.
    pub fn config(&self) -> Config {
        self.config
            .read()
            .map(|c| c.clone())
            .unwrap_or_else(|_| Config::default())
    }

    /// A copy of the live config, or `None` if someone is writing it. The
    /// dispatcher's way in.
    pub fn try_config(&self) -> Option<Config> {
        self.config.try_read().ok().map(|c| c.clone())
    }

    /// Broker state as the status page wants it.
    pub fn mqtt_status(&self) -> MqttStatus {
        self.mqtt
            .lock()
            .map(|m| m.clone())
            .unwrap_or_else(|_| MqttStatus::default())
    }

    /// Let the MQTT worker publish its state.
    pub fn set_mqtt_status(&self, status: MqttStatus) {
        if let Ok(mut m) = self.mqtt.lock() {
            *m = status;
        }
    }

    /// Publish an event.
    pub fn publish(&self, event: Event) {
        self.events.publish(event);
    }

    /// Set the admin password: PBKDF2-HMAC-SHA256, [`PBKDF2_ITERS`]
    /// iterations, the per-device salt (kept when there already is one,
    /// so API tokens stay valid), and `sec.admin_password_set` flipped so
    /// the HTTP layer leaves first-setup mode.
    pub fn set_admin_password(&self, password: &str) -> Result<(), String> {
        let mut store = self.store.lock().map_err(|_| "store is locked")?;
        let mut secrets = store.load_secrets();
        // The salt is minted once and then kept: API tokens are hashed
        // with it too, and rotating it would invalidate every token.
        // The API path (granite-core api.rs, set_password) does the same.
        let salt = match identity::from_hex(&secrets.admin_salt) {
            Some(salt) if !salt.is_empty() => salt,
            _ => {
                let salt = identity::random_bytes(PBKDF2_SALT);
                secrets.admin_salt = identity::hex(&salt);
                salt
            }
        };
        let hash = pbkdf2(password.as_bytes(), &salt).ok_or("pbkdf2 failed")?;
        secrets.admin_hash = identity::hex(&hash);
        secrets.admin_iters = PBKDF2_ITERS;
        store.save_secrets(&secrets).map_err(|e| e.to_string())?;

        let mut cfg = self.config.write().map_err(|_| "config is locked")?;
        cfg.sec.admin_password_set = true;
        store
            .save_section(&cfg, Section::Sec)
            .map_err(|e| e.to_string())?;
        drop(cfg);
        log::warn!("admin password set");
        Ok(())
    }

    /// Check a password against the stored hash. `false` when no password
    /// is set, which is what keeps first-setup mode closed to everything
    /// else.
    pub fn check_admin_password(&self, password: &str) -> bool {
        let Ok(mut store) = self.store.lock() else {
            return false;
        };
        let secrets: Secrets = store.load_secrets();
        if secrets.admin_hash.is_empty() || secrets.admin_salt.is_empty() {
            return false;
        }
        let Some(salt) = identity::from_hex(&secrets.admin_salt) else {
            return false;
        };
        let Some(hash) = pbkdf2_iters(password.as_bytes(), &salt, secrets.admin_iters) else {
            return false;
        };
        constant_time_eq(identity::hex(&hash).as_bytes(), secrets.admin_hash.as_bytes())
    }
}

/// PBKDF2-HMAC-SHA256 with the shipped iteration count.
pub fn pbkdf2(password: &[u8], salt: &[u8]) -> Option<[u8; PBKDF2_LEN]> {
    pbkdf2_iters(password, salt, PBKDF2_ITERS)
}

/// PBKDF2-HMAC-SHA256 with an explicit iteration count, so an old stored
/// hash still verifies after the default changes.
pub fn pbkdf2_iters(password: &[u8], salt: &[u8], iters: u32) -> Option<[u8; PBKDF2_LEN]> {
    let mut out = [0u8; PBKDF2_LEN];
    let rc = unsafe {
        mbedtls_pkcs5_pbkdf2_hmac_ext(
            mbedtls_md_type_t_MBEDTLS_MD_SHA256,
            password.as_ptr(),
            password.len(),
            salt.as_ptr(),
            salt.len(),
            if iters == 0 { PBKDF2_ITERS } else { iters },
            out.len() as u32,
            out.as_mut_ptr(),
        )
    };
    if rc == 0 { Some(out) } else { None }
}

/// Comparison whose duration does not depend on where the first
/// difference is.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Bring the platform up.
///
/// In order: NVS, config, log level, identity (which generates the
/// recovery token and the device certificate on a first boot), OTA state
/// and probation, then the network thread. The console is started by
/// `main.rs` once the `Arc<Platform>` exists.
pub fn init(
    net_hw: NetHw,
    sysloop: EspSystemEventLoop,
    commands: Arc<CommandChannel>,
    observed: Arc<RwLock<Observed>>,
) -> anyhow::Result<Arc<Platform>> {
    // lwIP and esp_netif first, before anything that might want a socket.
    //
    // esp-idf-svc does this lazily, when the first `EspNetif` is created -
    // which is inside the Ethernet bring-up. On a board whose W5500 does
    // not answer, that bring-up fails before it gets there, the lwIP
    // TCP/IP task never starts, and the next thing to open a socket (the
    // HTTPS server) aborts the firmware on `tcpip_send_msg_wait_sem
    // (Invalid mbox)`. A missing W5500 has to stay a logged error.
    if let Err(e) = NetifStack::initialize() {
        log::error!("esp_netif could not be initialised: {e}");
    }

    let mut store = Store::new()?;
    log::info!("nvs ready");

    let first_boot = store.is_blank();
    let (mut config, fallbacks) = store.load_config();
    if first_boot {
        log::warn!("no stored configuration; writing the defaults");
        if let Err(e) = store.save_all(&config) {
            log::error!("defaults could not be written: {e}");
        }
    } else {
        log::info!(
            "configuration loaded (schema {}), mqtt {}, modbus {}, ip {}",
            config.schema_version,
            if config.mqtt.enabled { "on" } else { "off" },
            if config.sec.modbus.enabled { "on" } else { "off" },
            match config.net.ip_mode {
                granite_core::config::IpMode::Dhcp => "dhcp",
                granite_core::config::IpMode::Static => "static",
            }
        );
    }
    for fb in &fallbacks {
        log::error!("{fb}");
    }
    config.migrate();
    config.normalise();

    logring::set_publish_level(config.sys.log_level);
    if config.sys.log_level != LogLevel::Warn {
        log::info!("log publish level {:?}", config.sys.log_level);
    }

    let identity = identity::init(&mut store, &config.sys.device_id)?;

    // Anything staged and not confirmed before the last reboot is dead:
    // `cfg` holds the value that is actually running, so drop the staging
    // slots and say so.
    let staged_at_boot = store.staged_sections();
    for section in &staged_at_boot {
        log::warn!("{section} had a staged, unconfirmed value; discarding it");
        if let Err(e) = store.clear_staged(*section) {
            log::error!("staging slot for {section} could not be cleared: {e}");
        }
    }

    let events = Arc::new(EventBus::new());
    for fb in &fallbacks {
        events.publish(Event::new(
            now_ms(),
            granite_core::msg::EventKind::Config {
                section: Some(fb.section),
                change: String::from("fallback"),
                detail: Some(fb.error.to_string()),
            },
        ));
    }

    // OTA before the network: if this image is on probation the ladder has
    // to be watching before the link comes up and sets its flag.
    if config.mqtt.enabled {
        ota::expect_broker();
    }
    ota::start(
        config.sys.t_validate_s,
        Some(events.sender()),
        Some(store.partition()),
    );

    let net = net::init(
        net_hw,
        identity.mac,
        identity.device_id.clone(),
        config.net.clone(),
        sysloop,
    );

    let mqtt = MqttStatus {
        enabled: config.mqtt.enabled,
        connected: false,
        broker: if config.mqtt.host.is_empty() {
            String::new()
        } else {
            format!("{}:{}", config.mqtt.host, config.mqtt.port)
        },
        last_error: String::new(),
    };

    Ok(Arc::new(Platform {
        store: Mutex::new(store),
        identity,
        net,
        config: Arc::new(RwLock::new(config)),
        observed,
        events,
        commands,
        mqtt: Mutex::new(mqtt),
        staged_at_boot,
    }))
}
