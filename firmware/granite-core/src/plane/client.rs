//! [`PlaneClient`]: discovery, the poll schedule, the commands, the
//! events, the config push and the firmware updates, over one
//! [`PlaneLink`].
//!
//! The firmware calls [`PlaneClient::poll`] every 50-100 ms with a
//! monotonic clock; the client decides what is due:
//!
//! - **discovery** (ADR 0004 decision 5): INFO on 0x30-0x37 at the
//!   first poll and every [`SCAN_PERIOD_MS`] for the addresses that
//!   have no plane. A NAK is absence. A plane the client knows that
//!   misses [`super::link::LOST_AFTER_FAILURES`] transactions in a row
//!   is lost, and is looked for again by the next scan.
//! - **the poll schedule** (ADR 0003 decision 5): STATUS and SUMMARY
//!   every [`STATUS_PERIOD_MS`] per plane, SLOT and UNIT every
//!   [`DETAIL_PERIOD_MS`] or at once when SUMMARY or an event shows a
//!   change, INFO at discovery, after a reboot and whenever an event
//!   says the config hash or the image state may have moved.
//! - **EXP_INT**: the line is the wired OR of every plane's, so an
//!   assertion brings every plane's STATUS forward; the `events
//!   pending` flag says which ones to read EVENTS from, and EV_ACK
//!   goes out once the events are in the client's queue.
//! - **commands**: at most [`super::MAX_IN_FLIGHT`] CTRL writes per
//!   plane, each under its own sequence number; a result that does not
//!   show up in [`RESEND_AFTER_READS`] STATUS reads means the write
//!   was lost, and the same frame goes out again under the same
//!   sequence number, which the plane's sequence memory makes safe.
//!
//! Two things keep one call short. Every poll has a transaction budget
//! ([`MAX_TRANSACTIONS_PER_POLL`]), and the planes are served from a
//! cursor that moves on every poll, so no plane can starve the others;
//! whatever is left over is simply due again on the next call. The 1 Hz
//! polls are also spread: plane *n*'s second starts
//! `n * STATUS_PERIOD_MS / 8` after the client's, so eight planes do
//! not all want their STATUS in the same millisecond.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::vec::Vec;

use granite_proto::{
    BLK_CONFIG_W, BLK_CTRL, BLK_EV_ACK, BLK_EVENTS, BLK_INFO, BLK_SLOT, BLK_STATUS, BLK_SUMMARY,
    BLK_UNIT, Command as Ctrl, EventKind, EventRecord, EventTarget, INDEX_NONE, IndexBitmap,
    MAX_PAYLOAD, MAX_PLANES, RecordWriter, ResultCode, SEQ_MEMORY, SLOT_COUNT, Status, UNIT_COUNT,
    Version, frame,
};

use crate::NodeId;
use crate::hal::PlaneLink;

use super::command::{
    Command, FinishedJob, JobId, JobOutcome, NodeCommand, Step, outcome_of, severity,
};
use super::config::DesiredConfig;
use super::event::ClientEvent;
use super::intent::{INTENT_SETTLE_MS, IntendedOff};
use super::link::{PlaneErrors, Transactions};
use super::model::Plane;
use super::update::{ImageSource, UpdateEnd, UpdateFailure, UpdateJob, UpdateRefused, UpdateStage};
use super::{MAX_IN_FLIGHT, node_id, node_location};

/// How often absent addresses are scanned again \[ms\].
pub const SCAN_PERIOD_MS: u64 = 10_000;

/// The 1 Hz poll of ADR 0003 decision 5: STATUS and SUMMARY \[ms\].
pub const STATUS_PERIOD_MS: u64 = 1_000;

/// The slow sweep of the SLOT and UNIT blocks \[ms\].
pub const DETAIL_PERIOD_MS: u64 = 10_000;

/// A backstop re-read of INFO \[ms\].
///
/// ADR 0003 decision 5 reads INFO at discovery, after a reboot and on a
/// config-hash or image-state change, all of which the client learns
/// from an event. An event can be lost (the plane's ring overflows, or
/// EXP_INT is held by another plane the client cannot read), so INFO is
/// also re-read on a slow timer. It is one 40-byte read a minute per
/// plane.
pub const INFO_PERIOD_MS: u64 = 60_000;

/// Transactions one [`PlaneClient::poll`] may spend.
///
/// At 100 kHz a block read is a few milliseconds, so this bounds a
/// call at some tens of milliseconds even when everything is due at
/// once; what is left over is due again on the next call.
pub const MAX_TRANSACTIONS_PER_POLL: u32 = 12;

/// SLOT or UNIT blocks one plane gives up per poll.
pub const DETAILS_PER_POLL: u32 = 2;

/// STATUS reads without a result before the client resends the same
/// CTRL write under the same sequence number (ADR 0003 decision 4).
pub const RESEND_AFTER_READS: u32 = 3;

/// How often one CTRL write is put on the link before the client calls
/// the plane unreachable for it.
pub const MAX_SENDS: u32 = 6;

/// CTRL writes one poll puts on one plane.
///
/// The plane takes writes off the link into a queue two deep and
/// executes them on its own 20 Hz cycle (ADR 0002 decision 8), so a
/// controller that writes faster than that gets
/// [`ResultCode::Busy`] - not executed, resend. One write per plane per
/// poll is matched to both cadences and leaves room for the
/// acknowledgements.
pub const WRITES_PER_POLL: usize = 1;

/// How long a job may run before the client gives up on it \[ms\].
///
/// A soft off waits [`super::command::OFF_TIMEOUT_S`] for the host and
/// then escalates, and a plane that goes away fails its jobs at once,
/// so this only catches a plane that answers but never finishes.
pub const JOB_TIMEOUT_MS: u64 = 600_000;

/// What ADR 0004 decision 8 calls the update policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    /// An update starts only when something asks for it. The default.
    #[default]
    Manual,
    /// Every plane whose version differs is updated, one at a time,
    /// after the controller's own image is confirmed. The client holds
    /// no image, so the firmware drives this with
    /// [`PlaneClient::update_candidates`].
    Auto,
}

impl UpdatePolicy {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            UpdatePolicy::Manual => "manual",
            UpdatePolicy::Auto => "auto",
        }
    }
}

/// One CTRL write of a job that is waiting for the link.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Queued {
    job: JobId,
    stage: usize,
    commands: Vec<Ctrl>,
    awaits_result: bool,
}

/// One CTRL write of a job that is on the plane.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    job: JobId,
    stage: usize,
    seq: u8,
    /// The payload, kept so a resend is the identical frame.
    payload: Vec<u8>,
    /// How often it has gone out.
    sends: u32,
    /// STATUS reads since the last result for it.
    no_result: u32,
    /// The plane answered "accepted": a sequence is running.
    accepted: bool,
}

/// An `off` or `force_off` that ended, waiting for a fresh SUMMARY so
/// the client can tell whether the rail was cut (see
/// [`super::intent`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingIntent {
    slot: u8,
    until_ms: u64,
}

/// A job the client has accepted and not finished.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Job {
    id: JobId,
    command: Command,
    stages: Vec<Vec<Step>>,
    stage: usize,
    /// Writes of the current stage that have no final result yet.
    outstanding: usize,
    /// The worst result code any plane has given it.
    code: Option<ResultCode>,
    submitted_ms: u64,
}

/// Everything the client knows and schedules for one address.
#[derive(Debug, Clone)]
struct PlaneState {
    plane: Plane,
    /// A plane answers here.
    present: bool,
    next_status_ms: u64,
    next_detail_ms: u64,
    next_info_ms: u64,
    /// INFO has to be read before anything else on this plane.
    info_due: bool,
    /// Slots whose full SLOT block is wanted now.
    slot_dirty: IndexBitmap,
    /// Units whose UNIT block is wanted now.
    unit_dirty: IndexBitmap,
    /// The plane holds unacknowledged events.
    events_due: bool,
    /// The next command sequence number to use.
    seq: u8,
    in_flight: Vec<Pending>,
    queue: VecDeque<Queued>,
    config: Option<DesiredConfig>,
    /// A CONFIG_W set has gone out and INFO has not been re-read.
    config_written: bool,
    /// The boot count the intended-OFF set was last re-applied for.
    reapplied_boot: Option<u32>,
    pending_intent: Vec<PendingIntent>,
    /// The collision flag has been reported once.
    collision_reported: bool,
}

impl PlaneState {
    fn new(address: u8, seq: u8) -> Self {
        PlaneState {
            plane: Plane::new(address),
            present: false,
            next_status_ms: 0,
            next_detail_ms: 0,
            next_info_ms: 0,
            info_due: false,
            slot_dirty: IndexBitmap::NONE,
            unit_dirty: IndexBitmap::NONE,
            events_due: false,
            seq,
            in_flight: Vec::new(),
            queue: VecDeque::new(),
            config: None,
            config_written: false,
            reapplied_boot: None,
            pending_intent: Vec::new(),
            collision_reported: false,
        }
    }

    /// The next sequence number, skipping the ones in flight.
    fn next_seq(&mut self) -> u8 {
        for _ in 0..=u8::MAX {
            let seq = self.seq;
            self.seq = self.seq.wrapping_add(1);
            if !self.in_flight.iter().any(|p| p.seq == seq) {
                return seq;
            }
        }
        self.seq
    }

    /// Mark a slot, and its unit, as wanting a full read.
    fn dirty_slot(&mut self, slot: u8) {
        if (slot as usize) < SLOT_COUNT {
            self.slot_dirty = self.slot_dirty.plus(slot);
        }
    }

    /// Mark a unit, and both its slots, as wanting a full read.
    fn dirty_unit(&mut self, unit: u8) {
        if (unit as usize) < UNIT_COUNT {
            self.unit_dirty = self.unit_dirty.plus(unit);
            self.dirty_slot(2 * unit);
            self.dirty_slot(2 * unit + 1);
        }
    }

    /// Everything wants a full read (the 10 s sweep, and a plane that
    /// has just been found).
    fn dirty_all(&mut self) {
        self.slot_dirty = IndexBitmap::from_bits(0xFF);
        self.unit_dirty = IndexBitmap::from_bits(0x0F);
    }
}

/// The controller's side of the plane link.
pub struct PlaneClient {
    planes: [PlaneState; MAX_PLANES],
    tx: Transactions,
    events: VecDeque<ClientEvent>,
    finished: VecDeque<FinishedJob>,
    jobs: Vec<Job>,
    next_job: JobId,
    next_scan_ms: u64,
    scan_due: bool,
    /// Which plane is served first this poll.
    cursor: usize,
    intent: IntendedOff,
    update: Option<UpdateJob>,
    policy: UpdatePolicy,
    /// Scratch for one answer; no read allocates.
    buf: [u8; frame::MAX_ANSWER],
}

impl core::fmt::Debug for PlaneClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PlaneClient")
            .field("planes", &self.present_count())
            .field("jobs", &self.jobs.len())
            .field("events", &self.events.len())
            .field("update", &self.update)
            .finish_non_exhaustive()
    }
}

impl PlaneClient {
    /// A client with nothing discovered yet.
    ///
    /// `seed` seeds the command sequence numbers: ADR 0003's expert
    /// answer wants them to start at a random value, so a controller
    /// that reboots mid-command cannot collide with the sequence
    /// memory a plane still holds. The firmware passes something from
    /// its RNG; a test passes a constant.
    pub fn new(seed: u32) -> Self {
        PlaneClient {
            planes: core::array::from_fn(|n| {
                PlaneState::new(granite_proto::ADDR_BASE + n as u8, seq_start(seed, n as u8))
            }),
            tx: Transactions::new(),
            events: VecDeque::new(),
            finished: VecDeque::new(),
            jobs: Vec::new(),
            next_job: 1,
            next_scan_ms: 0,
            scan_due: true,
            cursor: 0,
            intent: IntendedOff::new(),
            update: None,
            policy: UpdatePolicy::Manual,
            buf: [0u8; frame::MAX_ANSWER],
        }
    }

    // --- the poll -----------------------------------------------------

    /// Do whatever is due. Called often; never blocks.
    pub fn poll<L: PlaneLink>(&mut self, now_ms: u64, link: &mut L) {
        let mut budget = MAX_TRANSACTIONS_PER_POLL;

        // EXP_INT is shared: it says "someone has events", not which
        // one, so every plane's STATUS comes forward (ADR 0003
        // decision 5).
        if link.int_asserted() {
            for state in &mut self.planes {
                if state.present {
                    state.next_status_ms = state.next_status_ms.min(now_ms);
                }
            }
        }

        if now_ms >= self.next_scan_ms {
            self.next_scan_ms = now_ms.saturating_add(SCAN_PERIOD_MS);
            self.scan_due = true;
        }
        if self.scan_due {
            self.scan(now_ms, link, &mut budget);
        }

        self.cursor = (self.cursor + 1) % MAX_PLANES;
        for step in 0..MAX_PLANES {
            if budget == 0 {
                break;
            }
            let i = (self.cursor + step) % MAX_PLANES;
            if self.planes[i].present {
                self.service(i, now_ms, link, &mut budget);
            }
        }

        self.step_update(now_ms, link, &mut budget);
        self.expire_jobs(now_ms);
        self.expire_intents(now_ms);
    }

    /// INFO on every address that has no plane.
    fn scan<L: PlaneLink>(&mut self, now_ms: u64, link: &mut L, budget: &mut u32) {
        for i in 0..MAX_PLANES {
            if self.planes[i].present {
                continue;
            }
            if *budget == 0 {
                // Carry on at the next poll; the rest of the scan is
                // still due.
                return;
            }
            let address = self.planes[i].plane.address;
            *budget -= 1;
            match self
                .tx
                .read(link, address, BLK_INFO, INDEX_NONE, &mut self.buf)
            {
                Ok(n) => {
                    let payload_len = n;
                    self.planes[i].present = true;
                    self.planes[i]
                        .plane
                        .apply_info(now_ms, &self.buf[..payload_len]);
                    let state = &mut self.planes[i];
                    state.next_status_ms = now_ms + status_phase(i);
                    state.next_detail_ms = now_ms;
                    state.next_info_ms = now_ms + INFO_PERIOD_MS;
                    state.info_due = false;
                    state.config_written = false;
                    state.reapplied_boot = None;
                    state.dirty_all();
                    let (firmware, boot_count) = (state.plane.firmware, state.plane.boot_count);
                    self.events.push_back(ClientEvent::PlaneFound {
                        address,
                        firmware,
                        boot_count,
                    });
                }
                Err(_) => self.note_collision(i),
            }
        }
        self.scan_due = false;
    }

    /// One plane's turn.
    fn service<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L, budget: &mut u32) {
        if (self.planes[i].info_due || now_ms >= self.planes[i].next_info_ms)
            && spend(budget)
            && self.read_info(i, now_ms, link)
        {
            // INFO told the client who the plane is now; the config
            // push hangs off its hash.
            self.after_info(i);
        }

        if self.config_push_due(i) && spend(budget) {
            self.push_config(i, now_ms, link);
        }

        if now_ms >= self.planes[i].next_status_ms && spend(budget) {
            self.planes[i].next_status_ms = now_ms + STATUS_PERIOD_MS;
            if self.read_status(i, now_ms, link) {
                self.after_status(i, now_ms);
                if spend(budget) {
                    self.read_summary(i, now_ms, link);
                }
            }
        }

        if self.planes[i].events_due && spend(budget) {
            self.read_events(i, now_ms, link, budget);
        }

        if now_ms >= self.planes[i].next_detail_ms {
            self.planes[i].next_detail_ms = now_ms + DETAIL_PERIOD_MS;
            self.planes[i].dirty_all();
        }
        self.read_details(i, now_ms, link, budget);

        self.send_commands(i, now_ms, link, budget);
    }

    // --- the read blocks ----------------------------------------------

    /// Read INFO; false when the read failed.
    fn read_info<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L) -> bool {
        let address = self.planes[i].plane.address;
        match self
            .tx
            .read(link, address, BLK_INFO, INDEX_NONE, &mut self.buf)
        {
            Ok(n) => {
                let change = self.planes[i].plane.apply_info(now_ms, &self.buf[..n]);
                let state = &mut self.planes[i];
                state.info_due = false;
                state.next_info_ms = now_ms + INFO_PERIOD_MS;
                if change.rebooted {
                    let (boot_count, reset_cause) =
                        (state.plane.boot_count, state.plane.last_reset_cause);
                    let lost = state.plane.intended_state_lost();
                    state.dirty_all();
                    state.reapplied_boot = None;
                    self.events.push_back(ClientEvent::PlaneRebooted {
                        address,
                        boot_count,
                        reset_cause,
                        intended_state_lost: lost,
                    });
                }
                true
            }
            Err(_) => {
                self.after_failure(i, now_ms);
                false
            }
        }
    }

    /// What hangs off a fresh INFO: the config hash comparison.
    fn after_info(&mut self, i: usize) {
        let hash = self.planes[i].plane.config_hash;
        let address = self.planes[i].plane.address;
        if !self.planes[i].config_written {
            return;
        }
        self.planes[i].config_written = false;
        let Some(config) = self.planes[i].config.as_mut() else {
            return;
        };
        if config.needs_push(hash) {
            // The set went out and the hash did not move: the plane did
            // not take it. `needs_push` counts the tries.
            if config.is_refused() {
                self.events
                    .push_back(ClientEvent::ConfigRefused { address });
            }
        } else {
            config.note_stored(hash);
            self.events
                .push_back(ClientEvent::ConfigPushed { address, hash });
        }
    }

    /// Read STATUS; false when the read failed.
    fn read_status<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L) -> bool {
        let address = self.planes[i].plane.address;
        let before = self.planes[i].plane.uptime_s;
        match self
            .tx
            .read(link, address, BLK_STATUS, INDEX_NONE, &mut self.buf)
        {
            Ok(n) => {
                // The command results travel in this block; take them
                // before the payload is overwritten.
                let mut results = [(0u8, ResultCode::Ok); SEQ_MEMORY];
                let mut count = 0usize;
                for record in Status::records(&self.buf[..n]).flatten() {
                    if let Status::CommandResult(held) = record
                        && count < results.len()
                    {
                        results[count] = (held.seq, held.code);
                        count += 1;
                    }
                }
                self.planes[i].plane.apply_status(now_ms, &self.buf[..n]);
                let state = &mut self.planes[i];
                state.events_due = state.plane.events_pending();
                if state.plane.uptime_s < before {
                    // The uptime went backwards: the plane restarted
                    // and INFO has to say what it is now.
                    state.info_due = true;
                }
                self.resolve_results(i, now_ms, &results[..count]);
                true
            }
            Err(_) => {
                self.after_failure(i, now_ms);
                false
            }
        }
    }

    /// What hangs off a fresh STATUS: the intended-OFF re-apply.
    fn after_status(&mut self, i: usize, now_ms: u64) {
        let state = &self.planes[i];
        if !state.plane.intended_state_lost() || state.plane.info_ms == 0 {
            return;
        }
        let boot = state.plane.boot_count;
        if state.reapplied_boot == Some(boot) {
            return;
        }
        let address = state.plane.address;
        self.planes[i].reapplied_boot = Some(boot);
        let nodes = self.intent.of_plane(address);
        if nodes.is_empty() {
            return;
        }
        for node in &nodes {
            self.submit(now_ms, NodeCommand::PowerOff { node: *node });
        }
        self.events
            .push_back(ClientEvent::IntendedOffReapplied { address, nodes });
    }

    /// Read SUMMARY.
    fn read_summary<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L) {
        let address = self.planes[i].plane.address;
        match self
            .tx
            .read(link, address, BLK_SUMMARY, INDEX_NONE, &mut self.buf)
        {
            Ok(n) => {
                let changed = self.planes[i].plane.apply_summary(now_ms, &self.buf[..n]);
                for slot in 0..SLOT_COUNT as u8 {
                    if changed.has(slot) {
                        self.planes[i].dirty_slot(slot);
                    }
                }
                self.resolve_intents(i);
            }
            Err(_) => self.after_failure(i, now_ms),
        }
    }

    /// Read the SLOT and UNIT blocks that are wanted, a few per poll.
    fn read_details<L: PlaneLink>(
        &mut self,
        i: usize,
        now_ms: u64,
        link: &mut L,
        budget: &mut u32,
    ) {
        let address = self.planes[i].plane.address;
        for _ in 0..DETAILS_PER_POLL {
            let unit = (0..UNIT_COUNT as u8).find(|u| self.planes[i].unit_dirty.has(*u));
            let slot = (0..SLOT_COUNT as u8).find(|s| self.planes[i].slot_dirty.has(*s));
            let (block, index) = match (unit, slot) {
                (Some(u), _) => (BLK_UNIT, u),
                (None, Some(s)) => (BLK_SLOT, s),
                (None, None) => return,
            };
            if !spend(budget) {
                return;
            }
            match self.tx.read(link, address, block, index, &mut self.buf) {
                Ok(n) => {
                    if block == BLK_UNIT {
                        self.planes[i]
                            .plane
                            .apply_unit(now_ms, index, &self.buf[..n]);
                        self.planes[i].unit_dirty = self.planes[i]
                            .unit_dirty
                            .without(IndexBitmap::NONE.plus(index));
                    } else {
                        self.planes[i]
                            .plane
                            .apply_slot(now_ms, index, &self.buf[..n]);
                        self.planes[i].slot_dirty = self.planes[i]
                            .slot_dirty
                            .without(IndexBitmap::NONE.plus(index));
                    }
                }
                Err(_) => {
                    self.after_failure(i, now_ms);
                    return;
                }
            }
        }
    }

    /// Read EVENTS, queue what they say, acknowledge them.
    fn read_events<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L, budget: &mut u32) {
        let address = self.planes[i].plane.address;
        let n = match self
            .tx
            .read(link, address, BLK_EVENTS, INDEX_NONE, &mut self.buf)
        {
            Ok(n) => n,
            Err(_) => {
                self.after_failure(i, now_ms);
                return;
            }
        };
        let mut through: Option<u16> = None;
        let mut queue: Vec<ClientEvent> = Vec::new();
        let mut dirty_slots: Vec<u8> = Vec::new();
        let mut dirty_units: Vec<u8> = Vec::new();
        let mut info_due = false;
        let mut retry_limits: Vec<u8> = Vec::new();
        for record in EventRecord::records(&self.buf[..n]).flatten() {
            match record {
                EventRecord::Event(event) => {
                    through = Some(match through {
                        Some(seq) if seq >= event.seq => seq,
                        _ => event.seq,
                    });
                    match event.target {
                        EventTarget::Slot(slot) => {
                            dirty_slots.push(slot);
                            if event.kind == EventKind::RetryLimitReached {
                                retry_limits.push(slot);
                            }
                            queue.push(ClientEvent::Node {
                                node: node_id(address, slot).unwrap_or(0),
                                address,
                                slot,
                                kind: event.kind,
                                uptime_s: event.uptime_s,
                            });
                        }
                        EventTarget::Unit(unit) => {
                            dirty_units.push(unit);
                            queue.push(ClientEvent::Unit {
                                address,
                                unit,
                                kind: event.kind,
                                uptime_s: event.uptime_s,
                            });
                        }
                        EventTarget::Plane => {
                            if matches!(
                                event.kind,
                                EventKind::Boot
                                    | EventKind::ConfigStored
                                    | EventKind::UpdateStateChanged
                                    | EventKind::ImageConfirmed
                                    | EventKind::RolledBack
                            ) {
                                info_due = true;
                            }
                            queue.push(ClientEvent::Plane {
                                address,
                                kind: event.kind,
                                uptime_s: event.uptime_s,
                            });
                        }
                    }
                }
                EventRecord::Lost(lost) => queue.push(ClientEvent::EventsLost {
                    address,
                    count: lost.count,
                }),
            }
        }
        for slot in dirty_slots {
            self.planes[i].dirty_slot(slot);
        }
        for unit in dirty_units {
            self.planes[i].dirty_unit(unit);
        }
        for slot in retry_limits {
            self.planes[i].plane.note_retry_limit(slot);
        }
        self.planes[i].info_due |= info_due;
        for event in queue {
            self.events.push_back(event);
        }
        // EV_ACK only once the events are in the client's queue
        // (ADR 0003 decision 3: reading clears nothing).
        if let Some(seq) = through
            && spend(budget)
        {
            let mut buf = [0u8; 8];
            let mut w = RecordWriter::new(&mut buf);
            if granite_proto::AckThrough::new(seq).encode(&mut w).is_ok() {
                let payload = w.payload();
                if self.tx.write(link, address, BLK_EV_ACK, payload).is_ok() {
                    self.planes[i].events_due = false;
                } else {
                    self.after_failure(i, now_ms);
                }
            }
        }
    }

    // --- failures -----------------------------------------------------

    /// A transaction on this plane failed: lose the plane if it has
    /// missed enough of them.
    fn after_failure(&mut self, i: usize, now_ms: u64) {
        let address = self.planes[i].plane.address;
        self.note_collision(i);
        if !self.tx.errors(address).is_lost() || !self.planes[i].present {
            return;
        }
        self.planes[i].present = false;
        self.planes[i].plane.forget_states();
        self.planes[i].events_due = false;
        self.events.push_back(ClientEvent::PlaneLost { address });
        self.fail_jobs_of(address, JobOutcome::LinkLost, now_ms);
    }

    /// Report a suspected address collision once.
    fn note_collision(&mut self, i: usize) {
        let address = self.planes[i].plane.address;
        if !self.tx.errors(address).collision_suspected || self.planes[i].collision_reported {
            return;
        }
        self.planes[i].collision_reported = true;
        self.events
            .push_back(ClientEvent::AddressCollisionSuspected { address });
    }

    /// Fail every job that has work on a plane.
    fn fail_jobs_of(&mut self, address: u8, outcome: JobOutcome, now_ms: u64) {
        let mut hit: Vec<JobId> = Vec::new();
        if let Some(n) = granite_proto::plane_of_address(address) {
            let state = &mut self.planes[n as usize];
            for pending in state.in_flight.drain(..) {
                if !hit.contains(&pending.job) {
                    hit.push(pending.job);
                }
            }
            for queued in state.queue.drain(..) {
                if !hit.contains(&queued.job) {
                    hit.push(queued.job);
                }
            }
            state.pending_intent.clear();
        }
        for id in hit {
            self.finish_job(id, outcome, now_ms);
        }
    }

    // --- commands -----------------------------------------------------

    /// Accept a command and give back its job id.
    ///
    /// The job finishes asynchronously (ADR 0004 decision 6): the
    /// caller drains [`PlaneClient::drain_jobs`] for the outcome. A
    /// command for a node or plane that is not there finishes at once,
    /// before this call returns, with [`JobOutcome::Absent`].
    pub fn submit(&mut self, now_ms: u64, command: impl Into<Command>) -> JobId {
        let command = command.into();
        let id = self.next_job;
        self.next_job = self.next_job.wrapping_add(1).max(1);
        let stages = command.stages();
        let job = Job {
            id,
            command,
            stages,
            stage: 0,
            outstanding: 0,
            code: None,
            submitted_ms: now_ms,
        };
        let absent = job.stages.is_empty() || !self.targets_present(&job);
        self.jobs.push(job);
        if absent {
            self.finish_job(id, JobOutcome::Absent, now_ms);
            return id;
        }
        self.start_stage(id, now_ms);
        id
    }

    /// True when every node, unit and plane the job names is there.
    fn targets_present(&self, job: &Job) -> bool {
        for address in job.command.addresses() {
            let Some(n) = granite_proto::plane_of_address(address) else {
                return false;
            };
            if !self.planes[n as usize].present {
                return false;
            }
        }
        if let Command::Plane(super::PlaneCommand::UnitReset { address, unit }) = &job.command {
            let Some(n) = granite_proto::plane_of_address(*address) else {
                return false;
            };
            let state = &self.planes[n as usize];
            if !state.present {
                return false;
            }
            if !state
                .plane
                .units
                .get(*unit as usize)
                .is_some_and(|u| u.is_present())
            {
                return false;
            }
        }
        for node in job.command.nodes() {
            let Some((address, slot)) = node_location(node) else {
                return false;
            };
            let Some(n) = granite_proto::plane_of_address(address) else {
                return false;
            };
            let state = &self.planes[n as usize];
            // A node the client has not read yet is taken on trust: the
            // plane answers, and it is the plane that says "not
            // present" if the slot is empty. A node the client has read
            // and knows is absent fails here, without a write.
            let read = state
                .plane
                .node(slot)
                .is_some_and(|node| node.summary_ms != 0 || node.detail_ms != 0);
            if read
                && state
                    .plane
                    .node(slot)
                    .is_some_and(|node| !node.is_present())
            {
                return false;
            }
        }
        true
    }

    /// Queue every write of a job's current stage.
    fn start_stage(&mut self, id: JobId, now_ms: u64) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        let stage = job.stage;
        let Some(steps) = job.stages.get(stage).cloned() else {
            let code = job.code;
            let outcome = code.map_or(JobOutcome::Done, outcome_of);
            self.finish_job(id, outcome, now_ms);
            return;
        };
        job.outstanding = steps.len();
        for step in steps {
            let Some(n) = granite_proto::plane_of_address(step.address) else {
                continue;
            };
            self.planes[n as usize].queue.push_back(Queued {
                job: id,
                stage,
                commands: step.commands,
                awaits_result: step.awaits_result,
            });
        }
    }

    /// Put the queued writes of one plane on the link, and resend the
    /// ones whose result never came.
    fn send_commands<L: PlaneLink>(
        &mut self,
        i: usize,
        now_ms: u64,
        link: &mut L,
        budget: &mut u32,
    ) {
        // Resends first: a write with no result is older than anything
        // still in the queue. The frame is the identical one, under the
        // same sequence number, which is what makes it safe
        // (ADR 0003 decision 3: the plane's sequence memory).
        //
        // A write the plane has already answered "accepted" is never
        // resent. The plane's memory is eight sequence numbers deep: if
        // a long sequence's entry were pushed out of it, a resend would
        // no longer look like a duplicate and the command would run a
        // second time. Such a job ends on [`JOB_TIMEOUT_MS`] instead,
        // which is the harmless direction.
        let stale: Vec<u8> = self.planes[i]
            .in_flight
            .iter()
            .filter(|p| !p.accepted && p.no_result >= RESEND_AFTER_READS)
            .map(|p| p.seq)
            .take(WRITES_PER_POLL)
            .collect();
        let mut written = 0usize;
        for seq in stale {
            if !spend(budget) {
                return;
            }
            written += 1;
            let address = self.planes[i].plane.address;
            let Some(pending) = self.planes[i].in_flight.iter().find(|p| p.seq == seq) else {
                continue;
            };
            let (payload, sends, job) = (pending.payload.clone(), pending.sends, pending.job);
            if sends >= MAX_SENDS {
                self.finish_job(job, JobOutcome::LinkLost, now_ms);
                continue;
            }
            match self.tx.write(link, address, BLK_CTRL, &payload) {
                Ok(()) => {
                    if let Some(pending) =
                        self.planes[i].in_flight.iter_mut().find(|p| p.seq == seq)
                    {
                        pending.sends += 1;
                        pending.no_result = 0;
                    }
                }
                Err(_) => {
                    self.after_failure(i, now_ms);
                    return;
                }
            }
        }

        while self.planes[i].in_flight.len() < MAX_IN_FLIGHT {
            if self.planes[i].queue.is_empty() || written >= WRITES_PER_POLL {
                return;
            }
            if !spend(budget) {
                return;
            }
            written += 1;
            let queued = self.planes[i].queue.pop_front().expect("not empty");
            let seq = self.planes[i].next_seq();
            let mut buf = [0u8; MAX_PAYLOAD];
            let mut w = RecordWriter::new(&mut buf);
            if Ctrl::write_ctrl(&mut w, seq, &queued.commands).is_err() {
                self.step_finished(queued.job, queued.stage, ResultCode::BadArgument, now_ms);
                continue;
            }
            let payload = w.payload().to_vec();
            let address = self.planes[i].plane.address;
            match self.tx.write(link, address, BLK_CTRL, &payload) {
                Ok(()) => {
                    if queued.awaits_result {
                        self.planes[i].in_flight.push(Pending {
                            job: queued.job,
                            stage: queued.stage,
                            seq,
                            payload,
                            sends: 1,
                            no_result: 0,
                            accepted: false,
                        });
                    } else {
                        // A reboot never reports: the plane resets
                        // before its result can be read.
                        self.step_finished(queued.job, queued.stage, ResultCode::Ok, now_ms);
                    }
                }
                Err(_) => {
                    // Keep it: the next poll sends it again, and the
                    // plane never saw it.
                    self.planes[i].queue.push_front(queued);
                    self.after_failure(i, now_ms);
                    return;
                }
            }
        }
    }

    /// Match the results of a STATUS read against what is in flight.
    fn resolve_results(&mut self, i: usize, now_ms: u64, results: &[(u8, ResultCode)]) {
        let mut done: Vec<(JobId, usize, ResultCode)> = Vec::new();
        self.planes[i].in_flight.retain_mut(|pending| {
            match results.iter().find(|(seq, _)| *seq == pending.seq) {
                None => {
                    pending.no_result = pending.no_result.saturating_add(1);
                    true
                }
                Some((_, code)) if code.is_pending() => {
                    pending.accepted = true;
                    pending.no_result = 0;
                    true
                }
                Some((_, ResultCode::Busy)) => {
                    // The plane's write queue was full, so it never ran
                    // the write: the same frame goes out again under
                    // the same sequence number (the plane's memory only
                    // deduplicates writes it executed).
                    pending.no_result = RESEND_AFTER_READS;
                    true
                }
                Some((_, code)) => {
                    done.push((pending.job, pending.stage, *code));
                    false
                }
            }
        });
        for (job, stage, code) in done {
            self.step_finished(job, stage, code, now_ms);
        }
    }

    /// One write of a job's stage has a final result.
    fn step_finished(&mut self, id: JobId, stage: usize, code: ResultCode, now_ms: u64) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        if job.stage != stage {
            return;
        }
        job.code = Some(match job.code {
            Some(worst) if severity(worst) >= severity(code) => worst,
            _ => code,
        });
        job.outstanding = job.outstanding.saturating_sub(1);
        if job.outstanding > 0 {
            return;
        }
        let worst = job.code.unwrap_or(ResultCode::Ok);
        if worst.is_ok() && job.stage + 1 < job.stages.len() {
            job.stage += 1;
            job.code = None;
            self.start_stage(id, now_ms);
            return;
        }
        self.finish_job(id, outcome_of(worst), now_ms);
    }

    /// Take a job off the list, queue its outcome and apply what it
    /// means for the intended-OFF set.
    fn finish_job(&mut self, id: JobId, outcome: JobOutcome, now_ms: u64) {
        let Some(at) = self.jobs.iter().position(|j| j.id == id) else {
            return;
        };
        let job = self.jobs.remove(at);
        // Nothing of it may be on a plane any more.
        for state in &mut self.planes {
            state.in_flight.retain(|p| p.job != id);
            state.queue.retain(|q| q.job != id);
        }
        let code = job.code;
        self.note_intent(&job.command, outcome, now_ms);
        self.events
            .push_back(ClientEvent::JobFinished { id, outcome, code });
        self.finished.push_back(FinishedJob {
            id,
            command: job.command,
            outcome,
            code,
            submitted_ms: job.submitted_ms,
            finished_ms: now_ms,
        });
    }

    /// Give up on jobs that have run too long.
    fn expire_jobs(&mut self, now_ms: u64) {
        let late: Vec<JobId> = self
            .jobs
            .iter()
            .filter(|j| now_ms.saturating_sub(j.submitted_ms) > JOB_TIMEOUT_MS)
            .map(|j| j.id)
            .collect();
        for id in late {
            self.finish_job(id, JobOutcome::TimedOut, now_ms);
        }
    }

    // --- intended OFF -------------------------------------------------

    /// What a finished job means for the intended-OFF set.
    fn note_intent(&mut self, command: &Command, outcome: JobOutcome, now_ms: u64) {
        let Command::Node(node_command) = command else {
            return;
        };
        if !outcome.is_ok() {
            return;
        }
        if node_command.intends_rail_on() {
            for node in node_command.nodes() {
                self.intent.remove(node);
            }
        } else if node_command.intends_rail_off() {
            for node in node_command.nodes() {
                self.intent.insert(node);
            }
        } else if node_command.may_cut_rail() {
            // Whether the rail really went down is the plane's answer,
            // not the verb's: wait for the next SUMMARY.
            for node in node_command.nodes() {
                let Some((address, slot)) = node_location(node) else {
                    continue;
                };
                if let Some(n) = granite_proto::plane_of_address(address) {
                    self.planes[n as usize].pending_intent.push(PendingIntent {
                        slot,
                        until_ms: now_ms + INTENT_SETTLE_MS,
                    });
                }
            }
        }
    }

    /// A fresh SUMMARY has arrived: decide the `off` verbs that were
    /// waiting for it.
    fn resolve_intents(&mut self, i: usize) {
        let pending = core::mem::take(&mut self.planes[i].pending_intent);
        for entry in pending {
            let Some(node) = self.planes[i].plane.node(entry.slot) else {
                continue;
            };
            let (id, off) = (
                node.id,
                !node.status.contains(granite_proto::SlotStatus::ENABLED),
            );
            if off {
                self.intent.insert(id);
            }
        }
    }

    /// Drop the intent decisions nobody answered in time.
    fn expire_intents(&mut self, now_ms: u64) {
        for state in &mut self.planes {
            state.pending_intent.retain(|p| now_ms <= p.until_ms);
        }
    }

    /// The nodes the controller last commanded rail-off. The firmware
    /// persists this (ADR 0004: kept in NVS).
    pub fn intended_off(&self) -> &[NodeId] {
        self.intent.nodes()
    }

    /// Put back a persisted intended-OFF set, at boot.
    pub fn load_intended_off(&mut self, nodes: &[NodeId]) {
        self.intent.load(nodes);
    }

    /// True when the intended-OFF set has changed since the firmware
    /// last stored it; clears the flag.
    pub fn take_intent_dirty(&mut self) -> bool {
        self.intent.take_dirty()
    }

    // --- config -------------------------------------------------------

    /// Hold a desired config for a plane. It is written as soon as the
    /// plane's hash says it does not have it.
    pub fn set_plane_config(&mut self, address: u8, config: DesiredConfig) {
        if let Some(n) = granite_proto::plane_of_address(address) {
            self.planes[n as usize].config = Some(config);
            self.planes[n as usize].config_written = false;
        }
    }

    /// The desired config of a plane.
    pub fn plane_config(&self, address: u8) -> Option<&DesiredConfig> {
        granite_proto::plane_of_address(address)
            .and_then(|n| self.planes[n as usize].config.as_ref())
    }

    /// True when a CONFIG_W write is due for this plane.
    fn config_push_due(&self, i: usize) -> bool {
        let state = &self.planes[i];
        if state.config_written || state.plane.info_ms == 0 {
            return false;
        }
        state
            .config
            .as_ref()
            .is_some_and(|c| c.needs_push(state.plane.config_hash))
    }

    /// Write the desired config as one CONFIG_W set, commit last.
    fn push_config<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L) {
        let address = self.planes[i].plane.address;
        let Some(config) = self.planes[i].config.as_mut() else {
            return;
        };
        let set = config.write_set();
        config.note_push();
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        for record in &set {
            if record.encode(&mut w).is_err() {
                return;
            }
        }
        let payload = w.payload().to_vec();
        match self.tx.write(link, address, BLK_CONFIG_W, &payload) {
            Ok(()) => {
                // The block has no sequence number and no result
                // (ADR 0003): the hash in INFO is the answer.
                self.planes[i].config_written = true;
                self.planes[i].info_due = true;
            }
            Err(_) => self.after_failure(i, now_ms),
        }
    }

    // --- firmware updates ---------------------------------------------

    /// The update policy (ADR 0004 decision 8).
    pub const fn policy(&self) -> UpdatePolicy {
        self.policy
    }

    /// Set the update policy.
    pub fn set_policy(&mut self, policy: UpdatePolicy) {
        self.policy = policy;
    }

    /// The planes whose firmware is not `version`: what an `auto`
    /// policy offers an image to, one at a time.
    pub fn update_candidates(&self, version: Version) -> Vec<u8> {
        self.planes
            .iter()
            .filter(|s| s.present && s.plane.firmware != version)
            .map(|s| s.plane.address)
            .collect()
    }

    /// Start a firmware update on one plane.
    ///
    /// One plane at a time, never while a node sequence runs on it and
    /// never with commands of its own in flight (ADR 0004 decision 8).
    pub fn start_update(
        &mut self,
        now_ms: u64,
        address: u8,
        image: Box<dyn ImageSource>,
    ) -> Result<(), UpdateRefused> {
        if self.update.is_some() {
            return Err(UpdateRefused::Busy);
        }
        let Some(n) = granite_proto::plane_of_address(address) else {
            return Err(UpdateRefused::PlaneAbsent);
        };
        let state = &self.planes[n as usize];
        if !state.present {
            return Err(UpdateRefused::PlaneAbsent);
        }
        if image.size() == 0 {
            return Err(UpdateRefused::BadImage);
        }
        if state.plane.sequence_running() {
            return Err(UpdateRefused::SequenceRunning);
        }
        if !state.in_flight.is_empty() || !state.queue.is_empty() {
            return Err(UpdateRefused::CommandsInFlight);
        }
        if state.plane.info_ms != 0
            && !state
                .plane
                .capabilities
                .contains(granite_proto::Capabilities::FIRMWARE_UPDATE)
        {
            return Err(UpdateRefused::NotCapable);
        }
        self.update = Some(UpdateJob::new(now_ms, address, &state.plane, image));
        Ok(())
    }

    /// Stop the running update. The plane is left with an unconfirmed
    /// or aborted image, which rolls itself back.
    pub fn abort_update<L: PlaneLink>(&mut self, now_ms: u64, link: &mut L) {
        let Some(job) = self.update.take() else {
            return;
        };
        job.abort(&mut self.tx, link);
        self.events.push_back(ClientEvent::UpdateFailed {
            address: job.address,
            version: job.version,
            reason: UpdateFailure::Aborted,
            image_state: None,
        });
        let _ = now_ms;
    }

    /// Where the running update is, if one runs.
    pub fn update_stage(&self) -> Option<(u8, UpdateStage, u32)> {
        self.update
            .as_ref()
            .map(|j| (j.address, j.stage, j.percent()))
    }

    /// One step of the running update.
    fn step_update<L: PlaneLink>(&mut self, now_ms: u64, link: &mut L, budget: &mut u32) {
        if self.update.is_none() || *budget == 0 {
            return;
        }
        // The job holds the image, so it comes out of the client for
        // the step and goes back in unless it ended.
        let mut job = self.update.take().expect("checked");
        let address = job.address;
        // The step may spend several transactions of its own; one
        // budget unit covers the call, which is how the transfer gets
        // its pace from the poll cadence.
        *budget -= 1;
        let planes = &mut self.planes;
        let end = {
            // A sequence number is only taken when the reboot really
            // goes out: a counter that ran on every poll would come
            // round to a number the plane still remembers.
            let mut seq = || match granite_proto::plane_of_address(address) {
                Some(n) => planes[n as usize].next_seq(),
                None => 0,
            };
            job.step(now_ms, &mut self.tx, link, &mut seq)
        };
        match end {
            None => self.update = Some(job),
            Some(UpdateEnd::Confirmed) => {
                self.events.push_back(ClientEvent::UpdateConfirmed {
                    address,
                    version: job.version,
                });
                if let Some(n) = granite_proto::plane_of_address(address) {
                    self.planes[n as usize].info_due = true;
                }
            }
            Some(UpdateEnd::Failed(reason)) => {
                job.abort(&mut self.tx, link);
                let image_state = granite_proto::plane_of_address(address)
                    .map(|n| self.planes[n as usize].plane.image_state);
                self.events.push_back(ClientEvent::UpdateFailed {
                    address,
                    version: job.version,
                    reason,
                    image_state,
                });
                if let Some(n) = granite_proto::plane_of_address(address) {
                    self.planes[n as usize].info_due = true;
                }
            }
        }
    }

    // --- what a reader sees -------------------------------------------

    /// Every plane that answers, lowest address first.
    pub fn planes(&self) -> impl Iterator<Item = &Plane> {
        self.planes.iter().filter(|s| s.present).map(|s| &s.plane)
    }

    /// How many planes answer.
    pub fn present_count(&self) -> usize {
        self.planes.iter().filter(|s| s.present).count()
    }

    /// The addresses that answer.
    pub fn addresses(&self) -> Vec<u8> {
        self.planes().map(|p| p.address).collect()
    }

    /// One plane, if it answers.
    pub fn plane(&self, address: u8) -> Option<&Plane> {
        let n = granite_proto::plane_of_address(address)?;
        let state = &self.planes[n as usize];
        state.present.then_some(&state.plane)
    }

    /// Every node of every answering plane whose unit is on the chain
    /// and whose slot holds a module, by node id.
    ///
    /// These are the nodes a state payload lists (ADR 0004
    /// decision 2: nodes that are not present are not listed).
    pub fn nodes(&self) -> impl Iterator<Item = &super::Node> {
        self.planes().flat_map(|p| p.present_nodes())
    }

    /// One node by id, present or not, as long as its plane answers.
    pub fn node(&self, id: NodeId) -> Option<&super::Node> {
        let (address, slot) = node_location(id)?;
        self.plane(address)?.node(slot)
    }

    /// What the client counted on one address.
    pub fn errors(&self, address: u8) -> PlaneErrors {
        self.tx.errors(address)
    }

    /// True when two planes may be strapped to one address
    /// (ADR 0003: the straps have to be fixed).
    pub fn collision_suspected(&self, address: u8) -> bool {
        self.tx.errors(address).collision_suspected
    }

    /// Transactions the client has done: reads, then writes.
    pub const fn transaction_counts(&self) -> (u32, u32) {
        (self.tx.reads, self.tx.writes)
    }

    /// How many jobs are running.
    pub fn jobs_running(&self) -> usize {
        self.jobs.len()
    }

    /// How many CTRL writes are in flight on one plane.
    pub fn in_flight(&self, address: u8) -> usize {
        granite_proto::plane_of_address(address)
            .map(|n| self.planes[n as usize].in_flight.len())
            .unwrap_or(0)
    }

    /// Take the next event, oldest first.
    pub fn next_event(&mut self) -> Option<ClientEvent> {
        self.events.pop_front()
    }

    /// Take every event.
    pub fn drain_events(&mut self) -> Vec<ClientEvent> {
        self.events.drain(..).collect()
    }

    /// Take the next finished job, oldest first.
    pub fn next_finished_job(&mut self) -> Option<FinishedJob> {
        self.finished.pop_front()
    }

    /// Take every finished job: the MQTT acks and HTTP job results of
    /// ADR 0004 decision 6.
    pub fn drain_jobs(&mut self) -> Vec<FinishedJob> {
        self.finished.drain(..).collect()
    }
}

/// Take one transaction off a poll's budget.
fn spend(budget: &mut u32) -> bool {
    if *budget == 0 {
        return false;
    }
    *budget -= 1;
    true
}

/// Where plane `n`'s 1 Hz poll sits inside the second \[ms\], so eight
/// planes do not all want their STATUS at once.
const fn status_phase(n: usize) -> u64 {
    (n as u64 * STATUS_PERIOD_MS) / MAX_PLANES as u64
}

/// A sequence-number start for plane `n` from the client's seed.
///
/// ADR 0003's expert answer: the counter starts at a random value so a
/// controller that reboots mid-command cannot collide with the
/// sequence memory the plane still holds. This is the usual
/// xorshift-multiply mix, enough to spread a single seed over eight
/// planes.
fn seq_start(seed: u32, n: u8) -> u8 {
    let mut x = seed ^ 0x9E37_79B9u32.wrapping_mul(n as u32 + 1);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x & 0xFF) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_starts_are_spread_over_the_planes() {
        let starts: Vec<u8> = (0..MAX_PLANES as u8)
            .map(|n| seq_start(0x1234_5678, n))
            .collect();
        let mut sorted = starts.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert!(sorted.len() >= 6, "the seed spreads: {starts:?}");
        assert_ne!(
            seq_start(1, 0),
            seq_start(2, 0),
            "a different seed starts somewhere else"
        );
    }

    #[test]
    fn the_one_hz_polls_are_phased() {
        assert_eq!(status_phase(0), 0);
        assert_eq!(status_phase(4), 500);
        assert!(status_phase(7) < STATUS_PERIOD_MS);
    }

    #[test]
    fn a_command_to_an_unknown_plane_fails_at_once() {
        let mut client = PlaneClient::new(7);
        let id = client.submit(100, NodeCommand::on(1));
        let finished = client.drain_jobs();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].id, id);
        assert_eq!(finished[0].outcome, JobOutcome::Absent);
        assert!(client.jobs_running() == 0);
        // And a node id that is not one at all.
        client.submit(100, NodeCommand::on(200));
        assert_eq!(client.drain_jobs()[0].outcome, JobOutcome::Absent);
    }
}
