//! MQTT client: status, state, events, log, cmd/ack (ADR 0001 component 7).
//!
//! One worker thread owns one [`EspMqttClient`] (esp-mqtt) and is the only
//! thing in the firmware that touches the broker. Everything it publishes
//! is built by `granite-core` ([`Status`], [`State`], [`Event`],
//! [`Reply`], [`Topics`]), so the payload contract is host-tested and this
//! module stays glue:
//!
//! | Topic | Dir | Retained | What this module does |
//! |---|---|---|---|
//! | `.../status` | out | yes | on connect and every [`STATUS_PERIOD`] |
//! | `.../state` | out | yes | on change, coalesced [`STATE_COALESCE`], and every `mqtt.t_state_s` |
//! | `.../event` | out | no | every [`Event`] from the event bus |
//! | `.../log` | out | no | every log record at or above `sys.log_level` |
//! | `.../cmd` | in | - | parsed into a [`Command`] and handed to the dispatcher |
//! | `.../ack/<id>` | out | no | when the command *completes*, not when it is queued |
//!
//! The last will is `{"v":1,"online":false}` retained on `.../status`, so
//! a reader that finds the retained status also finds out that the
//! controller is gone.
//!
//! ### Threads and blocking
//!
//! The esp-mqtt event callback runs on esp-mqtt's own task, so it does
//! nothing but copy the event into a bounded channel ([`Wire`]). The
//! worker thread drains that channel, polls the event bus, the log ring
//! and the pending acks, and publishes. Nothing in the hot path blocks on
//! a lock the dispatcher holds, and nothing blocks the logger: when the
//! broker is down the worker keeps draining and drops what it cannot
//! publish.
//!
//! ### Acks
//!
//! An actuator action answers the dispatcher's channel immediately with
//! `accepted`; the ADR wants the ack published when the action *finishes*.
//! So an accepted actuator command becomes a [`Pending`] entry that is
//! resolved by the matching `action_done` / `action_failed` event from the
//! event bus. Commands that finish inside the dispatcher (`config_get`,
//! `rule_ack`, ...) are acked straight from the reply. The `accepted`
//! event the ADR asks for on receipt is the one the dispatcher already
//! emits for every queued action; this module publishes it like any other
//! event rather than inventing a second one.
//!
//! ### Reconnect
//!
//! esp-mqtt reconnects by itself; its own timer is set to the 60 s ceiling
//! and the worker asks for an earlier attempt with
//! `esp_mqtt_client_reconnect` on an exponential ladder of 1, 2, 4 ... 60
//! seconds (ADR component 7). The ladder resets on a successful connect.
//!
//! ### Integration
//!
//! `main.rs` builds the context as a struct literal and keeps the handle
//! alive (dropping it stops the worker):
//!
//! ```ignore
//! if platform.config().mqtt.enabled {
//!     platform::ota::expect_broker();           // probation step 3a
//! }
//! let mqtt = mqtt::start(mqtt::MqttCtx {
//!     config: Box::new({ let p = Arc::clone(&platform); move || p.config() }),
//!     secrets: Box::new({
//!         let p = Arc::clone(&platform);
//!         move || {
//!             p.store
//!                 .lock()
//!                 .map(|mut s| s.load_secrets())
//!                 .unwrap_or_default()
//!         }
//!     }),
//!     observed: Arc::clone(&platform.observed),
//!     events: platform.events.subscribe(),
//!     logs: platform::logring::subscribe(log::Level::Trace),
//!     commands: Box::new({
//!         let tx = commands_tx.clone();
//!         move |cmd, reply| {
//!             tx.send(Submit { cmd, reply: Some(reply) }).is_ok()
//!         }
//!     }),
//!     host: Box::new({
//!         let p = Arc::clone(&platform);
//!         move || mqtt::HostStatus {
//!             ip: p.net.status().ip,
//!             uptime_s: (platform::now_ms() / 1000) as u32,
//!             boot_reason: platform::boot_reason(),
//!             ota_state: platform::ota::ota_state().to_string(),
//!         }
//!     }),
//!     device_id: platform.device_id().to_string(),
//!     fw_version: platform::FW_VERSION.to_string(),
//!     on_connect: Some(Box::new(platform::ota::mark_broker)),
//! })?;
//! ```
//!
//! The worker also keeps [`granite_core::observed::Observed::mqtt_connected`]
//! up to date, which is what the rule engine's `mqtt_connected` source and
//! the `state` payload read. [`connected`] is the same bit as a free
//! function for anything that only wants the flag.

use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, channel, sync_channel};
use std::sync::{Arc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use esp_idf_svc::handle::RawHandle;
use esp_idf_svc::mqtt::client::{
    Details, EspMqttClient, EventPayload, LwtConfiguration, MqttClientConfiguration, QoS,
};
use esp_idf_svc::sys::{esp_mqtt_client_reconnect, esp_timer_get_time};
use esp_idf_svc::tls::X509;
use serde_json::Value;

use granite_core::config::{Config, Secrets};
use granite_core::hal::BootReason;
use granite_core::msg::{Command, Event, EventKind, LWT_PAYLOAD, Reply, State, Status, Topics};
use granite_core::observed::{Observed, Stamped};

use crate::platform::logring::LogRecord;

// ---------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------

/// Worker loop period. Also the longest an incoming command, an event or
/// a log record waits before it is looked at.
pub const TICK: Duration = Duration::from_millis(100);

/// Retained `status` republish period (ADR component 7).
pub const STATUS_PERIOD: Duration = Duration::from_secs(60);

/// A change in the snapshot waits this long before `state` goes out, so a
/// burst of readings becomes one publish (ADR component 7).
pub const STATE_COALESCE: Duration = Duration::from_millis(200);

/// Re-read the configuration this often: the log level, the QoS and
/// `t_state_s` are applied live.
pub const CONFIG_REFRESH: Duration = Duration::from_secs(5);

/// Longest the worker waits for the dispatcher to answer a command.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// Longest the worker waits for an accepted action to finish before it
/// acks the command as timed out. Soft-off plus escalation plus a cycle
/// is the slowest path the actuator has.
pub const ACTION_TIMEOUT: Duration = Duration::from_secs(300);

/// Reconnect ladder, in seconds (ADR component 7: 1 s to 60 s).
const BACKOFF_MIN_S: u64 = 1;
const BACKOFF_MAX_S: u64 = 60;

/// Depth of the channel between the esp-mqtt callback and the worker.
const WIRE_DEPTH: usize = 16;

/// Events and log records handled per loop pass, so a flood cannot starve
/// the rest of the loop.
const DRAIN_PER_TICK: usize = 16;

/// Inbound esp-mqtt buffer. Commands are small; this is the cap on one.
const IN_BUFFER: usize = 2048;

/// Outbound esp-mqtt buffer. A full `state` with eight nodes and eight
/// probes is the largest thing published.
const OUT_BUFFER: usize = 4096;

/// Cap on the QoS 1 outbox, so a long outage cannot eat the heap.
const OUTBOX_LIMIT: usize = 8192;

/// esp-mqtt task stack.
const MQTT_TASK_STACK: usize = 6144;

/// Worker thread stack: it serialises JSON.
const WORKER_STACK: usize = 10240;

/// Broker session state, published for anything that only needs the bit
/// (the OTA probation ladder, the console, the status page).
static CONNECTED: AtomicBool = AtomicBool::new(false);

/// True while the broker session is up.
pub fn connected() -> bool {
    CONNECTED.load(Ordering::Relaxed)
}

/// Monotonic milliseconds, the same clock the core compares against.
fn now_ms() -> u64 {
    (unsafe { esp_timer_get_time() } / 1000) as u64
}

// ---------------------------------------------------------------------
// What the worker is handed
// ---------------------------------------------------------------------

/// A snapshot of the live configuration. Called on every refresh, so it
/// must be cheap and must never block on a lock the dispatcher holds for
/// long.
pub type ConfigFn = Box<dyn Fn() -> Config + Send>;

/// The secrets, read once per connection attempt: broker password and the
/// client certificate pair.
pub type SecretsFn = Box<dyn Fn() -> Secrets + Send>;

/// The parts of the running system the `status` payload needs.
pub type HostFn = Box<dyn Fn() -> HostStatus + Send>;

/// Hands one command to the dispatcher with somewhere to put the reply.
/// Returns false when the dispatcher is gone. This is deliberately a
/// closure: the firmware's command channel carries
/// `crate::platform::Submit`, the HTTP layer has
/// `crate::http::CommandChannel`, and both fit here without this module
/// knowing either.
pub type CommandFn = Box<dyn FnMut(Command, Sender<Reply>) -> bool + Send>;

/// Called on every successful connect. `main.rs` points this at
/// `platform::ota::mark_broker` (probation step 3a); it is idempotent.
pub type ConnectHook = Box<dyn Fn() + Send>;

/// What the `status` payload needs and the MQTT worker cannot see itself.
#[derive(Debug, Clone)]
pub struct HostStatus {
    /// Current IPv4 address, empty when there is none.
    pub ip: String,
    /// Seconds since boot.
    pub uptime_s: u32,
    /// Why this boot happened.
    pub boot_reason: BootReason,
    /// OTA state of the running image, as the firmware page names it.
    pub ota_state: String,
}

impl Default for HostStatus {
    fn default() -> Self {
        HostStatus {
            ip: String::new(),
            uptime_s: 0,
            boot_reason: BootReason::Unknown,
            ota_state: String::new(),
        }
    }
}

/// Everything the MQTT worker reads, writes and sends to.
pub struct MqttCtx {
    /// The live configuration, re-read every [`CONFIG_REFRESH`].
    pub config: ConfigFn,
    /// The secrets namespace.
    pub secrets: SecretsFn,
    /// The sensor snapshot `state` is built from. The worker writes
    /// `mqtt_connected` into it and reads everything else.
    pub observed: Arc<RwLock<Observed>>,
    /// Events from the bus (`platform::EventBus::subscribe`).
    pub events: Receiver<Event>,
    /// Log records from the ring (`platform::logring::subscribe`). The
    /// level filter is applied here, from `sys.log_level`.
    pub logs: Receiver<LogRecord>,
    /// Where an inbound command goes.
    pub commands: CommandFn,
    /// The `status` fields the worker cannot see itself.
    pub host: HostFn,
    /// Device id: the topic leaf and the default client id.
    pub device_id: String,
    /// Firmware version, published in `status`.
    pub fw_version: String,
    /// Called on every successful connect.
    pub on_connect: Option<ConnectHook>,
}

/// The running worker. Dropping it stops the thread, so `main` keeps it.
pub struct MqttHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    enabled: bool,
}

impl MqttHandle {
    /// True while the broker session is up.
    pub fn connected(&self) -> bool {
        connected()
    }

    /// False when `mqtt.enabled` was off at start: no thread was spawned.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    /// Stop the worker and wait for it.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for MqttHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ---------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------

/// Start the MQTT worker.
///
/// Returns a handle that reports `enabled() == false` and holds no thread
/// when `mqtt.enabled` is off; an error only when the configuration says
/// MQTT is on but cannot be used (no host, or a PEM with a NUL byte).
pub fn start(ctx: MqttCtx) -> anyhow::Result<MqttHandle> {
    let cfg = (ctx.config)();
    let stop = Arc::new(AtomicBool::new(false));

    if !cfg.mqtt.enabled {
        log::info!("mqtt is disabled in the configuration; no broker session");
        return Ok(MqttHandle {
            stop,
            join: None,
            enabled: false,
        });
    }
    if cfg.mqtt.host.trim().is_empty() {
        anyhow::bail!("mqtt.enabled is set but mqtt.host is empty");
    }

    let worker_stop = Arc::clone(&stop);
    let join = thread::Builder::new()
        .name("mqtt".into())
        .stack_size(WORKER_STACK)
        .spawn(move || {
            if let Err(e) = run(ctx, &worker_stop) {
                log::error!("mqtt worker stopped: {e:#}");
            }
            CONNECTED.store(false, Ordering::Relaxed);
        })?;

    Ok(MqttHandle {
        stop,
        join: Some(join),
        enabled: true,
    })
}

// ---------------------------------------------------------------------
// The wire side
// ---------------------------------------------------------------------

/// What the esp-mqtt callback hands the worker. Owned, because the event
/// it comes from borrows esp-mqtt's buffer.
enum Wire {
    /// The session is up.
    Connected,
    /// The session is down; esp-mqtt will retry by itself as well.
    Disconnected,
    /// A message on a subscribed topic.
    Message {
        /// Topic, empty when esp-mqtt did not give one.
        topic: String,
        /// Payload.
        data: Vec<u8>,
        /// True when this was the whole message.
        complete: bool,
    },
    /// Transport or protocol error.
    Error,
}

/// The broker URL. `mqtts://` is the default; plain `mqtt://` is allowed
/// and logged, because the ADR's trust model is "the broker's TLS plus
/// credentials" and without TLS there is no first half of that.
fn broker_url(cfg: &Config) -> String {
    let host = cfg.mqtt.host.trim();
    if cfg.mqtt.tls {
        format!("mqtts://{host}:{}", cfg.mqtt.port)
    } else {
        log::warn!(
            "mqtt.tls is off: {host}:{} is plain MQTT, so the broker password and every command cross the network in the clear",
            cfg.mqtt.port
        );
        format!("mqtt://{host}:{}", cfg.mqtt.port)
    }
}

/// Leak a PEM as a NUL-terminated static buffer. esp-mqtt keeps the
/// pointer for the life of the client (which is the life of the
/// firmware), exactly like the HTTPS server does with its certificate.
fn leak_pem(pem: &str) -> Option<X509<'static>> {
    let trimmed = pem.trim();
    if trimmed.is_empty() {
        return None;
    }
    let c = CString::new(trimmed).ok()?;
    let bytes: &'static [u8] = Box::leak(c.into_bytes_with_nul().into_boxed_slice());
    Some(X509::pem_until_nul(bytes))
}

/// The QoS every publish and the `cmd` subscription use.
const fn qos_of(qos: u8) -> QoS {
    match qos {
        0 => QoS::AtMostOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtLeastOnce,
    }
}

// ---------------------------------------------------------------------
// Pending acks
// ---------------------------------------------------------------------

/// One command the worker still owes an ack for.
struct Pending {
    /// Command id, which is also the ack topic leaf.
    id: String,
    /// The dispatcher's reply channel, until it answered.
    reply: Option<Receiver<Reply>>,
    /// Monotonic deadline.
    deadline_ms: u64,
}

impl Pending {
    /// True once the dispatcher said "accepted" and the ack is waiting
    /// for the action to finish.
    const fn awaiting_completion(&self) -> bool {
        self.reply.is_none()
    }
}

// ---------------------------------------------------------------------
// The worker
// ---------------------------------------------------------------------

#[allow(clippy::too_many_lines)]
fn run(ctx: MqttCtx, stop: &AtomicBool) -> anyhow::Result<()> {
    let MqttCtx {
        config,
        secrets,
        observed,
        events,
        logs,
        mut commands,
        host,
        device_id,
        fw_version,
        on_connect,
    } = ctx;

    let mut cfg = config();
    let secrets = secrets();
    let client_id = if cfg.mqtt.client_id.trim().is_empty() {
        device_id.clone()
    } else {
        cfg.mqtt.client_id.trim().to_string()
    };
    let topics = Topics::new(cfg.topic_base(&device_id));
    let url = broker_url(&cfg);
    let mut qos = qos_of(cfg.mqtt.qos);
    let mut log_level = cfg.sys.log_level;
    let mut t_state = Duration::from_secs(u64::from(cfg.mqtt.t_state_s.max(1)));

    let ca = if cfg.mqtt.tls {
        let ca = leak_pem(&cfg.mqtt.ca_pem);
        if ca.is_none() {
            log::error!(
                "mqtt.tls is on but mqtt.ca_pem is empty: the TLS handshake will fail until a CA is configured"
            );
        }
        ca
    } else {
        None
    };
    let client_cert = leak_pem(&secrets.mqtt_client_cert_pem);
    let client_key = leak_pem(&secrets.mqtt_client_key_pem);
    if client_cert.is_some() != client_key.is_some() {
        log::warn!("only half of the MQTT client certificate pair is stored; ignoring both");
    }
    let pair = client_cert.is_some() && client_key.is_some();
    if cfg.mqtt.skip_time_check {
        log::warn!(
            "mqtt.skip_time_check is set, but esp-mqtt has no option for it; the clock starts at the build time instead (ADR component 6)"
        );
    }

    // Owned copies: the configuration struct borrows these, and `cfg` is
    // replaced on every refresh further down.
    let username = cfg.mqtt.username.trim().to_string();
    let password = secrets.mqtt_password.clone();
    let keepalive = Duration::from_secs(u64::from(cfg.mqtt.keepalive_s.max(5)));

    let status_topic = topics.status();
    let lwt = LwtConfiguration {
        topic: &status_topic,
        payload: LWT_PAYLOAD.as_bytes(),
        qos,
        retain: true,
    };

    let conf = MqttClientConfiguration {
        client_id: Some(&client_id),
        keep_alive_interval: Some(keepalive),
        // esp-mqtt's own retry is the ceiling of the ladder below.
        reconnect_timeout: Some(Duration::from_secs(BACKOFF_MAX_S)),
        network_timeout: Duration::from_secs(10),
        lwt: Some(lwt),
        // The ADR wants a session the broker keeps: clean session off.
        disable_clean_session: true,
        task_stack: MQTT_TASK_STACK,
        buffer_size: IN_BUFFER,
        out_buffer_size: OUT_BUFFER,
        outbox_limit: Some(OUTBOX_LIMIT),
        username: Some(username.as_str()).filter(|u| !u.is_empty()),
        password: Some(password.as_str()).filter(|p| !p.is_empty()),
        server_certificate: ca,
        client_certificate: if pair { client_cert } else { None },
        private_key: if pair { client_key } else { None },
        ..Default::default()
    };

    let (wire_tx, wire_rx) = sync_channel::<Wire>(WIRE_DEPTH);
    let callback_tx = wire_tx.clone();
    let mut client = EspMqttClient::new_cb(&url, &conf, move |event| {
        forward(&callback_tx, event.payload());
    })?;
    log::info!(
        "mqtt worker up: {url} as {client_id}, topics under {}, qos {}",
        topics.base(),
        cfg.mqtt.qos
    );

    let cmd_topic = topics.cmd();
    let mut pending: Vec<Pending> = Vec::new();
    let mut session = false;
    let mut backoff_s = BACKOFF_MIN_S;
    let mut retry_at: Option<u64> = None;
    let mut next_status = 0u64;
    let mut next_state = 0u64;
    let mut next_config = now_ms() + CONFIG_REFRESH.as_millis() as u64;
    let mut dirty_since: Option<u64> = None;
    let mut last_state: Option<State> = None;

    while !stop.load(Ordering::Relaxed) {
        // 1. The wire. recv_timeout is the loop's only sleep, so a
        //    connect or a command is acted on as soon as it lands.
        match wire_rx.recv_timeout(TICK) {
            Ok(first) => {
                let mut wire = Some(first);
                while let Some(event) = wire {
                    match event {
                        Wire::Connected => {
                            session = true;
                            backoff_s = BACKOFF_MIN_S;
                            retry_at = None;
                            CONNECTED.store(true, Ordering::Relaxed);
                            note_connected(&observed, true);
                            if let Err(e) = client.subscribe(&cmd_topic, qos) {
                                log::error!("mqtt: cannot subscribe to {cmd_topic}: {e}");
                            }
                            log::info!("mqtt: connected to {url}");
                            if let Some(hook) = on_connect.as_ref() {
                                hook();
                            }
                            // Retained status and state, straight away.
                            next_status = 0;
                            next_state = 0;
                            last_state = None;
                        }
                        Wire::Disconnected | Wire::Error => {
                            if session {
                                log::warn!("mqtt: disconnected from {url}");
                            }
                            session = false;
                            CONNECTED.store(false, Ordering::Relaxed);
                            note_connected(&observed, false);
                            if retry_at.is_none() {
                                retry_at = Some(now_ms() + backoff_s * 1000);
                            }
                        }
                        Wire::Message {
                            topic,
                            data,
                            complete,
                        } => {
                            if !complete {
                                log::warn!(
                                    "mqtt: a {} byte message on {topic} arrived in chunks and was dropped; commands must fit in {IN_BUFFER} bytes",
                                    data.len()
                                );
                            } else if topic == cmd_topic || topic.is_empty() {
                                on_command(
                                    &data,
                                    &mut client,
                                    &topics,
                                    qos,
                                    &mut commands,
                                    &mut pending,
                                );
                            } else {
                                log::debug!("mqtt: ignoring a message on {topic}");
                            }
                        }
                    }
                    wire = wire_rx.try_recv().ok();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("the esp-mqtt callback channel is gone");
            }
        }

        let now = now_ms();

        // 2. Events: every one goes out, and one may finish a pending ack.
        for _ in 0..DRAIN_PER_TICK {
            match events.try_recv() {
                Ok(event) => {
                    if session {
                        publish_json(&mut client, &topics.event(), qos, false, &event);
                    }
                    resolve_from_event(&event, &mut client, &topics, qos, &mut pending, session);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    anyhow::bail!("the event bus dropped the mqtt subscriber");
                }
            }
        }

        // 3. Log records at or above sys.log_level.
        for _ in 0..DRAIN_PER_TICK {
            match logs.try_recv() {
                Ok(rec) => {
                    if session && rec.level <= crate::platform::logring::level_of(log_level) {
                        publish(
                            &mut client,
                            &topics.log(),
                            qos,
                            false,
                            rec.to_json().as_bytes(),
                        );
                    }
                }
                Err(TryRecvError::Empty) => break,
                // The ring drops a subscriber only when the receiver is
                // gone, which cannot happen while this loop owns it.
                Err(TryRecvError::Disconnected) => break,
            }
        }

        // 4. Acks the dispatcher owes an answer for.
        resolve_pending(&mut client, &topics, qos, &mut pending, session, now);

        // 5. Retained status.
        if session && now >= next_status {
            next_status = now + STATUS_PERIOD.as_millis() as u64;
            let payload = status_json(&host(), &fw_version);
            publish(&mut client, &status_topic, qos, true, &payload);
        }

        // 6. Retained state: on change, coalesced, and every t_state.
        if session {
            let state = build_state(&observed, &cfg, now);
            let changed = last_state
                .as_ref()
                .is_none_or(|last| !same_state(last, &state));
            if changed && dirty_since.is_none() {
                dirty_since = Some(now);
            }
            let coalesced = dirty_since.is_some_and(|since| {
                now.saturating_sub(since) >= STATE_COALESCE.as_millis() as u64
            });
            if coalesced || now >= next_state {
                dirty_since = None;
                next_state = now + t_state.as_millis() as u64;
                publish_json(&mut client, &topics.state(), qos, true, &state);
                last_state = Some(state);
            }
        }

        // 7. The reconnect ladder. esp-mqtt retries on its own 60 s timer;
        //    this asks for an earlier attempt and doubles the wait.
        if !session
            && let Some(at) = retry_at
            && now >= at
        {
            backoff_s = (backoff_s * 2).min(BACKOFF_MAX_S);
            retry_at = Some(now + backoff_s * 1000);
            log::info!("mqtt: reconnecting, next attempt in {backoff_s} s if this one fails");
            // Safe: the handle is alive for as long as `client` is, and
            // this is not the esp-mqtt event task.
            let err = unsafe { esp_mqtt_client_reconnect(client.handle()) };
            if err != esp_idf_svc::sys::ESP_OK {
                log::debug!("mqtt: esp_mqtt_client_reconnect returned {err}");
            }
        }

        // 8. The parts of the configuration that can change under us.
        if now >= next_config {
            next_config = now + CONFIG_REFRESH.as_millis() as u64;
            let next = config();
            if connection_differs(&cfg, &next) {
                log::warn!(
                    "the mqtt section changed; the new broker settings are applied on the next restart (commit-confirm reboots for them)"
                );
            }
            qos = qos_of(next.mqtt.qos);
            log_level = next.sys.log_level;
            t_state = Duration::from_secs(u64::from(next.mqtt.t_state_s.max(1)));
            cfg = next;
        }
    }

    // A clean stop is not a crash: say so on the retained topic rather
    // than leaving the last will to do it.
    if session {
        publish(
            &mut client,
            &status_topic,
            qos,
            true,
            LWT_PAYLOAD.as_bytes(),
        );
    }
    CONNECTED.store(false, Ordering::Relaxed);
    note_connected(&observed, false);
    log::info!("mqtt worker stopping");
    Ok(())
}

/// The esp-mqtt callback: copy and hand over, never block. A full channel
/// means the worker is behind, and a dropped wire event is recoverable
/// (the next tick republishes status and state anyway).
fn forward(tx: &SyncSender<Wire>, payload: EventPayload<'_, esp_idf_svc::sys::EspError>) {
    let wire = match payload {
        EventPayload::Connected(_) => Wire::Connected,
        EventPayload::Disconnected => Wire::Disconnected,
        EventPayload::Error(_) => Wire::Error,
        EventPayload::Received {
            topic,
            data,
            details,
            ..
        } => Wire::Message {
            topic: topic.unwrap_or_default().to_string(),
            data: data.to_vec(),
            complete: matches!(details, Details::Complete),
        },
        // BeforeConnect, Subscribed, Unsubscribed, Published, Deleted:
        // bookkeeping the worker does not need.
        _ => return,
    };
    if tx.try_send(wire).is_err() {
        // No log::warn here: this runs on the esp-mqtt task and the log
        // ring feeds this very worker.
    }
}

/// Write the broker session into the shared snapshot, which is what the
/// rule engine's `mqtt_connected` source and the `state` payload read.
fn note_connected(observed: &Arc<RwLock<Observed>>, up: bool) {
    if let Ok(mut o) = observed.write() {
        o.mqtt_connected = Stamped::new(up, now_ms());
    }
}

/// The retained `status` payload: the ADR's shape plus the OTA state of
/// the running image.
fn status_json(host: &HostStatus, fw: &str) -> Vec<u8> {
    let status = Status::online(fw, &host.ip, host.uptime_s, host.boot_reason);
    match serde_json::to_value(&status) {
        Ok(Value::Object(mut map)) => {
            map.insert("ota".into(), Value::String(host.ota_state.clone()));
            serde_json::to_vec(&Value::Object(map)).unwrap_or_else(|_| LWT_PAYLOAD.into())
        }
        _ => LWT_PAYLOAD.into(),
    }
}

/// The `state` payload from the shared snapshot plus the node names.
fn build_state(observed: &Arc<RwLock<Observed>>, cfg: &Config, now: u64) -> State {
    let snapshot = observed
        .read()
        .map(|o| o.clone())
        .unwrap_or_else(|_| Observed::new());
    State::from_observed(&snapshot, &cfg.nodes, now)
}

/// Compare two `state` payloads while ignoring the fields that move on
/// their own: a timestamp and an uptime are not a change in state.
fn same_state(a: &State, b: &State) -> bool {
    let mut left = a.clone();
    let mut right = b.clone();
    left.ts = 0;
    right.ts = 0;
    left.uptime_s = 0;
    right.uptime_s = 0;
    left == right
}

/// True when something that only takes effect at connect time changed.
fn connection_differs(a: &Config, b: &Config) -> bool {
    a.mqtt.enabled != b.mqtt.enabled
        || a.mqtt.host != b.mqtt.host
        || a.mqtt.port != b.mqtt.port
        || a.mqtt.tls != b.mqtt.tls
        || a.mqtt.ca_pem != b.mqtt.ca_pem
        || a.mqtt.username != b.mqtt.username
        || a.mqtt.client_id != b.mqtt.client_id
        || a.mqtt.site != b.mqtt.site
        || a.mqtt.topic_root != b.mqtt.topic_root
        || a.mqtt.keepalive_s != b.mqtt.keepalive_s
}

// ---------------------------------------------------------------------
// Publishing
// ---------------------------------------------------------------------

fn publish(
    client: &mut EspMqttClient<'static>,
    topic: &str,
    qos: QoS,
    retain: bool,
    payload: &[u8],
) {
    if let Err(e) = client.publish(topic, qos, retain, payload) {
        log::warn!("mqtt: publish to {topic} failed: {e}");
    }
}

/// Serialise straight into a `Vec` and publish it; nothing is kept.
fn publish_json<T: serde::Serialize>(
    client: &mut EspMqttClient<'static>,
    topic: &str,
    qos: QoS,
    retain: bool,
    value: &T,
) {
    match serde_json::to_vec(value) {
        Ok(payload) => publish(client, topic, qos, retain, &payload),
        Err(e) => log::warn!("mqtt: cannot serialise the payload for {topic}: {e}"),
    }
}

fn publish_ack(
    client: &mut EspMqttClient<'static>,
    topics: &Topics,
    qos: QoS,
    reply: &Reply,
    session: bool,
) {
    if session {
        publish_json(client, &topics.ack(&reply.id), qos, false, reply);
    } else {
        log::warn!(
            "mqtt: no session, dropping the ack for {}: {:?}",
            reply.id,
            reply.result.as_deref().or(reply.error.as_deref())
        );
    }
}

// ---------------------------------------------------------------------
// Inbound commands
// ---------------------------------------------------------------------

/// One message on `.../cmd`.
fn on_command(
    data: &[u8],
    client: &mut EspMqttClient<'static>,
    topics: &Topics,
    qos: QoS,
    commands: &mut CommandFn,
    pending: &mut Vec<Pending>,
) {
    let text = match core::str::from_utf8(data) {
        Ok(t) => t,
        Err(_) => {
            log::warn!("mqtt: a command on {} is not UTF-8", topics.cmd());
            return;
        }
    };
    let cmd = match Command::from_json(text) {
        Ok(cmd) => cmd,
        Err(e) => {
            // An id is the only thing that makes a refusal addressable.
            match id_of(text) {
                Some(id) => {
                    let reply = Reply::err(id, format!("bad command: {e}"));
                    publish_ack(client, topics, qos, &reply, true);
                }
                None => log::warn!("mqtt: dropping a command with no usable id: {e}"),
            }
            return;
        }
    };
    if cmd.id.trim().is_empty() {
        log::warn!("mqtt: dropping a command with an empty id");
        return;
    }

    let (reply_tx, reply_rx) = channel();
    if !commands(cmd.clone(), reply_tx) {
        let reply = Reply::err(&cmd.id, "the controller task is not accepting commands");
        publish_ack(client, topics, qos, &reply, true);
        return;
    }
    log::info!("mqtt: command {} {} on {}", cmd.id, cmd.action, cmd.target);
    pending.push(Pending {
        id: cmd.id.clone(),
        reply: Some(reply_rx),
        deadline_ms: now_ms() + REPLY_TIMEOUT.as_millis() as u64,
    });
}

/// The `id` of a payload that did not parse as a command.
fn id_of(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    let id = value.get("id")?.as_str()?.trim();
    if id.is_empty() || id.len() > 64 {
        return None;
    }
    Some(id.to_string())
}

/// Replies that arrived, and deadlines that passed.
fn resolve_pending(
    client: &mut EspMqttClient<'static>,
    topics: &Topics,
    qos: QoS,
    pending: &mut Vec<Pending>,
    session: bool,
    now: u64,
) {
    let mut done: Vec<Reply> = Vec::new();
    pending.retain_mut(|p| {
        if let Some(rx) = p.reply.as_ref() {
            match rx.try_recv() {
                Ok(reply) => {
                    // "accepted" means the actuator queued it; the ack
                    // the ADR wants goes out when the action finishes.
                    let queued = reply.ok && reply.result.as_deref() == Some("accepted");
                    if queued {
                        p.reply = None;
                        p.deadline_ms = now + ACTION_TIMEOUT.as_millis() as u64;
                        return true;
                    }
                    done.push(reply);
                    return false;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    done.push(Reply::err(&p.id, "the controller task dropped the command"));
                    return false;
                }
            }
        }
        if now >= p.deadline_ms {
            let why = if p.awaiting_completion() {
                "the action did not finish in time"
            } else {
                "the controller task did not answer in time"
            };
            done.push(Reply::err(&p.id, why));
            return false;
        }
        true
    });
    for reply in &done {
        publish_ack(client, topics, qos, reply, session);
    }
}

/// An `action_done` or `action_failed` event is the completion an accepted
/// actuator command was waiting for.
fn resolve_from_event(
    event: &Event,
    client: &mut EspMqttClient<'static>,
    topics: &Topics,
    qos: QoS,
    pending: &mut Vec<Pending>,
    session: bool,
) {
    let reply = match &event.kind {
        EventKind::ActionDone { id, result, .. } => {
            // Escalation is a step inside `off`, not the end of it.
            if result == "escalated" {
                return;
            }
            Reply::ok(id, result.clone())
        }
        EventKind::ActionFailed { id, error, .. } => Reply::err(id, error.clone()),
        _ => return,
    };
    let before = pending.len();
    pending.retain(|p| !(p.awaiting_completion() && p.id == reply.id));
    if pending.len() != before {
        publish_ack(client, topics, qos, &reply, session);
    }
}
