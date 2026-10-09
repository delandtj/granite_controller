//! Log ring (ADR 0001 component 12).
//!
//! One `log::Log` implementation sits in front of the ESP-IDF logger. Every
//! record it accepts is
//!
//!   1. forwarded to [`EspLogger`] unchanged, so the USB console keeps the
//!      ESP-IDF format and the ESP-IDF per-target level filter,
//!   2. formatted once and pushed into a ring that holds the last
//!      [`RING_BYTES`] of lines (the `log` console command and the
//!      Maintenance page read it through [`tail`]),
//!   3. offered to every subscriber whose level threshold it meets, so the
//!      MQTT worker can publish log lines without this module knowing
//!      anything about MQTT.
//!
//! Nothing in here logs: a `log::Log` that logs deadlocks itself.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Mutex, OnceLock};

use esp_idf_svc::log::{EspIdfLogFilter, EspLogger};
use granite_core::config::LogLevel;
use log::{Level, LevelFilter, Log, Metadata, Record};

/// How much formatted text the ring keeps. The ADR asks for 16 KB.
pub const RING_BYTES: usize = 16 * 1024;

/// Hard cap on the number of lines, so a flood of tiny lines cannot make
/// the deque itself the memory problem.
const RING_LINES: usize = 512;

/// Depth of a subscriber channel. A subscriber that falls behind loses
/// records rather than blocking the logging task.
const SUB_DEPTH: usize = 32;

/// One kept log record.
#[derive(Debug, Clone)]
pub struct LogRecord {
    /// Monotonic milliseconds since boot.
    pub ts_ms: u64,
    /// Severity.
    pub level: Level,
    /// `log` target, usually the module path.
    pub target: String,
    /// The formatted message.
    pub message: String,
}

impl LogRecord {
    /// The line as the console and the Maintenance page show it.
    pub fn line(&self) -> String {
        format!(
            "{:>8}.{:03} {} {}: {}",
            self.ts_ms / 1000,
            self.ts_ms % 1000,
            marker(self.level),
            self.target,
            self.message
        )
    }

    /// The line as a JSON object, which is what the MQTT `log` topic wants.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "v": granite_core::msg::PROTOCOL_VERSION,
            "ts": self.ts_ms,
            "level": level_name(self.level),
            "target": self.target,
            "msg": self.message,
        })
        .to_string()
    }
}

const fn marker(level: Level) -> &'static str {
    match level {
        Level::Error => "E",
        Level::Warn => "W",
        Level::Info => "I",
        Level::Debug => "D",
        Level::Trace => "V",
    }
}

const fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    }
}

/// The `sys.log_level` config value as a `log` level.
pub const fn level_of(level: LogLevel) -> Level {
    match level {
        LogLevel::Error => Level::Error,
        LogLevel::Warn => Level::Warn,
        LogLevel::Info => Level::Info,
        LogLevel::Debug => Level::Debug,
        LogLevel::Trace => Level::Trace,
    }
}

struct Ring {
    lines: VecDeque<LogRecord>,
    bytes: usize,
}

impl Ring {
    const fn new() -> Self {
        Ring {
            lines: VecDeque::new(),
            bytes: 0,
        }
    }

    fn push(&mut self, rec: LogRecord) {
        self.bytes += rec.message.len() + rec.target.len() + 24;
        self.lines.push_back(rec);
        while self.bytes > RING_BYTES || self.lines.len() > RING_LINES {
            match self.lines.pop_front() {
                Some(old) => {
                    self.bytes = self
                        .bytes
                        .saturating_sub(old.message.len() + old.target.len() + 24)
                }
                None => break,
            }
        }
    }
}

struct Subscriber {
    min_level: Level,
    tx: SyncSender<LogRecord>,
}

struct State {
    ring: Mutex<Ring>,
    subs: Mutex<Vec<Subscriber>>,
    /// Level at or above which subscribers are fed, as a `Level as u8`.
    publish_level: AtomicU8,
}

static STATE: OnceLock<State> = OnceLock::new();

fn state() -> &'static State {
    STATE.get_or_init(|| State {
        ring: Mutex::new(Ring::new()),
        subs: Mutex::new(Vec::new()),
        publish_level: AtomicU8::new(Level::Warn as u8),
    })
}

/// The ESP-IDF logger this one wraps. Its filter honours
/// `CONFIG_LOG_MAXIMUM_LEVEL` and `esp_log_level_set`.
static ESP: EspLogger = EspLogger::new(EspIdfLogFilter::new());

struct RingLogger;

static RING_LOGGER: RingLogger = RingLogger;

impl Log for RingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        ESP.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if !ESP.enabled(record.metadata()) {
            return;
        }
        ESP.log(record);

        let rec = LogRecord {
            ts_ms: crate::platform::now_ms(),
            level: record.level(),
            target: record.target().to_string(),
            message: record.args().to_string(),
        };

        let st = state();
        if let Ok(mut ring) = st.ring.lock() {
            ring.push(rec.clone());
        }

        // Lower numeric value = higher severity in `log::Level`.
        if rec.level as u8 <= st.publish_level.load(Ordering::Relaxed)
            && let Ok(mut subs) = st.subs.lock()
        {
            subs.retain(|s| {
                if rec.level > s.min_level {
                    return true;
                }
                // try_send so a stalled subscriber never blocks a logger;
                // only a disconnected one is dropped.
                !matches!(
                    s.tx.try_send(rec.clone()),
                    Err(std::sync::mpsc::TrySendError::Disconnected(_))
                )
            });
        }
    }

    fn flush(&self) {
        ESP.flush();
    }
}

/// Install the ring logger. Call this before anything logs; it replaces
/// `EspLogger::initialize_default`.
///
/// Returns `false` if a logger was already installed, which only happens
/// if this is called twice.
pub fn init() -> bool {
    state();
    ESP.filter().initialize();
    let installed = log::set_logger(&RING_LOGGER).is_ok();
    if installed {
        // The ESP-IDF filter set the max level from CONFIG_LOG_MAXIMUM_LEVEL;
        // never go below Info or the heartbeat and the boot lines vanish.
        if log::max_level() < LevelFilter::Info {
            log::set_max_level(LevelFilter::Info);
        }
    }
    installed
}

/// Set the level at or above which records are handed to subscribers
/// (`sys.log_level`).
pub fn set_publish_level(level: LogLevel) {
    state()
        .publish_level
        .store(level_of(level) as u8, Ordering::Relaxed);
}

/// The last `n` lines, oldest first. `n == 0` means "everything kept".
pub fn tail(n: usize) -> Vec<String> {
    let Ok(ring) = state().ring.lock() else {
        return Vec::new();
    };
    let skip = if n == 0 || n >= ring.lines.len() {
        0
    } else {
        ring.lines.len() - n
    };
    ring.lines.iter().skip(skip).map(LogRecord::line).collect()
}

/// The last `n` records, oldest first, for a publisher that wants the
/// fields rather than a formatted line.
pub fn tail_records(n: usize) -> Vec<LogRecord> {
    let Ok(ring) = state().ring.lock() else {
        return Vec::new();
    };
    let skip = if n == 0 || n >= ring.lines.len() {
        0
    } else {
        ring.lines.len() - n
    };
    ring.lines.iter().skip(skip).cloned().collect()
}

/// How many lines the ring holds right now.
pub fn len() -> usize {
    state().ring.lock().map(|r| r.lines.len()).unwrap_or(0)
}

/// Subscribe to records at or above `min_level`. The subscriber is dropped
/// automatically once the receiver is gone.
pub fn subscribe(min_level: Level) -> Receiver<LogRecord> {
    let (tx, rx) = sync_channel(SUB_DEPTH);
    if let Ok(mut subs) = state().subs.lock() {
        subs.push(Subscriber { min_level, tx });
    }
    rx
}
