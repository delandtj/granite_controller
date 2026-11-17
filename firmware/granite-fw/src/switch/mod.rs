//! The block-switch worker (ADR 0006 components 17, 18 and 26).
//!
//! One thread owns the clients and therefore every byte that goes to a
//! switch. It talks to the rest of the firmware the way the MQTT worker
//! does: it reads the live configuration, writes the `switch` part of
//! the [`Observed`] snapshot, publishes events on the bus, and takes
//! requests on a channel. It never touches the actuator, the plane link
//! or the dispatcher, and it is the only thread that blocks on the
//! switch's slow web server (400 ms a page).
//!
//! ### Two switches, one thread
//!
//! A controller that serves two blocks has two block switches, the
//! `switch` and `switch2` sections (ADR 0006 component 26). The thread
//! holds one [`Unit`] per switch and serves them in turn, so requests to
//! the two never overlap and one 16 KB page buffer, allocated once when
//! the thread starts, serves both (ADR 0001 component 27). A switch
//! that does not answer delays the other by one request timeout per
//! attempt; the backoff spaces the attempts. Everything per switch is
//! per switch: client, profile, pairing, own port, drift, snapshot,
//! reachability, password. The faults are shared, raised while either
//! switch has the condition and cleared when neither has.
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
//! A staged switch section is applied **without saving**: the switch's
//! flash keeps the last confirmed configuration, so a reboot of the
//! switch is always a valid way back. On a confirm the worker saves; on
//! a revert it plans from the current state back to the snapshot it took
//! before it applied anything and, only if that cannot be done or does
//! not verify, reboots the switch. Nothing in here calls
//! [`client::SwitchClient::save`] on any other path.
//!
//! ### Which switch it is talking to
//!
//! Every `info.cgi` resolves the (model, firmware) pair against the
//! profile table (ADR 0006 component 24). A `verified` row is what
//! allows writes; no row at all, or a `read_only` row, is polled and
//! published while its pages parse and never written
//! (`switch_fw_unknown`, Modbus fault bit 9). The row's layout is what
//! the parsers, the planner and the published summary use from then on;
//! before the first `info.cgi`, and for a switch without a row, that is
//! the widest layout in the table.
//!
//! ### Read-only by default
//!
//! With `manage` false the worker polls and publishes and writes
//! nothing: the only POSTs it sends are the login and the MAC table's
//! paging, which moves the table's view and no configuration.

pub mod client;

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use granite_core::config::{Config, SwitchCfg, SwitchId};
use granite_core::dispatch::{SwitchRequest, SwitchVerb};
use granite_core::modbus_map::fault_bits;
use granite_core::msg::{Event, switch_event};
use granite_core::observed::Observed;
use granite_core::switch::mgmt;
use granite_core::switch::model::{PortStats, SwitchDesired, SwitchState, same_mac};
use granite_core::switch::obs::SwitchObs;
use granite_core::switch::plan::{self, Protected};
use granite_core::switch::profile::{self, Layout, Profile};
use granite_core::switch::{counters, plan::Op};

use client::{ClientError, PageBuf, SwitchClient};

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

/// True while a switch answers, per switch, for anything that only
/// wants the bit.
static REACHABLE: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

/// True while a switch is configured at all, per switch.
static CONFIGURED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

/// What each switch would set in the Modbus fault word on its own. The
/// word itself gets the union, so a shared fault bit is raised while
/// either switch has the condition and cleared when neither has (ADR
/// 0006 component 26).
static FAULT_SHADOW: [AtomicU16; 2] = [AtomicU16::new(0), AtomicU16::new(0)];

/// The fault bits the two switches share.
const SWITCH_FAULTS: u16 = fault_bits::SWITCH_UNREACHABLE | fault_bits::SWITCH_UNSUPPORTED;

/// Where a request to the worker goes. Set once, when the worker
/// starts: the dispatcher's side effect, the HTTP API's confirm and the
/// config watcher's revert all reach the worker through this and none
/// of them has a handle. The request itself says which switch it is for.
static REQUESTS: Mutex<Option<Sender<SwitchRequest>>> = Mutex::new(None);

/// The counters payload waiting to be published, per switch. The MQTT
/// worker owns the broker session, so the switch worker leaves the
/// payload here and MQTT picks it up on its next pass (ADR 0006
/// components 21 and 26, `.../switch/counters` and
/// `.../switch2/counters`).
static COUNTERS: [Mutex<Option<String>>; 2] = [Mutex::new(None), Mutex::new(None)];

/// True while this switch is reachable (false when it is not
/// configured).
pub fn reachable(which: SwitchId) -> bool {
    let i = which.index();
    CONFIGURED[i].load(Ordering::Relaxed) && REACHABLE[i].load(Ordering::Relaxed)
}

/// True while this switch is configured and polled.
pub fn configured(which: SwitchId) -> bool {
    CONFIGURED[which.index()].load(Ordering::Relaxed)
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

/// Take the counters payload the worker last built for this switch, if
/// it has not been published yet. Called by the MQTT worker.
pub fn take_counters(which: SwitchId) -> Option<String> {
    COUNTERS[which.index()]
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
}

/// What the worker is handed. Mirrors `mqtt::MqttCtx`: closures for the
/// live configuration and the secrets, the shared snapshot, the event
/// bus and the fault word. Shared by the two per-switch units, so the
/// closures have to be callable from the worker thread.
pub struct SwitchCtx {
    /// The live configuration, re-read every tick that needs it.
    pub config: Box<dyn Fn() -> Config + Send + Sync>,
    /// The login password of one switch out of the secrets namespace.
    pub password: Box<dyn Fn(SwitchId) -> String + Send + Sync>,
    /// The snapshot the worker writes `switch` and `switch2` into.
    pub observed: Arc<RwLock<Observed>>,
    /// Where events go.
    pub events: Box<dyn Fn(Event) + Send + Sync>,
    /// The Modbus fault word.
    pub faults: Arc<AtomicU16>,
    /// The controller's own Ethernet MAC, `aa:bb:cc:dd:ee:ff`. The port
    /// it shows up on in a switch's table is the one the controller
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
/// idles on the configuration and starts polling as soon as a section
/// says it may, so enabling a switch needs no reboot.
pub fn start(ctx: SwitchCtx) -> anyhow::Result<SwitchHandle> {
    let (tx, rx) = channel::<SwitchRequest>();
    if let Ok(mut slot) = REQUESTS.lock() {
        *slot = Some(tx);
    }
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let join = crate::platform::spawn(THREAD, STACK, move || {
        Worker::new(Arc::new(ctx)).run(&rx, &worker_stop);
    })?;
    Ok(SwitchHandle {
        stop,
        join: Some(join),
    })
}

/// The thread: one unit per switch, served in turn.
struct Worker {
    units: [Unit; 2],
    /// The one page buffer, allocated here when the worker starts and
    /// handed to every request of both switches (ADR 0001 component
    /// 27). They are served in turn, so one buffer is all it takes.
    page: PageBuf,
}

impl Worker {
    fn new(ctx: Arc<SwitchCtx>) -> Self {
        Worker {
            units: [
                Unit::new(Arc::clone(&ctx), SwitchId::One),
                Unit::new(ctx, SwitchId::Two),
            ],
            page: PageBuf::new(),
        }
    }

    fn run(mut self, rx: &Receiver<SwitchRequest>, stop: &AtomicBool) {
        crate::platform::watch_this_thread("switch");
        log::info!("switch worker started");
        while !stop.load(Ordering::Relaxed) {
            // Once per loop, as before: a request step feeds it again
            // from inside the client (ADR 0006 component 26 keeps the
            // watchdog as it was).
            crate::platform::feed_task_watchdog();
            for unit in self.units.iter_mut() {
                unit.refresh_config();
            }
            // Requests are taken even while a switch is not polled,
            // so a command gets an answer either way.
            loop {
                match rx.try_recv() {
                    Ok(request) => {
                        self.units[request.which.index()].handle(request.verb, &mut self.page);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return,
                }
            }
            // In turn: the second switch's pass starts when the first
            // one's is done, so two slow web servers never overlap.
            //
            // Nothing special about a firmware image coming in any
            // more: every request takes the memory gate first and
            // simply does not get it while an update holds it (ADR
            // 0001 component 27, which replaces the special case of
            // ADR 0006 component 17). The deadlines are late when it
            // is over.
            for unit in self.units.iter_mut() {
                if unit.cfg.should_poll() {
                    unit.poll(&mut self.page);
                }
            }
            thread::sleep(TICK);
        }
        log::info!("switch worker stopped");
    }
}

/// Everything the worker remembers about one switch between ticks.
struct Unit {
    ctx: Arc<SwitchCtx>,
    /// Which switch this is: the section it reads, the log prefix, the
    /// slot in `state` and the number its events carry.
    which: SwitchId,
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
    /// The ports that carry VLAN 1 and are neither the uplink nor the
    /// controller's own port (ADR 0006 component 25). Report only.
    mgmt_exposed: Vec<u8>,
    /// The state as it was before the last apply: what a revert goes
    /// back to (ADR 0006 component 18).
    snapshot: Option<SwitchState>,
    /// The desired document that was applied without being saved.
    applied: Option<SwitchDesired>,
    last_reconcile_ms: u64,
    mismatch: bool,
    fw_unknown: bool,
    /// The profile row the switch resolved to, `None` while `info.cgi`
    /// has not been read or the unit has no row (ADR 0006 component
    /// 24).
    profile: Option<&'static Profile>,
    /// The layout everything is read and planned with: the row's, or
    /// the widest in the table while no row is resolved.
    layout: &'static Layout,
    /// The uplink port the layout has not got, once it was logged.
    uplink_warned: Option<u8>,
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

impl Unit {
    fn new(ctx: Arc<SwitchCtx>, which: SwitchId) -> Self {
        let cfg = (ctx.config)().switch_cfg(which).clone();
        Unit {
            ctx,
            which,
            cfg,
            client: None,
            state: SwitchState::default(),
            last_stats: None,
            last_uptime_s: 0,
            fails: 0,
            reachable: false,
            own_port: None,
            drift: Vec::new(),
            mgmt_exposed: Vec::new(),
            snapshot: None,
            applied: None,
            last_reconcile_ms: 0,
            mismatch: false,
            fw_unknown: false,
            profile: None,
            layout: Layout::widest(),
            uplink_warned: None,
            next_stats_ms: 0,
            next_info_ms: 0,
            next_mac_ms: 0,
            next_drift_ms: 0,
            next_counters_ms: 0,
            retry_at_ms: 0,
            backoff_s: 1,
        }
    }

    /// `switch` or `switch2`: what the log lines say, and the name of
    /// the section this unit reads.
    fn tag(&self) -> &'static str {
        self.which.key()
    }

    /// Pick up a configuration change. A new host or login means a new
    /// client; turning the section off drops the client and the state.
    fn refresh_config(&mut self) {
        let next = (self.ctx.config)().switch_cfg(self.which).clone();
        let password = (self.ctx.password)(self.which);
        let changed = next != self.cfg;
        self.cfg = next;
        CONFIGURED[self.which.index()].store(self.cfg.should_poll(), Ordering::Relaxed);
        let tag = self.tag();
        if !self.cfg.should_poll() {
            if self.client.is_some() {
                log::info!("{tag}: the section is off; dropping the client");
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
            log::info!("{tag}: talking to {host} as {user}");
            self.client = Some(SwitchClient::new(&host, &user, &password));
            self.state = SwitchState::default();
            self.last_stats = None;
            self.own_port = None;
            // Another switch's exposure says nothing about this one.
            self.mgmt_exposed = Vec::new();
            // Another switch is another profile: nothing is known about
            // it until its info.cgi has been read.
            self.profile = None;
            self.layout = Layout::widest();
            self.uplink_warned = None;
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
            // A changed uplink is worth saying again if the switch has
            // not got it either.
            self.uplink_warned = None;
            self.check_uplink();
            // A changed uplink changes which port is exempt from the
            // management VLAN report.
            self.check_mgmt_exposure();
        }
    }

    // -----------------------------------------------------------------
    // Polling
    // -----------------------------------------------------------------

    fn poll(&mut self, page: &mut PageBuf) {
        let now = crate::platform::now_ms();
        if now < self.retry_at_ms {
            return;
        }
        // info.cgi first on a fresh client: the pairing and firmware
        // checks gate everything else (ADR 0006 components 16 and 22).
        if self.state.info.is_none() || now >= self.next_info_ms {
            self.next_info_ms = now + secs(self.cfg.poll.info_s);
            if !self.step(page, |w, p| w.read_health(p)) {
                return;
            }
        }
        if self.mismatch {
            // Nothing beyond info.cgi while this is the wrong switch.
            return;
        }
        if now >= self.next_stats_ms {
            self.next_stats_ms = now + secs(self.cfg.poll.stats_s);
            if !self.step(page, |w, p| w.read_stats(p)) {
                return;
            }
        }
        if now >= self.next_mac_ms {
            self.next_mac_ms = now + secs(self.cfg.poll.mac_s);
            if !self.step(page, |w, p| w.read_macs(p)) {
                return;
            }
            // The walk is where the controller's own port comes from,
            // and that is the port the report exempts.
            self.check_mgmt_exposure();
        }
        if now >= self.next_drift_ms {
            self.next_drift_ms = now + secs(self.cfg.poll.drift_s);
            if !self.step(page, |w, p| w.read_config_pages(p)) {
                return;
            }
            self.check_drift(page);
            self.check_mgmt_exposure();
        }
        if now >= self.next_counters_ms {
            self.next_counters_ms = now + secs(self.cfg.poll.counters_s);
            self.publish_counters(now);
        }
        self.publish_obs(Some(now));
    }

    /// Run one poll step, counting the failure if it fails. False when
    /// the step failed, so the caller stops this pass.
    fn step(
        &mut self,
        page: &mut PageBuf,
        step: impl FnOnce(&mut Self, &mut PageBuf) -> Result<(), ClientError>,
    ) -> bool {
        match step(self, page) {
            Ok(()) => {
                self.note_success();
                true
            }
            // The memory gate said no, so the request never left the
            // board (ADR 0001 component 27). That says nothing about
            // the switch: this pass stops and the next one tries
            // again, and nothing is counted or reported.
            Err(ClientError::Busy(who)) => {
                log::debug!("{}: this pass waits, {who} has the memory gate", self.tag());
                false
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
            log::info!("{}: answering again", self.tag());
        }
    }

    fn note_failure(&mut self, error: &ClientError) {
        self.fails += 1;
        log::warn!(
            "{}: poll failed ({error}), {} in a row",
            self.tag(),
            self.fails
        );
        if error.is_unreachable() || matches!(error, ClientError::Auth) {
            self.backoff_s = (self.backoff_s * 2).min(BACKOFF_MAX_S);
            self.retry_at_ms = crate::platform::now_ms() + self.backoff_s * 1000;
        }
        if self.fails >= UNREACHABLE_AFTER && self.reachable {
            self.set_reachable(false);
            self.event("switch_unreachable", None, Some(format!("{error}")));
            self.publish_obs(None);
        }
        if matches!(error, ClientError::Page(_)) {
            // A page that does not parse means the firmware is not one
            // this image knows, which is a write-blocking fault.
            self.fw_unknown = true;
            self.raise(fault_bits::SWITCH_UNSUPPORTED);
        }
    }

    fn set_reachable(&mut self, up: bool) {
        self.reachable = up;
        REACHABLE[self.which.index()].store(up, Ordering::Relaxed);
        if up {
            self.lower(fault_bits::SWITCH_UNREACHABLE);
        } else {
            self.raise(fault_bits::SWITCH_UNREACHABLE);
        }
    }

    /// Note that this switch has a fault condition, and put the union
    /// of the two switches into the Modbus word (ADR 0006 component
    /// 26: raised while either has it).
    fn raise(&self, bit: u16) {
        FAULT_SHADOW[self.which.index()].fetch_or(bit, Ordering::Relaxed);
        self.publish_faults();
    }

    /// Note that this switch no longer has the condition. The bit goes
    /// out only when the other switch has not got it either.
    fn lower(&self, bit: u16) {
        FAULT_SHADOW[self.which.index()].fetch_and(!bit, Ordering::Relaxed);
        self.publish_faults();
    }

    fn publish_faults(&self) {
        let union = FAULT_SHADOW[0].load(Ordering::Relaxed) | FAULT_SHADOW[1].load(Ordering::Relaxed);
        self.ctx
            .faults
            .fetch_or(union & SWITCH_FAULTS, Ordering::Relaxed);
        self.ctx
            .faults
            .fetch_and(!(SWITCH_FAULTS & !union), Ordering::Relaxed);
    }

    /// Resolve the (model, firmware) pair against the profile table and
    /// take the row's layout (ADR 0006 component 24).
    ///
    /// Writes need a `verified` row. No row at all and a `read_only`
    /// row both keep `switch_fw_unknown` and the `switch_unsupported`
    /// fault bit; the log line says which of the two it is.
    fn resolve_profile(&mut self, model: &str, firmware: &str) {
        let tag = self.tag();
        let found = profile::lookup(model, firmware);
        let layout: &'static Layout = found.map_or_else(Layout::widest, |p| &p.layout);
        if !core::ptr::eq(layout, self.layout) {
            log::info!("{tag}: reading it as a {} port unit", layout.ports);
            self.uplink_warned = None;
        }
        self.layout = layout;
        self.profile = found;
        if let Some(client) = self.client.as_mut() {
            client.set_layout(layout);
        }

        let unknown = !found.is_some_and(Profile::may_write);
        if unknown == self.fw_unknown {
            self.check_uplink();
            return;
        }
        self.fw_unknown = unknown;
        if unknown {
            match found {
                Some(row) => log::warn!(
                    "{tag}: {model} {firmware} is a {} profile in this image; monitoring \
                     continues, writes are refused",
                    row.tier.as_str()
                ),
                None => log::warn!(
                    "{tag}: {model} {firmware} is not a switch this image was tested against; \
                     monitoring continues, writes are refused"
                ),
            }
            self.event(
                "switch_fw_unknown",
                None,
                Some(format!("{model} {firmware}")),
            );
            self.raise(fault_bits::SWITCH_UNSUPPORTED);
        } else {
            log::info!("{tag}: {model} {firmware} is a verified profile; writes are allowed");
            if !self.mismatch {
                self.lower(fault_bits::SWITCH_UNSUPPORTED);
            }
        }
        self.check_uplink();
    }

    /// Say once when the section's `uplink_port` names a port this unit
    /// has not got: it then counts as "no uplink declared" and protects
    /// nothing (ADR 0006 component 24).
    fn check_uplink(&mut self) {
        let Some(port) = self.cfg.uplink() else {
            return;
        };
        if self.layout.is_port(port) {
            self.uplink_warned = None;
            return;
        }
        if self.uplink_warned == Some(port) {
            return;
        }
        self.uplink_warned = Some(port);
        log::warn!(
            "{tag}: {tag}.uplink_port is {port} but this switch has {} port(s), so no uplink \
             is declared and only the controller's own port is protected",
            self.layout.ports,
            tag = self.tag()
        );
    }

    /// `info.cgi`: the pairing check, the firmware check and the
    /// reboot check.
    fn read_health(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let tag = self.tag();
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        let (info, links) = client.read_info(page)?;

        // Pairing (ADR 0006 component 22, the part that is in scope):
        // a switch whose MAC is not the configured one is not ours.
        let want = self.cfg.mac.trim();
        let wrong = !want.is_empty() && !same_mac(&info.mac, want);
        if wrong != self.mismatch {
            self.mismatch = wrong;
            if wrong {
                log::error!(
                    "{tag}: {} answers with MAC {} but {tag}.mac says {want}; talking to \
                     nothing but info.cgi",
                    self.cfg.host,
                    info.mac
                );
                self.event(
                    "switch_mismatch",
                    None,
                    Some(format!("MAC {} is not {want}", info.mac)),
                );
                self.raise(fault_bits::SWITCH_UNSUPPORTED);
            } else {
                self.lower(fault_bits::SWITCH_UNSUPPORTED);
            }
        }

        // The profile (ADR 0006 component 24): the (model, firmware)
        // pair decides the layout the pages are read with and whether
        // anything may be written at all.
        self.resolve_profile(&info.model, &info.firmware);

        // An uptime that went down is a reboot, and a reboot drops
        // everything that was applied but not saved.
        if self.last_uptime_s != 0 && info.uptime_s + 5 < self.last_uptime_s {
            log::warn!(
                "{tag}: uptime went from {} s to {} s, so it rebooted",
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
    fn read_stats(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let tag = self.tag();
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        let stats = client.read_stats(page)?;
        let now = crate::platform::now_ms();

        // A counter that went down with a stable uptime is a Clear on
        // the web UI; with a lower uptime it is the reboot, which
        // read_health has already reported.
        if let Some((previous, _)) = self.last_stats.as_ref()
            && counters::counters_went_backwards(previous, &stats)
        {
            log::info!("{tag}: the counters were reset");
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
                match client.read_ports(page) {
                    Ok(ports) => self.state.ports = ports,
                    Err(e) => log::warn!("{tag}: the port page did not read back: {e}"),
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
    fn read_macs(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let tag = self.tag();
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        self.state.macs = client.walk_macs(MAC_PAGES, page)?;
        let own_mac = self.ctx.own_mac.clone();
        let found = self.state.port_of_mac(&own_mac);
        if found != self.own_port {
            match found {
                Some(port) => log::info!(
                    "{tag}: the controller's own MAC {own_mac} is on port {port}, which is \
                     protected from VLAN and admin-state changes"
                ),
                None => log::info!(
                    "{tag}: the controller's own MAC {own_mac} is not in the switch's table, so \
                     every port is protected until it is"
                ),
            }
            self.own_port = found;
        }
        Ok(())
    }

    /// The configuration pages the desired state needs, read in one go
    /// for the drift check and before a reconcile.
    fn read_config_pages(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        self.state.vlans = Some(client.read_vlans(page)?);
        self.state.port_vlans = client.read_port_vlans(page)?;
        self.state.ports = client.read_ports(page)?;
        self.state.jumbo = Some(client.read_jumbo(page)?);
        self.state.loop_cfg = Some(client.read_loop(page)?);
        self.state.ip = Some(client.read_ip(page)?);
        // Storm control and the rate limits are only read when the
        // desired state mentions them: two more 400 ms pages otherwise
        // buy nothing.
        if self.cfg.desired.storm.is_some() {
            self.state.storm = client.read_storm(page)?;
        }
        Ok(())
    }

    /// Report what differs from the desired state, and put it back when
    /// `enforce` says so.
    fn check_drift(&mut self, page: &mut PageBuf) {
        let tag = self.tag();
        if !self.cfg.manage || self.cfg.desired.is_empty() {
            self.drift.clear();
            return;
        }
        let drift = plan::drift(self.layout, &self.state, &self.cfg.desired);
        if drift == self.drift {
            return;
        }
        self.drift = drift;
        if self.drift.is_empty() {
            return;
        }
        log::warn!("{tag}: drifted from the desired state: {:?}", self.drift);
        self.event("switch_drift", None, Some(self.drift.join("; ")));
        if self.cfg.enforce {
            // The desired state was confirmed once already, so putting
            // it back is applied and saved at once, with no new confirm
            // window (ADR 0006, "Who wins").
            log::warn!("{tag}: enforce is on, putting the desired state back");
            if self.apply_desired(false, page).is_some() {
                self.save_now(page);
            }
        }
    }

    /// Report which ports carry VLAN 1, the switch's management VLAN,
    /// and should not (ADR 0006 component 25).
    ///
    /// Report only, like drift, and whatever `manage` says: nothing is
    /// written, no fault bit is set and the planner knows nothing about
    /// this. The fix is a desired state that makes the node ports
    /// access ports in a data VLAN.
    ///
    /// Called whenever the port VLAN page, the controller's own port or
    /// the uplink can have changed, so at the latest on the drift tick.
    fn check_mgmt_exposure(&mut self) {
        let tag = self.tag();
        let Some(exposed) =
            mgmt::exposed(self.layout, &self.state, self.cfg.uplink(), self.own_port)
        else {
            // No own port yet, or the page was not read: there is no
            // telling which port to exempt, so nothing is reported and
            // the last report stands.
            return;
        };
        if exposed == self.mgmt_exposed {
            return;
        }
        self.mgmt_exposed = exposed;
        if self.mgmt_exposed.is_empty() {
            log::info!("{tag}: only the uplink and the controller's own port carry VLAN 1 again");
            self.event("switch_mgmt_closed", None, None);
            return;
        }
        let list = mgmt::port_list(&self.mgmt_exposed);
        log::warn!(
            "{tag}: port(s) {list} carry VLAN 1, the switch's management VLAN: whatever hangs \
             there can reach the switch's web UI"
        );
        self.event("switch_mgmt_exposed", None, Some(list));
    }

    // -----------------------------------------------------------------
    // Requests
    // -----------------------------------------------------------------

    fn handle(&mut self, verb: SwitchVerb, page: &mut PageBuf) {
        if !self.cfg.should_poll() {
            log::warn!(
                "{}: {verb:?} ignored, no switch is configured",
                self.tag()
            );
            return;
        }
        if !matches!(verb, SwitchVerb::Revert) && !self.writes_allowed() {
            return;
        }
        match verb {
            SwitchVerb::Port {
                port,
                enabled,
                force,
            } => self.port_command(port, enabled, force, page),
            SwitchVerb::Save => self.save_now(page),
            SwitchVerb::Reboot => self.reboot_switch(page),
            SwitchVerb::FactoryReset => {
                if let Some(client) = self.client.as_mut() {
                    match client.factory_reset(page) {
                        Ok(()) => self.event("switch_factory_reset", None, None),
                        Err(e) => self.refuse(format!("factory reset: {e}")),
                    }
                }
                self.state = SwitchState::default();
                self.snapshot = None;
                self.applied = None;
            }
            SwitchVerb::Reconcile | SwitchVerb::Staged => {
                let staged = matches!(verb, SwitchVerb::Staged);
                self.reconcile(staged, page);
            }
            SwitchVerb::Confirmed => self.confirm(page),
            SwitchVerb::Revert => self.revert(page),
        }
    }

    /// True when the worker may write. Everything that stops a write is
    /// reported once, as a `switch_refused` event.
    fn writes_allowed(&mut self) -> bool {
        if !self.cfg.manage {
            self.refuse(format!(
                "{}.manage is false, so nothing is written to the switch",
                self.tag()
            ));
            return false;
        }
        if self.mismatch {
            self.refuse(format!(
                "the switch's MAC is not the one {}.mac names",
                self.tag()
            ));
            return false;
        }
        if self.fw_unknown {
            let what = match self.profile {
                Some(row) => format!("is a {} profile in this image", row.tier.as_str()),
                None => String::from("is not a switch this image was tested against"),
            };
            self.refuse(format!(
                "{} {} {what}, so nothing is written to it",
                self.state.model(),
                self.state.firmware()
            ));
            return false;
        }
        true
    }

    fn refuse(&self, reason: String) {
        log::warn!("{}: refused: {reason}", self.tag());
        (self.ctx.events)(switch_event(
            crate::platform::now_ms(),
            self.which,
            "switch_refused",
            None,
            None,
            Some(reason),
        ));
    }

    /// `switch_port`: enable or disable one port now.
    fn port_command(&mut self, port: u8, enabled: bool, force: bool, page: &mut PageBuf) {
        // The port page has to be current: the form resends the speed,
        // duplex and flow control the port already has.
        if self.step(page, |w, p| w.read_port_page(p)) {
            let protected = self.protected(force);
            match plan::port_enable_op(self.layout, &self.state, port, enabled, &protected) {
                Ok(op) => self.apply_ops(&[op], "switch_port", page),
                Err(e) => self.refuse(e),
            }
        }
    }

    fn read_port_page(&mut self, page: &mut PageBuf) -> Result<(), ClientError> {
        let Some(client) = self.client.as_mut() else {
            return Ok(());
        };
        self.state.ports = client.read_ports(page)?;
        Ok(())
    }

    /// The protected set: the uplink and the controller's own port, or
    /// everything while that port is unknown. `force` clears it.
    fn protected(&self, force: bool) -> Protected {
        if force {
            Protected::none()
        } else {
            // An uplink this unit has not got is dropped here; the log
            // line for it comes from `check_uplink`.
            Protected::new(self.layout, self.cfg.uplink(), self.own_port)
        }
    }

    /// Read, plan, apply. `staged` means this switch's section is in
    /// its confirm window: the snapshot is kept so a revert can undo
    /// it, and nothing is saved.
    fn reconcile(&mut self, staged: bool, page: &mut PageBuf) {
        let tag = self.tag();
        if self.cfg.desired.is_empty() {
            self.refuse(format!(
                "{tag}.desired is empty, so there is nothing to apply"
            ));
            return;
        }
        if !self.step(page, |w, p| w.read_config_pages(p)) {
            self.refuse(String::from(
                "the switch could not be read, so nothing was applied",
            ));
            return;
        }
        match self.apply_desired(staged, page) {
            Some(applied) => {
                if staged {
                    log::warn!(
                        "{tag}: {applied} op(s) applied and not saved; a confirm saves them, the \
                         deadline undoes them"
                    );
                }
            }
            None => log::info!("{tag}: already in the desired state"),
        }
    }

    /// Plan the desired state and apply it. Returns how many ops were
    /// applied, or `None` when there was nothing to do or nothing could
    /// be done.
    fn apply_desired(&mut self, staged: bool, page: &mut PageBuf) -> Option<usize> {
        let desired = self.cfg.desired.clone();
        let protected = self.protected(false);
        let made = plan::plan(self.layout, &self.state, &desired, &protected);
        for refusal in &made.refusals {
            self.refuse(format!("{}: {}", refusal.kind.as_str(), refusal.reason));
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
        self.apply_ops(&made.ops, "switch_reconciled", page);
        Some(count)
    }

    /// Carry out a list of ops, read back and report.
    fn apply_ops(&mut self, ops: &[Op], what: &str, page: &mut PageBuf) {
        let tag = self.tag();
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
            match client.apply(op, page) {
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
                log::warn!("{tag}: following it to its new address {host}");
                client.set_host(host);
            }
        }
        if done.is_empty() {
            return;
        }
        self.last_reconcile_ms = crate::platform::now_ms();
        // Read back: what the switch says now is what the next plan and
        // the next drift check work from.
        if self.step(page, |w, p| w.read_config_pages(p)) {
            self.drift = if self.cfg.desired.is_empty() {
                Vec::new()
            } else {
                plan::drift(self.layout, &self.state, &self.cfg.desired)
            };
            if !self.drift.is_empty() {
                log::warn!(
                    "{tag}: after applying, these still differ: {:?}",
                    self.drift
                );
            }
            // A port VLAN op can have opened or closed the management
            // VLAN on a port.
            self.check_mgmt_exposure();
        }
        self.event(what, None, Some(done.join("; ")));
        self.publish_obs(None);
    }

    /// A confirm: the one place that saves (ADR 0006's invariant).
    fn confirm(&mut self, page: &mut PageBuf) {
        if self.applied.is_none() {
            log::info!(
                "{}: a confirm with nothing applied; saving anyway is not done",
                self.tag()
            );
            return;
        }
        self.save_now(page);
        self.snapshot = None;
        self.applied = None;
    }

    fn save_now(&mut self, page: &mut PageBuf) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        match client.save(page) {
            Ok(()) => self.event("switch_saved", None, None),
            Err(e) => self.refuse(format!("save: {e}")),
        }
    }

    /// A revert: undo while the switch is reachable, reboot it
    /// otherwise, and say which it was (ADR 0006 component 18).
    fn revert(&mut self, page: &mut PageBuf) {
        let tag = self.tag();
        let Some(snapshot) = self.snapshot.clone() else {
            log::info!("{tag}: a revert with nothing applied; nothing to undo");
            return;
        };
        let applied = self.applied.clone().unwrap_or_default();
        if self.reachable && self.step(page, |w, p| w.read_config_pages(p)) {
            let back = plan::undo(self.layout, &self.state, &snapshot, &applied);
            if !back.ops.is_empty() {
                log::warn!("{tag}: undoing {} op(s)", back.ops.len());
                self.apply_ops(&back.ops, "switch_reverting", page);
            }
            // Did it land? The read-back has to match the snapshot.
            let left = plan::plan(
                self.layout,
                &self.state,
                &plan::undo_desired(self.layout, &snapshot, &applied),
                &Protected::none(),
            );
            if left.ops.is_empty() {
                self.event("switch_reverted", None, Some(String::from("undo")));
                self.snapshot = None;
                self.applied = None;
                return;
            }
            log::error!(
                "{tag}: the undo did not verify ({:?}); rebooting the switch instead",
                left.summary()
            );
        }
        // Either the switch is gone or the undo did not verify. A
        // reboot drops everything unsaved, and the flash still holds
        // the last confirmed configuration.
        self.reboot_switch(page);
        self.event("switch_reverted", None, Some(String::from("reboot")));
        self.snapshot = None;
        self.applied = None;
    }

    /// Reboot the switch and wait for it to come back.
    fn reboot_switch(&mut self, page: &mut PageBuf) {
        let Some(client) = self.client.as_mut() else {
            return;
        };
        if let Err(e) = client.reboot(page) {
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
            self.which,
            what,
            port,
            node,
            detail,
        ));
    }

    /// Write the summary into this switch's slot of the shared
    /// snapshot (ADR 0006 component 26: `switch` and `switch2`).
    fn publish_obs(&self, _now: Option<u64>) {
        let Ok(mut observed) = self.ctx.observed.write() else {
            return;
        };
        let slot = observed.switch_slot(self.which);
        if !self.cfg.should_poll() {
            *slot = None;
            return;
        }
        let mut obs = if self.reachable {
            SwitchObs::of_state(self.layout, &self.state, &self.cfg.desired)
        } else {
            SwitchObs::unreachable(slot.as_ref())
        };
        obs.drift = self.drift.clone();
        obs.mgmt_exposed = self.mgmt_exposed.clone();
        obs.last_reconcile = self.last_reconcile_ms;
        obs.mismatch = self.mismatch;
        obs.fw_unknown = self.fw_unknown;
        obs.managed = self.cfg.manage;
        *slot = Some(obs);
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
                if let Ok(mut slot) = COUNTERS[self.which.index()].lock() {
                    *slot = Some(json);
                }
            }
            Err(e) => log::warn!(
                "{}: the counters payload did not serialise: {e}",
                self.tag()
            ),
        }
    }
}

/// Seconds as milliseconds, with a floor so a zero cannot busy-loop.
fn secs(s: u32) -> u64 {
    u64::from(s.max(1)) * 1000
}
