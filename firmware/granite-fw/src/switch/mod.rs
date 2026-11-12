//! The block-switch worker (ADR 0006 components 17 and 18).
//!
//! One thread owns the client and therefore every byte that goes to the
//! switch. It talks to the rest of the firmware the way the MQTT worker
//! does: it reads the live configuration, writes the `switch` part of
//! the [`Observed`] snapshot, publishes events on the bus, and takes
//! requests on a channel. It never touches the actuator, the plane link
//! or the dispatcher, and it is the only thread that blocks on the
//! switch's slow web server (400 ms a page).
//!
//! ### The schedule
//!
//! | What | Page | Period |
//! |---|---|---|
//! | link state and counters | `port.cgi?page=stats` | `poll.stats_s` (5 s) |
//! | port speed and duplex | `port.cgi` | on a link change |
//! | health | `info.cgi` | `poll.info_s` (60 s) |
//! | MAC table | `mac.cgi` walk | `poll.mac_s` (120 s) and on a link change |
//! | drift | the configuration pages | `poll.drift_s` (10 min) |
//! | counters topic | - | `poll.counters_s` (60 s) |
//!
//! ### Reconcile, and the invariant
//!
//! A staged `switch` section is applied **without saving**: the switch's
//! flash keeps the last confirmed configuration, so a reboot of the
//! switch is always a valid way back. On a confirm the worker saves; on
//! a revert it plans from the current state back to the snapshot it took
//! before it applied anything and, only if that cannot be done or does
//! not verify, reboots the switch. Nothing in here calls
//! [`client::SwitchClient::save`] on any other path.
//!
//! ### Read-only by default
//!
//! With `switch.manage` false the worker polls and publishes and writes
//! nothing: the only POSTs it sends are the login and the MAC table's
//! paging, which moves the table's view and no configuration.

pub mod client;

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use granite_core::config::{Config, SwitchCfg};
use granite_core::dispatch::SwitchRequest;
use granite_core::modbus_map::fault_bits;
use granite_core::msg::{Event, switch_event};
use granite_core::observed::Observed;
use granite_core::switch::model::{PortStats, SwitchDesired, SwitchState, firmware_known, same_mac};
use granite_core::switch::obs::SwitchObs;
use granite_core::switch::plan::{self, Protected};
use granite_core::switch::{counters, plan::Op};

use client::{ClientError, SwitchClient};

/// Worker loop period: short enough that a request on the channel is
/// picked up quickly, long enough to be invisible next to a 400 ms page.
const TICK: Duration = Duration::from_millis(200);

/// Failed polls in a row before the switch counts as unreachable.
const UNREACHABLE_AFTER: u32 = 3;

/// Longest a reconnect attempt waits after a failure \[s\], doubling.
const BACKOFF_MAX_S: u64 = 60;

/// MAC table pages the walk reads at most: 30 rows each, so this is
/// 300 entries, well past the 8 nodes plus the uplink of a block.
const MAC_PAGES: u32 = 10;

/// Thread name and stack. It parses pages and serialises JSON.
const THREAD: &core::ffi::CStr = c"switch";
const STACK: usize = 10_240;

/// True while the switch answers, for anything that only wants the bit.
static REACHABLE: AtomicBool = AtomicBool::new(false);

/// True while a switch is configured at all.
static CONFIGURED: AtomicBool = AtomicBool::new(false);

/// Where a request to the worker goes. Set once, when the worker
/// starts: the dispatcher's side effect, the HTTP API's confirm and the
/// config watcher's revert all reach the worker through this and none
/// of them has a handle.
static REQUESTS: Mutex<Option<Sender<SwitchRequest>>> = Mutex::new(None);

/// The counters payload waiting to be published, if any. The MQTT
/// worker owns the broker session, so the switch worker leaves the
/// payload here and MQTT picks it up on its next pass (ADR 0006
/// component 21, `.../switch/counters`).
static COUNTERS: Mutex<Option<String>> = Mutex::new(None);

/// True while the switch is reachable (false when none is configured).
pub fn reachable() -> bool {
    CONFIGURED.load(Ordering::Relaxed) && REACHABLE.load(Ordering::Relaxed)
}

/// True while a switch is configured and polled.
pub fn configured() -> bool {
    CONFIGURED.load(Ordering::Relaxed)
}

/// Hand a request to the worker. False when no worker is running, which
/// is what a caller reports as "the switch worker is not there".
pub fn submit(request: SwitchRequest) -> bool {
    let Ok(guard) = REQUESTS.lock() else {
        return false;
    };
    match guard.as_ref() {
        Some(tx) => tx.send(request).is_ok(),
        None => false,
    }
}

/// Take the counters payload the worker last built, if it has not been
/// published yet. Called by the MQTT worker.
pub fn take_counters() -> Option<String> {
    COUNTERS.lock().ok().and_then(|mut slot| slot.take())
}

/// What the worker is handed. Mirrors `mqtt::MqttCtx`: closures for the
/// live configuration and the secrets, the shared snapshot, the event
/// bus and the fault word.
pub struct SwitchCtx {
    /// The live configuration, re-read every tick that needs it.
    pub config: Box<dyn Fn() -> Config + Send>,
    /// The switch password out of the secrets namespace.
    pub password: Box<dyn Fn() -> String + Send>,
    /// The snapshot the worker writes `switch` into.
    pub observed: Arc<RwLock<Observed>>,
    /// Where events go.
    pub events: Box<dyn Fn(Event) + Send>,
    /// The Modbus fault word.
    pub faults: Arc<AtomicU16>,
    /// The controller's own Ethernet MAC, `aa:bb:cc:dd:ee:ff`. The port
    /// it shows up on in the switch's table is the one the controller
    /// is reached through, and therefore protected.
    pub own_mac: String,
}

/// The running worker. Dropping it stops the thread.
pub struct SwitchHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl SwitchHandle {
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

impl Drop for SwitchHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start the worker.
///
/// Started whatever the configuration says, like the MQTT worker: it
/// idles on the configuration and starts polling as soon as the section
/// says it may, so enabling the switch needs no reboot.
pub fn start(ctx: SwitchCtx) -> anyhow::Result<SwitchHandle> {
    let (tx, rx) = channel::<SwitchRequest>();
    if let Ok(mut slot) = REQUESTS.lock() {
        *slot = Some(tx);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let join = crate::platform::spawn(THREAD, STACK, move || {
        Worker::new(ctx).run(&rx, &worker_stop);
    })?;
    Ok(SwitchHandle {
        stop,
        join: Some(join),
    })
}

/// Everything the worker remembers between ticks.
struct Worker {
    ctx: SwitchCtx,
    cfg: SwitchCfg,
    client: Option<SwitchClient>,
    /// What the switch last read as.
    state: SwitchState,
    /// The previous counter read and when it was taken.
    last_stats: Option<(Vec<PortStats>, u64)>,
    /// The uptime of the last `info.cgi`, to spot a reboot.
    last_uptime_s: u32,
    /// Consecutive failed polls.
    fails: u32,
    reachable: bool,
    /// Which port the controller's own MAC is on, when it was seen.
    own_port: Option<u8>,
    /// What differs from the desired state.
    drift: Vec<String>,
    /// The state as it was before the last apply: what a revert goes
    /// back to (ADR 0006 component 18).
    snapshot: Option<SwitchState>,
    /// The desired document that was applied without being saved.
    applied: Option<SwitchDesired>,
    last_reconcile_ms: u64,
    mismatch: bool,
    fw_unknown: bool,
    /// Deadlines, in monotonic milliseconds.
    next_stats_ms: u64,
    next_info_ms: u64,
    next_mac_ms: u64,
    next_drift_ms: u64,
    next_counters_ms: u64,
    /// Set after a failure: nothing is attempted before this.
    retry_at_ms: u64,
    backoff_s: u64,
}

impl Worker {
    fn new(ctx: SwitchCtx) -> Self {
        let cfg = (ctx.config)().switch;
        Worker {
            ctx,
            cfg,
            client: None,
            state: SwitchState::default(),
            last_stats: None,
            last_uptime_s: 0,
            fails: 0,
            reachable: false,
            own_port: None,
            drift: Vec::new(),
            snapshot: None,
            applied: None,
            last_reconcile_ms: 0,
            mismatch: false,
            fw_unknown: false,
            next_stats_ms: 0,
            next_info_ms: 0,
            next_mac_ms: 0,
            next_drift_ms: 0,
            next_counters_ms: 0,
            retry_at_ms: 0,
            backoff_s: 1,
        }
    }

    fn run(mut self, rx: &Receiver<SwitchRequest>, stop: &AtomicBool) {
        crate::platform::watch_this_thread("switch");
        log::info!("switch worker started");
        while !stop.load(Ordering::Relaxed) {
            crate::platform::feed_task_watchdog();
            self.refresh_config();
            // Requests are taken even while the switch is not polled,
            // so a command gets an answer either way.
            loop {
                match rx.try_recv() {
                    Ok(request) => self.handle(request),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            if self.cfg.should_poll() {
                self.poll();
            }
            thread::sleep(TICK);
        }
        log::info!("switch worker stopped");
    }

    /// Pick up a configuration change. A new host or login means a new
    /// client; turning the section off drops the client and the state.
    fn refresh_config(&mut self) {
        let next = (self.ctx.config)().switch;
        let password = (self.ctx.password)();
        let changed = next != self.cfg;
        self.cfg = next;
        CONFIGURED.store(self.cfg.should_poll(), Ordering::Relaxed);
        if !self.cfg.should_poll() {
            if self.client.is_some() {
                log::info!("switch: the section is off; dropping the client");
                self.client = None;
                self.state = SwitchState::default();
                self.set_reachable(false);
                self.publish_obs(None);
            }
            return;
        }
        let host = self.cfg.host.trim().to_string();
        let user = self.cfg.user.trim().to_string();
        let fresh = match self.client.as_ref() {
            Some(c) => !c.matches(&host, &user, &password),
            None => true,
        };
        if fresh {
            log::info!("switch: talking to {host} as {user}");
            self.client = Some(SwitchClient::new(&host, &user, &password));
            self.state = SwitchState::default();
            self.last_stats = None;
            self.own_port = None;
            self.fails = 0;
            self.next_stats_ms = 0;
            self.next_info_ms = 0;
            self.next_mac_ms = 0;
            self.next_drift_ms = 0;
        } else if changed {
            // A changed desired state is re-checked on the next drift
            // pass rather than applied here: applying is the staged
            // path's job (ADR 0006 component 18).
            self.next_drift_ms = 0;
        }
    }

    // -----------------------------------------------------------------
    // Polling
    // -----------------------------------------------------------------

    fn poll(&mut self) {
        let now = crate::platform::now_ms();
        if now < self.retry_at_ms {
            return;
        }
        // info.cgi first on a fresh client: the pairing and firmware
        // checks gate everything else (ADR 0006 components 16 and 22).
        if self.state.info.is_none() || now >= self.next_info_ms {
            self.next_info_ms = now + secs(self.cfg.poll.info_s);
            if !self.step(|w| w.read_health()) {
                return;
            }
        }
        if self.mismatch {
            // Nothing beyond info.cgi while this is the wrong switch.
            return;
        }
        if now >= self.next_stats_ms {
            self.next_stats_ms = now + secs(self.cfg.poll.stats_s);
            if !self.step(|w| w.read_stats()) {
                return;
            }
        }
        if now >= self.next_mac_ms {
            self.next_mac_ms = now + secs(self.cfg.poll.mac_s);
            if !self.step(|w| w.read_macs()) {
                return;
            }
        }
        if now >= self.next_drift_ms {
            self.next_drift_ms = now + secs(self.cfg.poll.drift_s);
            if !self.step(|w| w.read_config_pages()) {
                return;
            }
            self.check_drift();
        }
        if now >= self.next_counters_ms {
            self.next_counters_ms = now + secs(self.cfg.poll.counters_s);
            self.publish_counters(now);
        }
        self.publish_obs(Some(now));
    }

    /// Run one poll step, counting the failure if it fails. False when
    /// the step failed, so the caller stops this pass.
    fn step(&mut self, step: impl FnOnce(&mut Self) -> Result<(), ClientError>) -> bool {
        match step(self) {
            Ok(()) => {
                self.note_success();
                true
            }
            Err(e) => {
                self.note_failure(&e);
                false
            }
        }
    }

    fn note_success(&mut self) {
        self.fails = 0;
        self.backoff_s = 1;
        self.retry_at_ms = 0;
        if !self.reachable {
            self.set_reachable(true);
            self.event("switch_reachable", None, None);
            log::info!("switch: answering again");
        }
    }

    fn note_failure(&mut self, error: &ClientError) {
        self.fails += 1;
        log::warn!(
            "switch: poll failed ({error}), {} in a row",
            self.fails
        );
        if error.is_unreachable() || matches!(error, ClientError::Auth) {
            self.backoff_s = (self.backoff_s * 2).min(BACKOFF_MAX_S);
            self.retry_at_ms = crate::platform::now_ms() + self.backoff_s * 1000;
        }
        if self.fails >= UNREACHABLE_AFTER && self.reachable {
            self.set_reachable(false);
            self.event(
                "switch_unreachable",
                None,
                Some(format!("{error}")),
            );
            self.publish_obs(None);
        }
        if matches!(error, ClientError::Page(_)) {
            // A page that does not parse means the firmware is not one
            // this image knows, which is a write-blocking fault.
            self.fw_unknown = true;
            self.ctx
                .faults
                .fetch_or(fault_bits::SWITCH_UNSUPPORTED, Ordering::Relaxed);
        }
    }

    fn set_reachable(&mut self, up: bool) {
        self.reachable = up;
        REACHABLE.store(up, Ordering::Relaxed);
        if up {
            self.ctx
                .faults
                .fetch_and(!fault_bits::SWITCH_UNREACHABLE, Ordering::Relaxed);
        } else {
            self.ctx
                .faults
                .fetch_or(fault_bits::SWITCH_UNREACHABLE, Ordering::Relaxed);
        }
    }

    /// `info.cgi`: the pairing check, the firmware check and the
    /// reboot check.
    fn read_health(&mut self) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        let (info, links) = client.read_info()?;

        // Pairing (ADR 0006 component 22, the part that is in scope):
        // a switch whose MAC is not the configured one is not ours.
        let want = self.cfg.mac.trim();
        let wrong = !want.is_empty() && !same_mac(&info.mac, want);
        if wrong != self.mismatch {
            self.mismatch = wrong;
            if wrong {
                log::error!(
                    "switch: {} answers with MAC {} but switch.mac says {want}; talking to \
                     nothing but info.cgi",
                    self.cfg.host,
                    info.mac
                );
                self.event(
                    "switch_mismatch",
                    None,
                    Some(format!("MAC {} is not {want}", info.mac)),
                );
                self.ctx
                    .faults
                    .fetch_or(fault_bits::SWITCH_UNSUPPORTED, Ordering::Relaxed);
            } else {
                self.ctx
                    .faults
                    .fetch_and(!fault_bits::SWITCH_UNSUPPORTED, Ordering::Relaxed);
            }
        }

        let unknown = !firmware_known(&info.firmware);
        if unknown != self.fw_unknown {
            self.fw_unknown = unknown;
            if unknown {
                log::warn!(
                    "switch: firmware {} is not one this image was tested against; monitoring \
                     continues, writes are refused",
                    info.firmware
                );
                self.event(
                    "switch_fw_unknown",
                    None,
                    Some(info.firmware.clone()),
                );
                self.ctx
                    .faults
                    .fetch_or(fault_bits::SWITCH_UNSUPPORTED, Ordering::Relaxed);
            } else if !self.mismatch {
                self.ctx
                    .faults
                    .fetch_and(!fault_bits::SWITCH_UNSUPPORTED, Ordering::Relaxed);
            }
        }

        // An uptime that went down is a reboot, and a reboot drops
        // everything that was applied but not saved.
        if self.last_uptime_s != 0 && info.uptime_s + 5 < self.last_uptime_s {
            log::warn!(
                "switch: uptime went from {} s to {} s, so it rebooted",
                self.last_uptime_s,
                info.uptime_s
            );
            self.event(
                "switch_reboot",
                None,
                Some(format!("uptime {} s", info.uptime_s)),
            );
            if let Some(client) = self.client.as_mut() {
                client.forget_session();
            }
            self.last_stats = None;
            // Unsaved changes are gone: the switch is back on what its
            // flash holds, which is the last confirmed state.
            self.applied = None;
            self.snapshot = None;
        }
        self.last_uptime_s = info.uptime_s;
        self.state.info = Some(info);
        // info.cgi carries the link state too, which keeps the port
        // list usable before the first stats read.
        if self.state.stats.is_empty() {
            self.state.stats = links
                .iter()
                .map(|l| PortStats {
                    port: l.port,
                    enabled: true,
                    link: l.link,
                    ..PortStats::default()
                })
                .collect();
        }
        Ok(())
    }

    /// `port.cgi?page=stats`: link state and the counters, plus the
    /// `switch_port` events a change produces.
    fn read_stats(&mut self) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        let stats = client.read_stats()?;
        let now = crate::platform::now_ms();

        // A counter that went down with a stable uptime is a Clear on
        // the web UI; with a lower uptime it is the reboot, which
        // read_health has already reported.
        if let Some((previous, _)) = self.last_stats.as_ref()
            && counters::counters_went_backwards(previous, &stats)
        {
            log::info!("switch: the counters were reset");
        }

        let mut link_changed = false;
        for port in &stats {
            let was = self.state.stats_port(port.port).copied();
            let changed = was.map(|w| (w.link, w.enabled)) != Some((port.link, port.enabled));
            if was.is_some() && changed {
                link_changed = true;
            }
        }
        let previous = core::mem::replace(&mut self.state.stats, stats);
        if self.last_stats.is_none() {
            self.last_stats = Some((self.state.stats.clone(), now));
        }

        if link_changed {
            // The negotiated speed of a port that just came up only
            // shows on the port page (ADR 0006 component 17).
            if let Some(client) = self.client.as_mut() {
                match client.read_ports() {
                    Ok(ports) => self.state.ports = ports,
                    Err(e) => log::warn!("switch: the port page did not read back: {e}"),
                }
            }
            for port in self.state.stats.clone() {
                let was = previous.iter().find(|p| p.port == port.port);
                if was.map(|w| (w.link, w.enabled)) == Some((port.link, port.enabled)) {
                    continue;
                }
                let speed = self
                    .state
                    .port(port.port)
                    .map(|p| if port.link { p.speed_actual } else { p.speed })
                    .unwrap_or_default();
                let node = self.cfg.desired.node_of_port(port.port);
                self.event_for(
                    "switch_port",
                    Some(port.port),
                    node,
                    Some(format!(
                        "link {} at {speed}, admin {}",
                        if port.link { "up" } else { "down" },
                        if port.enabled { "enabled" } else { "disabled" }
                    )),
                );
            }
            // A link change moves MACs about, so the table is worth
            // re-reading soon (ADR 0006 component 17).
            self.next_mac_ms = now;
        }
        Ok(())
    }

    /// The MAC table walk, which is also how the controller finds its
    /// own port and therefore what is protected.
    fn read_macs(&mut self) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        self.state.macs = client.walk_macs(MAC_PAGES)?;
        let own_mac = self.ctx.own_mac.clone();
        let found = self.state.port_of_mac(&own_mac);
        if found != self.own_port {
            match found {
                Some(port) => log::info!(
                    "switch: the controller's own MAC {own_mac} is on port {port}, which is \
                     protected from VLAN and admin-state changes"
                ),
                None => log::info!(
                    "switch: the controller's own MAC {own_mac} is not in the switch's table, so \
                     every port is protected until it is"
                ),
            }
            self.own_port = found;
        }
        Ok(())
    }

    /// The configuration pages the desired state needs, read in one go
    /// for the drift check and before a reconcile.
    fn read_config_pages(&mut self) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        self.state.vlans = Some(client.read_vlans()?);
        self.state.port_vlans = client.read_port_vlans()?;
        self.state.ports = client.read_ports()?;
        self.state.jumbo = Some(client.read_jumbo()?);
        self.state.loop_cfg = Some(client.read_loop()?);
        self.state.ip = Some(client.read_ip()?);
        // Storm control and the rate limits are only read when the
        // desired state mentions them: two more 400 ms pages otherwise
        // buy nothing.
        if self.cfg.desired.storm.is_some() {
            self.state.storm = client.read_storm()?;
        }
        Ok(())
    }

    /// Report what differs from the desired state, and put it back when
    /// `enforce` says so.
    fn check_drift(&mut self) {
        if !self.cfg.manage || self.cfg.desired.is_empty() {
            self.drift.clear();
            return;
        }
        let drift = plan::drift(&self.state, &self.cfg.desired);
        if drift == self.drift {
            return;
        }
        self.drift = drift;
        if self.drift.is_empty() {
            return;
        }
        log::warn!("switch: drifted from the desired state: {:?}", self.drift);
        self.event(
            "switch_drift",
            None,
            Some(self.drift.join("; ")),
        );
        if self.cfg.enforce {
            // The desired state was confirmed once already, so putting
            // it back is applied and saved at once, with no new confirm
            // window (ADR 0006, "Who wins").
            log::warn!("switch: enforce is on, putting the desired state back");
            if self.apply_desired(false).is_some() {
                self.save_now();
            }
        }
    }

    // -----------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------

    fn handle(&mut self, request: SwitchRequest) {
        if !self.cfg.should_poll() {
            log::warn!("switch: {request:?} ignored, no switch is configured");
            return;
        }
        if !matches!(request, SwitchRequest::Revert) && !self.writes_allowed() {
            return;
        }
        match request {
            SwitchRequest::Port {
                port,
                enabled,
                force,
            } => self.port_command(port, enabled, force),
            SwitchRequest::Save => self.save_now(),
            SwitchRequest::Reboot => self.reboot_switch(),
            SwitchRequest::FactoryReset => {
                if let Some(client) = self.client.as_mut() {
                    match client.factory_reset() {
                        Ok(()) => self.event("switch_factory_reset", None, None),
                        Err(e) => self.refuse(format!("factory reset: {e}")),
                    }
                }
                self.state = SwitchState::default();
                self.snapshot = None;
                self.applied = None;
            }
            SwitchRequest::Reconcile | SwitchRequest::Staged => {
                let staged = matches!(request, SwitchRequest::Staged);
                self.reconcile(staged);
            }
            SwitchRequest::Confirmed => self.confirm(),
            SwitchRequest::Revert => self.revert(),
        }
    }

    /// True when the worker may write. Everything that stops a write is
    /// reported once, as a `switch_refused` event.
    fn writes_allowed(&mut self) -> bool {
        if !self.cfg.manage {
            self.refuse(String::from(
                "switch.manage is false, so nothing is written to the switch",
            ));
            return false;
        }
        if self.mismatch {
            self.refuse(String::from(
                "the switch's MAC is not the one switch.mac names",
            ));
            return false;
        }
        if self.fw_unknown {
            self.refuse(format!(
                "the switch runs firmware {}, which this image was not tested against",
                self.state.firmware()
            ));
            return false;
        }
        true
    }

    fn refuse(&self, reason: String) {
        log::warn!("switch: refused: {reason}");
        (self.ctx.events)(switch_event(
            crate::platform::now_ms(),
            "switch_refused",
            None,
            None,
            Some(reason),
        ));
    }

    /// `switch_port`: enable or disable one port now.
    fn port_command(&mut self, port: u8, enabled: bool, force: bool) {
        // The port page has to be current: the form resends the speed,
        // duplex and flow control the port already has.
        if self.step(|w| w.read_port_page()) {
            let protected = self.protected(force);
            match plan::port_enable_op(&self.state, port, enabled, &protected) {
                Ok(op) => self.apply_ops(&[op], "switch_port"),
                Err(e) => self.refuse(e),
            }
        }
    }

    fn read_port_page(&mut self) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        self.state.ports = client.read_ports()?;
        Ok(())
    }

    /// The protected set: the uplink and the controller's own port, or
    /// everything while that port is unknown. `force` clears it.
    fn protected(&self, force: bool) -> Protected {
        if force {
            Protected::none()
        } else {
            Protected::new(self.cfg.uplink(), self.own_port)
        }
    }

    /// Read, plan, apply. `staged` means the `switch` section is in its
    /// confirm window: the snapshot is kept so a revert can undo it,
    /// and nothing is saved.
    fn reconcile(&mut self, staged: bool) {
        if self.cfg.desired.is_empty() {
            self.refuse(String::from("switch.desired is empty, so there is nothing to apply"));
            return;
        }
        if !self.step(|w| w.read_config_pages()) {
            self.refuse(String::from("the switch could not be read, so nothing was applied"));
            return;
        }
        match self.apply_desired(staged) {
            Some(applied) => {
                if staged {
                    log::warn!(
                        "switch: {applied} op(s) applied and not saved; a confirm saves them, the \
                         deadline undoes them"
                    );
                }
            }
            None => log::info!("switch: already in the desired state"),
        }
    }

    /// Plan the desired state and apply it. Returns how many ops were
    /// applied, or `None` when there was nothing to do or nothing could
    /// be done.
    fn apply_desired(&mut self, staged: bool) -> Option<usize> {
        let desired = self.cfg.desired.clone();
        let protected = self.protected(false);
        let made = plan::plan(&self.state, &desired, &protected);
        for refusal in &made.refusals {
            self.refuse(format!(
                "{}: {}",
                refusal.kind.as_str(),
                refusal.reason
            ));
        }
        if !made.unread.is_empty() {
            self.refuse(format!(
                "these pages have not been read: {}",
                made.unread.join(", ")
            ));
            return None;
        }
        if made.ops.is_empty() {
            self.drift.clear();
            return None;
        }
        // The snapshot is taken before the first write: it is what a
        // revert plans back to (ADR 0006 component 18).
        if staged {
            self.snapshot = Some(self.state.clone());
            self.applied = Some(desired);
        }
        let count = made.ops.len();
        self.apply_ops(&made.ops, "switch_reconciled");
        Some(count)
    }

    /// Carry out a list of ops, read back and report.
    fn apply_ops(&mut self, ops: &[Op], what: &str) {
        let mut done: Vec<String> = Vec::new();
        for op in ops {
            let Some(client) = self.client.as_mut() else {
                return;
            };
            // An address change moves the switch: follow it before the
            // read-back, which is the one op whose verify needs the new
            // address (ADR 0006 component 18).
            let follow = (op.kind == plan::OpKind::Ip)
                .then(|| self.cfg.desired.ip.as_ref().map(|ip| ip.addr.clone()))
                .flatten();
            match client.apply(op) {
                Ok(()) => done.push(op.detail.clone()),
                Err(e) => {
                    self.refuse(format!("{} failed: {e}", op.detail));
                    break;
                }
            }
            if let Some(addr) = follow
                && let Some(host) = addr.split('/').next()
                && let Some(client) = self.client.as_mut()
            {
                log::warn!("switch: following it to its new address {host}");
                client.set_host(host);
            }
        }
        if done.is_empty() {
            return;
        }
        self.last_reconcile_ms = crate::platform::now_ms();
        // Read back: what the switch says now is what the next plan and
        // the next drift check work from.
        if self.step(|w| w.read_config_pages()) {
            self.drift = if self.cfg.desired.is_empty() {
                Vec::new()
            } else {
                plan::drift(&self.state, &self.cfg.desired)
            };
            if !self.drift.is_empty() {
                log::warn!(
                    "switch: after applying, these still differ: {:?}",
                    self.drift
                );
            }
        }
        self.event(what, None, Some(done.join("; ")));
        self.publish_obs(None);
    }

    /// A confirm: the one place that saves (ADR 0006's invariant).
    fn confirm(&mut self) {
        if self.applied.is_none() {
            log::info!("switch: a confirm with nothing applied; saving anyway is not done");
            return;
        }
        self.save_now();
        self.snapshot = None;
        self.applied = None;
    }

    fn save_now(&mut self) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        match client.save() {
            Ok(()) => self.event("switch_saved", None, None),
            Err(e) => self.refuse(format!("save: {e}")),
        }
    }

    /// A revert: undo while the switch is reachable, reboot it
    /// otherwise, and say which it was (ADR 0006 component 18).
    fn revert(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else {
            log::info!("switch: a revert with nothing applied; nothing to undo");
            return;
        };
        let applied = self.applied.clone().unwrap_or_default();
        if self.reachable && self.step(|w| w.read_config_pages()) {
            let back = plan::undo(&self.state, &snapshot, &applied);
            if !back.ops.is_empty() {
                log::warn!("switch: undoing {} op(s)", back.ops.len());
                self.apply_ops(&back.ops, "switch_reverting");
            }
            // Did it land? The read-back has to match the snapshot.
            let left = plan::plan(
                &self.state,
                &plan::undo_desired(&snapshot, &applied),
                &Protected::none(),
            );
            if left.ops.is_empty() {
                self.event(
                    "switch_reverted",
                    None,
                    Some(String::from("undo")),
                );
                self.snapshot = None;
                self.applied = None;
                return;
            }
            log::error!(
                "switch: the undo did not verify ({:?}); rebooting the switch instead",
                left.summary()
            );
        }
        // Either the switch is gone or the undo did not verify. A
        // reboot drops everything unsaved, and the flash still holds
        // the last confirmed configuration.
        self.reboot_switch();
        self.event(
            "switch_reverted",
            None,
            Some(String::from("reboot")),
        );
        self.snapshot = None;
        self.applied = None;
    }

    /// Reboot the switch and wait for it to come back.
    fn reboot_switch(&mut self) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        if let Err(e) = client.reboot() {
            self.refuse(format!("reboot: {e}"));
            return;
        }
        self.event("switch_reboot", None, Some(String::from("asked for")));
        // Every port is down for about 3 s; nothing else here may run
        // in the meantime, and the watchdog is fed while we wait.
        let end = crate::platform::now_ms() + client::REBOOT_WAIT.as_millis() as u64;
        while crate::platform::now_ms() < end {
            crate::platform::feed_task_watchdog();
            thread::sleep(TICK);
        }
        self.state = SwitchState::default();
        self.last_stats = None;
        self.last_uptime_s = 0;
        self.next_info_ms = 0;
        self.next_stats_ms = 0;
        self.next_mac_ms = 0;
        self.next_drift_ms = 0;
    }

    // -----------------------------------------------------------------
    // Publishing
    // -----------------------------------------------------------------

    fn event(&self, what: &str, port: Option<u8>, detail: Option<String>) {
        self.event_for(what, port, None, detail);
    }

    fn event_for(
        &self,
        what: &str,
        port: Option<u8>,
        node: Option<u8>,
        detail: Option<String>,
    ) {
        (self.ctx.events)(switch_event(
            crate::platform::now_ms(),
            what,
            port,
            node,
            detail,
        ));
    }

    /// Write the summary into the shared snapshot.
    fn publish_obs(&self, _now: Option<u64>) {
        let Ok(mut observed) = self.ctx.observed.write() else {
            return;
        };
        if !self.cfg.should_poll() {
            observed.switch = None;
            return;
        }
        let mut obs = if self.reachable {
            SwitchObs::of_state(&self.state, &self.cfg.desired)
        } else {
            SwitchObs::unreachable(observed.switch.as_ref())
        };
        obs.drift = self.drift.clone();
        obs.last_reconcile = self.last_reconcile_ms;
        obs.mismatch = self.mismatch;
        obs.fw_unknown = self.fw_unknown;
        obs.managed = self.cfg.manage;
        observed.switch = Some(obs);
    }

    /// Build the counters payload and leave it for the MQTT worker.
    fn publish_counters(&mut self, now: u64) {
        if self.state.stats.is_empty() {
            return;
        }
        let previous = self
            .last_stats
            .as_ref()
            .map(|(stats, ms)| (stats.as_slice(), *ms));
        let rates = counters::rates(previous, &self.state.stats, now);
        self.last_stats = Some((self.state.stats.clone(), now));
        match rates.to_json() {
            Ok(json) => {
                if let Ok(mut slot) = COUNTERS.lock() {
                    *slot = Some(json);
                }
            }
            Err(e) => log::warn!("switch: the counters payload did not serialise: {e}"),
        }
    }
}

/// Seconds as milliseconds, with a floor so a zero cannot busy-loop.
fn secs(s: u32) -> u64 {
    u64::from(s.max(1)) * 1000
}
