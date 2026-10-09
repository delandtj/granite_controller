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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::spi::SPI2;
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
use granite_core::hal::{BootReason, Clock, HalError, HalResult, NodeSwitches, Reboot, Switch};
use granite_core::msg::Event;
use granite_core::observed::Observed;
use granite_core::NodeId;

use identity::Identity;
use net::{EthPins, Net};
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

/// Register the callback that opens every relay.
///
/// The hardware layer passes its "release all" here during `hw::init`. The
/// platform layer never reaches into `hw` itself, so a build without the
/// hardware layer still links; the cost of a missing hook is only that a
/// reboot does not actively open the relays, which the GPIO10 pull-down
/// does anyway.
pub fn set_release_hook<F>(hook: F)
where
    F: Fn() + Send + Sync + 'static,
{
    if RELEASE_HOOK.set(Box::new(hook)).is_err() {
        log::warn!("release hook was already registered; keeping the first one");
    }
}

/// Open every relay now, if a hook is registered.
pub fn release_outputs() {
    if let Some(hook) = RELEASE_HOOK.get() {
        hook();
    }
}

/// The one way the firmware restarts itself.
///
/// Releases every relay first, lets the last log line reach the console,
/// then calls `esp_restart`. Never returns.
pub fn planned_reboot(reason: &str) -> ! {
    if !REBOOT_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        log::warn!("planned reboot ({reason})");
    }
    release_outputs();
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
#[derive(Default)]
pub struct EventBus {
    subs: Mutex<Vec<Sender<Event>>>,
}

impl EventBus {
    /// Empty bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a subscriber.
    pub fn subscribe(&self) -> std::sync::mpsc::Receiver<Event> {
        let (tx, rx) = std::sync::mpsc::channel();
        if let Ok(mut subs) = self.subs.lock() {
            subs.push(tx);
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
        if let Ok(mut subs) = self.subs.lock() {
            subs.retain(|tx| tx.send(event.clone()).is_ok());
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
// A NodeSwitches that does nothing
// ---------------------------------------------------------------------------

/// Stand-in for the relay driver until the hardware layer owns it.
///
/// **This is a stub.** Every call is logged and refused with
/// [`HalError::NotPresent`], so a command reaches the actuator, fails
/// honestly and shows up as `action_failed` rather than silently
/// pretending to press a button. `main.rs` replaces it with the hardware
/// layer's `NodeSwitches` at the marked integration point.
#[derive(Debug, Default)]
pub struct NullSwitches;

impl NodeSwitches for NullSwitches {
    fn assert(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        log::warn!("no relay driver: refusing to press {sw} on node {node}");
        Err(HalError::NotPresent)
    }

    fn release(&mut self, _node: NodeId, _sw: Switch) -> HalResult<()> {
        Ok(())
    }

    fn release_all(&mut self) -> HalResult<()> {
        Ok(())
    }
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
    /// iterations, a fresh per-device salt, and `sec.admin_password_set`
    /// flipped so the HTTP layer leaves first-setup mode.
    pub fn set_admin_password(&self, password: &str) -> Result<(), String> {
        let salt = identity::random_bytes(PBKDF2_SALT);
        let hash = pbkdf2(password.as_bytes(), &salt).ok_or("pbkdf2 failed")?;
        let mut store = self.store.lock().map_err(|_| "store is locked")?;
        let mut secrets = store.load_secrets();
        secrets.admin_salt = identity::hex(&salt);
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
    spi: SPI2<'static>,
    eth_pins: EthPins,
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
    ota::start(config.sys.t_validate_s, Some(events.sender()));

    let net = net::init(
        spi,
        eth_pins,
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
