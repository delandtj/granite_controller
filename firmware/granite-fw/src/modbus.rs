//! Modbus TCP server (ADR 0001 component 9).
//!
//! This module is sockets, threads and timeouts. Every frame, every
//! register and every exception comes from
//! [`granite_core::modbus_server`], which is host-tested
//! (`granite-core/tests/modbus_server.rs`); what is here reads a frame off
//! a `TcpStream`, hands it over, and writes the answer back.
//!
//! Modbus has no authentication, so the guard is the peer address:
//!
//! - The server does not start unless `sec.modbus.enabled` is true **and**
//!   `sec.modbus.allow` holds at least one usable IPv4 address or CIDR.
//!   Both cases are logged with the reason.
//! - A connection's peer is checked against the allow-list before a
//!   single byte is read, and a rejected peer is closed. Those log lines
//!   are rate-limited ([`REJECT_LOG_INTERVAL`]) with a suppressed count,
//!   because an unwanted scanner must not fill the log ring.
//! - At most `sec.modbus.max_conn` connections are served at once
//!   (default 4); the next one is closed immediately. Each gets its own
//!   thread with a [`CONN_STACK`]-byte stack, and goes away after
//!   [`IDLE_TIMEOUT`] without a frame.
//!
//! Reads are served from a [`RegisterImage`] rebuilt from the shared
//! [`Observed`] per request, which is a handful of array writes. Writes
//! become [`Command`]s and travel the same channel the HTTP and MQTT
//! transports use, so the dispatcher is still the only thing that talks to
//! the actuator; holding register 9 reports the ack of the last one.
//!
//! ### Entry point
//!
//! `main.rs` starts this once, unconditionally, after the dispatcher
//! thread exists (the handle is kept so the HTTP layer can reconfigure
//! the server without a reboot):
//!
//! ```ignore
//! let modbus = modbus::start(modbus::ModbusCtx {
//!     observed: Arc::clone(&platform.observed),
//!     commands: Arc::clone(&platform.commands),
//!     config: platform.config().sec.modbus.clone(),
//!     fw: modbus::fw_version(),
//!     faults: Arc::clone(&faults),
//! })?;
//! ```
//!
//! `start` returns a handle even when the configuration says off: it is
//! the supervisor thread that is always running, not the listener. When
//! `sec.modbus` changes (the setup page's security section, a
//! `config_set` over MQTT), the owner of the handle calls
//! [`ModbusHandle::set_config`] with the new section and the supervisor
//! binds, rebinds or closes the listener within [`POLL`]. Nothing here
//! needs a reboot, and nothing here can change node state on its own.

use std::io::{ErrorKind as IoErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use granite_core::config::ModbusCfg;
use granite_core::modbus_map::{FwVersion, ResultCode};
use granite_core::modbus_server::{
    AllowList, MAX_FRAME, MBAP_HEADER, RegisterImage, handle_frame, mbap_payload_len,
};
use granite_core::msg::{Command, Reply};
use granite_core::observed::Observed;

use crate::platform::CommandChannel;

/// How long a connection may sit idle before the server closes it.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Write timeout. A peer that cannot take 250 bytes in this time is gone.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Supervisor and accept poll interval. Also the worst-case delay of a
/// [`ModbusHandle::set_config`], because every wait below is sliced into
/// intervals of this length.
pub const POLL: Duration = Duration::from_millis(200);

/// How often the supervisor re-reads the configuration while the server
/// is off. A [`ModbusHandle::set_config`] does not wait for this: it
/// bumps the generation the wait checks every [`POLL`].
pub const OFF_POLL: Duration = Duration::from_secs(1);

/// How long to wait before binding again after a bind failure (the
/// network may simply not be up yet).
pub const REBIND_DELAY: Duration = Duration::from_secs(5);

/// Stack of a connection thread. One [`RegisterImage`], one 256-byte
/// frame buffer and a small response vector.
pub const CONN_STACK: usize = 5120;

/// Stack of the supervisor thread.
pub const SUPERVISOR_STACK: usize = 4096;

/// Command ids one frame may consume. A function 16 write covers at most
/// 123 registers.
const SEQ_SPAN: u32 = 256;

/// At most one "rejected" log line per interval, with the suppressed
/// count attached to the next one.
pub const REJECT_LOG_INTERVAL: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------
// What the Modbus task is handed
// ---------------------------------------------------------------------

/// Everything the server needs. A struct literal on purpose: a
/// constructor with five arguments would only hide which is which.
pub struct ModbusCtx {
    /// The sensor snapshot, written by the sense task. Node states are in
    /// here too, so this is the whole read side.
    pub observed: Arc<RwLock<Observed>>,
    /// The command channel into the dispatcher thread: the same
    /// [`CommandChannel`] the HTTP API and MQTT push through, so the
    /// dispatcher stays the only thing that touches the actuator.
    pub commands: Arc<CommandChannel>,
    /// The `sec.modbus` section as it stands at start.
    pub config: ModbusCfg,
    /// Firmware version for input register 12.
    pub fw: FwVersion,
    /// Fault bitmap for input register 13
    /// ([`granite_core::modbus_map::fault_bits`]). Shared, so whoever
    /// notices a fault sets the bit and every reader sees it; it stays 0
    /// when nobody writes it.
    pub faults: Arc<AtomicU16>,
}

/// The firmware version as the register map reports it, from the crate
/// version. A version part that is not a number reads as 0.
pub fn fw_version() -> FwVersion {
    let mut parts = env!("CARGO_PKG_VERSION").split('.');
    FwVersion {
        major: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
        minor: parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
    }
}

/// The running server. Dropping it does not stop the supervisor; call
/// [`ModbusHandle::stop`] or disable the server through
/// [`ModbusHandle::set_config`].
#[derive(Clone)]
pub struct ModbusHandle {
    shared: Arc<Shared>,
}

impl ModbusHandle {
    /// Apply a new `sec.modbus` section.
    ///
    /// Takes effect within [`POLL`]: the supervisor closes the listener
    /// when the server is switched off or loses its allow-list, and binds
    /// (or rebinds, on a port change) when it is switched on. Live
    /// connections of a removed peer are not torn down; the allow-list is
    /// checked at accept time.
    pub fn set_config(&self, cfg: &ModbusCfg) {
        let next = Desired::of(cfg);
        let mut guard = match self.shared.desired.lock() {
            Ok(g) => g,
            Err(_) => {
                log::error!("modbus: the config lock is poisoned; keeping the old settings");
                return;
            }
        };
        if *guard == next {
            return;
        }
        *guard = next;
        drop(guard);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        log::info!("modbus: configuration reloaded");
    }

    /// True while a listener is bound.
    pub fn is_listening(&self) -> bool {
        self.shared.listening.load(Ordering::Relaxed)
    }

    /// Open connections right now.
    pub fn connections(&self) -> usize {
        self.shared.conns.load(Ordering::Relaxed)
    }

    /// What holding register 9 reports.
    pub fn last_result(&self) -> ResultCode {
        self.shared.last_result()
    }

    /// Stop the supervisor and let every connection thread finish its
    /// current frame and exit.
    pub fn stop(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// Start the server.
///
/// Never fails on a configuration that says "off": that is logged, not an
/// error. The only error is a thread that cannot be spawned.
pub fn start(ctx: ModbusCtx) -> anyhow::Result<ModbusHandle> {
    let shared = Arc::new(Shared {
        observed: ctx.observed,
        commands: ctx.commands,
        faults: ctx.faults,
        fw: ctx.fw,
        desired: Mutex::new(Desired::of(&ctx.config)),
        generation: AtomicU32::new(0),
        listening: AtomicBool::new(false),
        conns: AtomicUsize::new(0),
        stop: AtomicBool::new(false),
        last_result: AtomicU16::new(ResultCode::Idle.as_u16()),
        rejects: AtomicU32::new(0),
        reject_log: Mutex::new(None),
        seq: AtomicU32::new(0),
    });

    let worker = Arc::clone(&shared);
    thread::Builder::new()
        .name("modbus".into())
        .stack_size(SUPERVISOR_STACK)
        .spawn(move || supervisor(worker))?;

    Ok(ModbusHandle { shared })
}

// ---------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------

/// The configuration the supervisor is trying to reach.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Desired {
    enabled: bool,
    port: u16,
    unit_id: u8,
    max_conn: usize,
    allow: Vec<String>,
}

impl Desired {
    fn of(cfg: &ModbusCfg) -> Self {
        Desired {
            enabled: cfg.enabled,
            port: cfg.port,
            unit_id: cfg.unit_id,
            max_conn: cfg.max_conn as usize,
            allow: cfg.allow.clone(),
        }
    }
}

struct Shared {
    observed: Arc<RwLock<Observed>>,
    commands: Arc<CommandChannel>,
    faults: Arc<AtomicU16>,
    fw: FwVersion,
    desired: Mutex<Desired>,
    generation: AtomicU32,
    listening: AtomicBool,
    conns: AtomicUsize,
    stop: AtomicBool,
    last_result: AtomicU16,
    rejects: AtomicU32,
    reject_log: Mutex<Option<Instant>>,
    seq: AtomicU32,
}

impl Shared {
    fn desired(&self) -> Option<Desired> {
        self.desired.lock().ok().map(|d| d.clone())
    }

    fn last_result(&self) -> ResultCode {
        match self.last_result.load(Ordering::Relaxed) {
            1 => ResultCode::Ok,
            2 => ResultCode::Accepted,
            3 => ResultCode::Refused,
            4 => ResultCode::Failed,
            5 => ResultCode::ShutdownPending,
            _ => ResultCode::Idle,
        }
    }

    fn set_last_result(&self, code: ResultCode) {
        self.last_result.store(code.as_u16(), Ordering::Relaxed);
    }

    /// The image a read is served from: one snapshot, rendered through
    /// the map.
    fn image(&self) -> RegisterImage {
        let faults = self.faults.load(Ordering::Relaxed);
        let last = self.last_result();
        match self.observed.read() {
            Ok(o) => RegisterImage::build(&o, self.fw, faults, last),
            Err(_) => {
                // A poisoned snapshot lock must not make the server lie
                // about node state: every node reads as unknown, every
                // temperature as missing.
                log::error!("modbus: the snapshot lock is poisoned; serving an empty snapshot");
                RegisterImage::build(&Observed::new(), self.fw, faults, last)
            }
        }
    }

    /// Send one command to the dispatcher and wait for its ack.
    ///
    /// The wait is bounded by [`crate::http::COMMAND_TIMEOUT`], and a
    /// queued action is acked with `accepted` immediately, so a slow
    /// actuator cannot hold a Modbus connection.
    fn submit(&self, cmd: &Command) -> Reply {
        self.commands.call(cmd)
    }

    /// Log a refused connection at most once per
    /// [`REJECT_LOG_INTERVAL`], with the suppressed count.
    fn log_reject(&self, peer: &SocketAddr, why: &str) {
        let suppressed = self.rejects.fetch_add(1, Ordering::Relaxed);
        let Ok(mut last) = self.reject_log.lock() else {
            return;
        };
        let now = Instant::now();
        let quiet = last.is_some_and(|t| now.duration_since(t) < REJECT_LOG_INTERVAL);
        if quiet {
            return;
        }
        *last = Some(now);
        self.rejects.store(0, Ordering::Relaxed);
        if suppressed > 0 {
            log::warn!("modbus: refused {peer} ({why}), {suppressed} more since the last line");
        } else {
            log::warn!("modbus: refused {peer} ({why})");
        }
    }
}

// ---------------------------------------------------------------------
// The supervisor
// ---------------------------------------------------------------------

fn supervisor(shared: Arc<Shared>) {
    let mut announced_off = false;
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            log::info!("modbus: stopped");
            return;
        }
        let generation = shared.generation.load(Ordering::SeqCst);
        let Some(desired) = shared.desired() else {
            log::error!("modbus: the config lock is poisoned; the server stays off");
            return;
        };

        let allow = AllowList::parse(&desired.allow);
        for bad in &allow.rejected {
            log::warn!("modbus: sec.modbus.allow entry \"{bad}\" is not an address or a CIDR");
        }

        let reason = if !desired.enabled {
            Some("sec.modbus.enabled is false")
        } else if desired.allow.is_empty() {
            Some("sec.modbus.allow is empty and Modbus has no authentication")
        } else if allow.is_empty() {
            Some("no entry in sec.modbus.allow is a usable address or CIDR")
        } else {
            None
        };
        if let Some(reason) = reason {
            if !announced_off {
                log::info!("modbus: off ({reason})");
                announced_off = true;
            }
            wait_for_change(&shared, generation, OFF_POLL);
            continue;
        }
        announced_off = false;

        match TcpListener::bind(("0.0.0.0", desired.port)) {
            Ok(listener) => {
                log::info!(
                    "modbus: listening on 0.0.0.0:{}, unit {}, {} allowed network(s), \
                     {} connection(s) max",
                    desired.port,
                    desired.unit_id,
                    allow.len(),
                    desired.max_conn
                );
                serve(&shared, &listener, &desired, &allow, generation);
                shared.listening.store(false, Ordering::Relaxed);
                log::info!("modbus: listener on port {} closed", desired.port);
            }
            Err(e) => {
                log::warn!(
                    "modbus: cannot bind port {}: {e}; retrying in {} s",
                    desired.port,
                    REBIND_DELAY.as_secs()
                );
                wait_for_change(&shared, generation, REBIND_DELAY);
            }
        }
    }
}

/// Sleep in [`POLL`] slices until `limit` has passed, the configuration
/// changed, or the server was stopped.
fn wait_for_change(shared: &Shared, generation: u32, limit: Duration) {
    let start = Instant::now();
    while start.elapsed() < limit {
        if shared.stop.load(Ordering::SeqCst)
            || shared.generation.load(Ordering::SeqCst) != generation
        {
            return;
        }
        thread::sleep(POLL.min(limit));
    }
}

/// Accept loop. Returns when the configuration changed or the server was
/// stopped, so the caller can rebind or go away.
fn serve(
    shared: &Arc<Shared>,
    listener: &TcpListener,
    desired: &Desired,
    allow: &AllowList,
    generation: u32,
) {
    if let Err(e) = listener.set_nonblocking(true) {
        log::error!("modbus: the listener cannot be polled ({e}); not serving");
        return;
    }
    shared.listening.store(true, Ordering::Relaxed);

    loop {
        if shared.stop.load(Ordering::SeqCst)
            || shared.generation.load(Ordering::SeqCst) != generation
        {
            return;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                accept_one(shared, stream, peer, desired, allow);
            }
            Err(e) if e.kind() == IoErrorKind::WouldBlock => thread::sleep(POLL),
            Err(e) if e.kind() == IoErrorKind::Interrupted => {}
            Err(e) => {
                log::warn!("modbus: accept failed ({e}); rebinding");
                return;
            }
        }
    }
}

/// Guard one accepted connection and give it a thread if it passes.
fn accept_one(
    shared: &Arc<Shared>,
    stream: TcpStream,
    peer: SocketAddr,
    desired: &Desired,
    allow: &AllowList,
) {
    let Some(ip) = ipv4_of(&peer) else {
        shared.log_reject(&peer, "not an IPv4 peer");
        close(&stream);
        return;
    };
    // The allow-list is checked before a single byte is read.
    if !allow.allows(ip) {
        shared.log_reject(&peer, "not on sec.modbus.allow");
        close(&stream);
        return;
    }
    if shared.conns.load(Ordering::SeqCst) >= desired.max_conn.max(1) {
        shared.log_reject(&peer, "too many connections");
        close(&stream);
        return;
    }

    shared.conns.fetch_add(1, Ordering::SeqCst);
    let worker = Arc::clone(shared);
    let unit_id = desired.unit_id;
    let spawned = thread::Builder::new()
        .name("modbus-conn".into())
        .stack_size(CONN_STACK)
        .spawn(move || {
            log::debug!("modbus: {peer} connected");
            connection(&worker, stream, peer, unit_id);
            worker.conns.fetch_sub(1, Ordering::SeqCst);
            log::debug!("modbus: {peer} closed");
        });
    if let Err(e) = spawned {
        shared.conns.fetch_sub(1, Ordering::SeqCst);
        log::error!("modbus: no thread for {peer} ({e})");
    }
}

/// The peer's IPv4 address. An IPv4-mapped IPv6 peer counts; a real IPv6
/// peer does not, because the allow-list is IPv4 and a guess would be a
/// hole in it.
fn ipv4_of(peer: &SocketAddr) -> Option<[u8; 4]> {
    match peer {
        SocketAddr::V4(v4) => Some(v4.ip().octets()),
        SocketAddr::V6(v6) => v6.ip().to_ipv4_mapped().map(|ip| ip.octets()),
    }
}

fn close(stream: &TcpStream) {
    let _ = stream.shutdown(Shutdown::Both);
}

// ---------------------------------------------------------------------
// One connection
// ---------------------------------------------------------------------

fn connection(shared: &Arc<Shared>, mut stream: TcpStream, peer: SocketAddr, unit_id: u8) {
    if stream.set_read_timeout(Some(IDLE_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        log::warn!("modbus: {peer} cannot be given timeouts; closing");
        close(&stream);
        return;
    }
    let _ = stream.set_nodelay(true);

    let mut buf = [0u8; MAX_FRAME];
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        match read_frame(&mut stream, &mut buf) {
            Ok(Some(len)) => {
                let image = shared.image();
                // One frame can carry at most 123 writes, so a span per
                // frame keeps the command ids unique across connections
                // without a lock.
                let mut seq = shared.seq.fetch_add(SEQ_SPAN, Ordering::Relaxed);
                let outcome = handle_frame(unit_id, &buf[..len], &image, &mut seq, &mut |cmd| {
                    shared.submit(cmd)
                });
                if let Some(code) = outcome.result {
                    shared.set_last_result(code);
                }
                if !outcome.response.is_empty() && stream.write_all(&outcome.response).is_err() {
                    break;
                }
                if outcome.broken {
                    log::debug!("modbus: {peer} sent a frame that cannot be answered; closing");
                    break;
                }
            }
            // A clean end of stream, an idle timeout or a frame that is
            // not Modbus TCP: all three mean "this connection is done".
            Ok(None) | Err(_) => break,
        }
    }
    close(&stream);
}

/// Read one whole MBAP frame into `buf`.
///
/// `Ok(None)` means the connection is finished (end of stream, idle
/// timeout, or a header that is not Modbus TCP).
fn read_frame(stream: &mut TcpStream, buf: &mut [u8; MAX_FRAME]) -> std::io::Result<Option<usize>> {
    match stream.read_exact(&mut buf[..MBAP_HEADER]) {
        Ok(()) => {}
        Err(e) if is_closed(&e) => return Ok(None),
        Err(e) => return Err(e),
    }
    let Some(len) = mbap_payload_len(&buf[..MBAP_HEADER]) else {
        return Ok(None);
    };
    let total = MBAP_HEADER + len;
    match stream.read_exact(&mut buf[MBAP_HEADER..total]) {
        Ok(()) => Ok(Some(total)),
        Err(e) if is_closed(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

fn is_closed(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        IoErrorKind::UnexpectedEof
            | IoErrorKind::WouldBlock
            | IoErrorKind::TimedOut
            | IoErrorKind::ConnectionReset
            | IoErrorKind::ConnectionAborted
            | IoErrorKind::BrokenPipe
    )
}
