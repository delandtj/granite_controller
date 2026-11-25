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
//! - **events**: [`PlaneLink::event_nodes`] says which planes raised
//!   events since the last poll (ADR 0005 decision 6), and their STATUS
//!   comes forward; on an I2C link the mask is every plane while EXP_INT
//!   is low, because that line is the wired OR of every plane's. The
//!   `events pending` flag in STATUS says which ones to read EVENTS
//!   from, and EV_ACK goes out once the events are in the client's
//!   queue.
//! - **SYNC**: once a second the controller's keepalive goes out with
//!   its epoch (ADR 0005 decision 7). A CAN link sends the frame; an
//!   I2C link has nowhere to put it and does nothing.
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
    BLK_CONFIG_W, BLK_CTRL, BLK_EV_ACK, BLK_EVENTS, BLK_INFO, BLK_LOG, BLK_SLOT, BLK_STATUS,
    BLK_SUMMARY, BLK_UNIT, Command as Ctrl, EventKind, EventRecord, EventTarget, INDEX_NONE,
    IndexBitmap, MAX_PAYLOAD, MAX_PLANES, RecordWriter, ResultCode, SEQ_MEMORY, SLOT_COUNT, Status,
    UNIT_COUNT, Version, frame,
};

use crate::NodeId;
use crate::hal::PlaneLink;

use super::command::{
    Command, DEFAULT_HOST_RUNNING_DW, FinishedJob, HOST_RUNNING_CONFIRM_MS, JobId, JobOutcome,
    NodeCommand, STAGE_WATCH_MS, SlotLatch, StageCondition, Step, outcome_of, severity,
};
use super::config::DesiredConfig;
use super::event::ClientEvent;
use super::intent::{INTENT_SETTLE_MS, IntendedOff};
use super::link::{PlaneErrors, Transactions};
use super::log::{PlaneLogLine, PlaneLogState};
use super::model::Plane;
use super::update::{ImageSource, UpdateEnd, UpdateFailure, UpdateJob, UpdateRefused, UpdateStage};
use super::{MAX_IN_FLIGHT, node_id, node_location};

/// How often absent addresses are scanned again \[ms\].
pub const SCAN_PERIOD_MS: u64 = 10_000;

/// The 1 Hz poll of ADR 0003 decision 5: STATUS and SUMMARY \[ms\].
pub const STATUS_PERIOD_MS: u64 = 1_000;

/// The slow sweep of the SLOT and UNIT blocks \[ms\].
pub const DETAIL_PERIOD_MS: u64 = 10_000;

/// How often the controller's SYNC goes out \[ms\] (ADR 0005
/// decision 7).
pub const SYNC_PERIOD_MS: u64 = 1_000;

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

/// What a job gets on top of the longest its own command can take
/// \[ms\] before the client gives up on it.
///
/// The deadline of a job is [`job_timeout_ms`]: what the command's
/// options allow (a soft off waits the node's whole `t_soft_off_s`,
/// which may be an hour) plus this margin for the link's resends and
/// the plane's command queue. It only catches a plane that answers but
/// never finishes - one that goes away fails its jobs at once - so the
/// margin is generous and the deadline is never shorter than the
/// sequence it covers.
pub const JOB_MARGIN_MS: u64 = 60_000;

/// How long a job whose command is `command` may run \[ms\].
pub fn job_timeout_ms(command: &Command) -> u64 {
    command.worst_case_ms() + JOB_MARGIN_MS
}

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

/// A stage that is waiting for the slot to say whether it is needed
/// (ADR 0004, "`force_off` always ends with the rail down").
///
/// The cut stage of `off` and `force_off` asks a question about the
/// slot, and SUMMARY answers it once a second. The watch holds the
/// question and what has been seen so far; [`PlaneClient::read_summary`]
/// feeds it and it ends one of three ways: the writes are queued, the
/// job finishes without them, or it runs out of time
/// ([`StageCondition::cuts_when_unreadable`] decides which of the two
/// that is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StageWatch {
    condition: StageCondition,
    /// The plane and slot the question is about.
    address: u8,
    slot: u8,
    /// A SUMMARY has to be newer than this to count: anything older was
    /// composed while the stage before this one was still running.
    after_ms: u64,
    /// When the watch gives up \[ms\].
    until_ms: u64,
    /// Since when every fresh reading has met the condition \[ms\]:
    /// where [`HOST_RUNNING_CONFIRM_MS`] is counted from.
    above_since_ms: Option<u64>,
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
    /// When the client gives up on it \[ms\]: the submission plus
    /// [`job_timeout_ms`] of its own command.
    deadline_ms: u64,
    /// The current stage is waiting out a delay of its own and goes out
    /// at this time \[ms\] (see [`Command::stage_delay_ms`]): the rail
    /// cycle of a node with no front panel is the only one.
    hold_until_ms: Option<u64>,
    /// The current stage is waiting for the slot to say whether it is
    /// needed at all (see [`StageWatch`]).
    watch: Option<StageWatch>,
    /// The controller sent the cut stage: the rail came down from here
    /// and not from the plane's own escalation.
    rail_cut: bool,
    /// The latch the slot reported after a rail-on stage, which is what
    /// turns a plane's `ok` into a failed job
    /// ([`StageCondition::SlotUnlatched`]).
    latched: Option<SlotLatch>,
}

/// Everything the client knows and schedules for one address.
#[derive(Debug, Clone)]
struct PlaneState {
    plane: Plane,
    /// A plane answers here.
    present: bool,
    /// A plane answered here and stopped: the controller is missing one
    /// it had found. Cleared when the address answers again or when the
    /// plane is forgotten; a fresh client has nothing to miss.
    lost: bool,
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
    /// Per slot, the SUMMARY timestamp at which the intended-OFF
    /// re-apply last wrote: the next attempt waits for a fresher one,
    /// so one lost state is answered once per slot report and not once
    /// per STATUS read.
    reapplied_after: [u64; SLOT_COUNT],
    pending_intent: Vec<PendingIntent>,
    /// The collision flag has been reported once.
    collision_reported: bool,
    /// The plane's LOG block: the cache, the rate limit and what has
    /// been reported (see [`super::log`]).
    log: PlaneLogState,
}

impl PlaneState {
    fn new(address: u8, seq: u8) -> Self {
        PlaneState {
            plane: Plane::new(address),
            present: false,
            lost: false,
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
            reapplied_after: [0; SLOT_COUNT],
            pending_intent: Vec::new(),
            collision_reported: false,
            log: PlaneLogState::new(),
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
///
/// The two big members are boxed, and are built on the heap rather than
/// moved there: this value is constructed inside the dispatcher thread's
/// frame, `lto = true` inlines the constructor into it, and a 7.8 KB
/// array of plane states then exists twice in one frame - which overflowed
/// a 12 KB stack on a rev C board, and still used 83 % of 24 KB
/// (measured 2026-11-04). With them boxed `size_of::<PlaneClient>()` goes
/// from 8512 to 464 bytes on the host, and the frame no longer scales with
/// `MAX_PLANES`. Nothing else in here is over 512 bytes: `Transactions` is
/// the next largest at 232.
pub struct PlaneClient {
    planes: Box<[PlaneState; MAX_PLANES]>,
    tx: Transactions,
    events: VecDeque<ClientEvent>,
    finished: VecDeque<FinishedJob>,
    jobs: Vec<Job>,
    next_job: JobId,
    next_scan_ms: u64,
    scan_due: bool,
    /// The epoch of the next SYNC (ADR 0005 decision 7): random at
    /// boot, one more on every send.
    epoch: u32,
    /// When the next SYNC is due \[ms\].
    next_sync_ms: u64,
    /// Which plane is served first this poll.
    cursor: usize,
    intent: IntendedOff,
    update: Option<UpdateJob>,
    policy: UpdatePolicy,
    /// `nodes.host_running_w` \[0.1 W\]: the draw above which a dark
    /// slot whose rail is still up is taken to hold a running host, so
    /// `off` cuts it (ADR 0004, "`force_off` always ends with the rail
    /// down"). `0` turns the check off.
    host_running_dw: u16,
    /// Scratch for one answer; no read allocates. Boxed with `planes`,
    /// for the same reason.
    buf: Box<[u8; frame::MAX_ANSWER]>,
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
    /// its RNG; a test passes a constant. The SYNC epoch of ADR 0005
    /// decision 7 starts there too, for the same reason: a standby
    /// controller must be able to tell a restarted keepalive from the
    /// one it was watching.
    ///
    /// `planes` and `buf` are grown on the heap, element by element,
    /// instead of being built as an array and moved: an array literal or
    /// `core::array::from_fn` would put all 7.8 KB of plane state in the
    /// caller's frame first (see the type's own note).
    pub fn new(seed: u32) -> Self {
        let planes: Vec<PlaneState> = (0..MAX_PLANES)
            .map(|n| PlaneState::new(granite_proto::ADDR_BASE + n as u8, seq_start(seed, n as u8)))
            .collect();
        let planes: Box<[PlaneState; MAX_PLANES]> = planes
            .into_boxed_slice()
            .try_into()
            .expect("MAX_PLANES plane states");
        let buf: Box<[u8; frame::MAX_ANSWER]> = alloc::vec![0u8; frame::MAX_ANSWER]
            .into_boxed_slice()
            .try_into()
            .expect("MAX_ANSWER bytes");
        PlaneClient {
            planes,
            tx: Transactions::new(),
            events: VecDeque::new(),
            finished: VecDeque::new(),
            jobs: Vec::new(),
            next_job: 1,
            next_scan_ms: 0,
            scan_due: true,
            epoch: epoch_start(seed),
            next_sync_ms: 0,
            cursor: 0,
            intent: IntendedOff::new(),
            update: None,
            policy: UpdatePolicy::Manual,
            host_running_dw: DEFAULT_HOST_RUNNING_DW,
            buf,
        }
    }

    // --- the poll -----------------------------------------------------

    /// Do whatever is due. Called often; never blocks.
    pub fn poll<L: PlaneLink>(&mut self, now_ms: u64, link: &mut L) {
        let mut budget = MAX_TRANSACTIONS_PER_POLL;

        self.send_sync(now_ms, link);

        // The planes in the mask raised events, so their STATUS comes
        // forward (ADR 0003 decision 5, ADR 0005 decision 6). An I2C
        // link cannot tell which plane pulled EXP_INT and answers with
        // every one of them.
        let raised = link.event_nodes();
        if raised != 0 {
            for (n, state) in self.planes.iter_mut().enumerate() {
                if raised & (1 << n) != 0 && state.present {
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

        // A stage that was waiting out a delay of its own goes out in
        // this poll, before the planes are served.
        self.release_stages(now_ms);

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
        self.expire_stage_watches(now_ms);
        self.expire_jobs(now_ms);
        self.expire_intents(now_ms);
    }

    /// The keepalive of ADR 0005 decision 7: SYNC once per second, with
    /// the epoch one higher on every send.
    ///
    /// It costs no transaction budget - it is one frame nobody answers -
    /// and a link that refuses it is not an error worth reporting:
    /// nothing retries a keepalive, the next second brings another one.
    /// The epoch moves whether or not the frame got out, so a standby
    /// controller counting them sees the gap.
    fn send_sync<L: PlaneLink>(&mut self, now_ms: u64, link: &mut L) {
        if now_ms < self.next_sync_ms {
            return;
        }
        self.next_sync_ms = now_ms.saturating_add(SYNC_PERIOD_MS);
        let _ = link.sync(self.epoch);
        self.epoch = self.epoch.wrapping_add(1);
    }

    /// The epoch the next SYNC will carry.
    pub const fn sync_epoch(&self) -> u32 {
        self.epoch
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
                .read(link, address, BLK_INFO, INDEX_NONE, &mut self.buf[..])
            {
                Ok(n) => {
                    let payload_len = n;
                    self.planes[i].present = true;
                    // It answers again: nothing is missing here.
                    self.planes[i].lost = false;
                    self.planes[i]
                        .plane
                        .apply_info(now_ms, &self.buf[..payload_len]);
                    let state = &mut self.planes[i];
                    state.next_status_ms = now_ms + status_phase(i);
                    state.next_detail_ms = now_ms;
                    state.next_info_ms = now_ms + INFO_PERIOD_MS;
                    state.info_due = false;
                    state.config_written = false;
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

        // Only when something asked for it, and at most once every
        // `log::READ_PERIOD_MS`: the LOG block is never on a schedule of
        // its own, so it cannot displace the 1 Hz poll.
        if self.planes[i].log.is_due(now_ms) && spend(budget) {
            self.read_log(i, now_ms, link);
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
            .read(link, address, BLK_INFO, INDEX_NONE, &mut self.buf[..])
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
                    // A plane that restarted has a new ring, and what it
                    // wrote on the way up says why it restarted.
                    state.log.request();
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
            .read(link, address, BLK_STATUS, INDEX_NONE, &mut self.buf[..])
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
    ///
    /// A plane that lost its core power says so until a reset that keeps
    /// VDD, and it comes back with its boot count at 1 - the counter
    /// lives in the backup domain that went with it (ADR 0002
    /// decision 3) - so neither the flag's edge nor the boot count can
    /// say whether this loss has been answered. What can is the state
    /// itself: while the plane flags the loss, every node the controller
    /// last commanded rail-off whose rail is up again gets its OFF
    /// re-applied (ADR 0004, "What an Expert Would Ask"). That is
    /// self-limiting - the slot goes off and stops matching - and a verb
    /// that wants the rail up takes the node out of the set, so a user
    /// who turns a node on is never fought over.
    fn after_status(&mut self, i: usize, now_ms: u64) {
        let state = &self.planes[i];
        if !state.plane.intended_state_lost() || state.plane.info_ms == 0 {
            return;
        }
        let address = state.plane.address;
        let nodes: Vec<NodeId> = self
            .intent
            .of_plane(address)
            .into_iter()
            .filter(|node| self.reapply_due(*node))
            .collect();
        if nodes.is_empty() {
            return;
        }
        for node in &nodes {
            if let Some((_, slot)) = node_location(*node) {
                let seen = self.planes[i]
                    .plane
                    .node(slot)
                    .map(|n| n.summary_ms)
                    .unwrap_or(0);
                self.planes[i].reapplied_after[slot as usize] = seen;
            }
            self.submit(now_ms, NodeCommand::PowerOff { node: *node });
        }
        self.events
            .push_back(ClientEvent::IntendedOffReapplied { address, nodes });
    }

    /// True when a node the controller wants off needs its OFF put
    /// back: the plane reports its rail enabled, a SUMMARY newer than
    /// the last attempt says so, and no job of ours is already running
    /// on it.
    fn reapply_due(&self, node: NodeId) -> bool {
        let Some((address, slot)) = node_location(node) else {
            return false;
        };
        let Some(n) = granite_proto::plane_of_address(address) else {
            return false;
        };
        let state = &self.planes[n as usize];
        let fresh = state.plane.node(slot).is_some_and(|node| {
            node.summary_ms != 0
                && node.summary_ms > state.reapplied_after[slot as usize]
                && node.status.contains(granite_proto::SlotStatus::ENABLED)
        });
        fresh && !self.has_job_for(node)
    }

    /// True while a job of this client is running on a node: a re-apply
    /// never queues a second OFF behind the first.
    fn has_job_for(&self, node: NodeId) -> bool {
        self.jobs
            .iter()
            .any(|job| job.command.nodes().contains(&node))
    }

    /// Read SUMMARY.
    fn read_summary<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L) {
        let address = self.planes[i].plane.address;
        match self
            .tx
            .read(link, address, BLK_SUMMARY, INDEX_NONE, &mut self.buf[..])
        {
            Ok(n) => {
                let changed = self.planes[i].plane.apply_summary(now_ms, &self.buf[..n]);
                for slot in 0..SLOT_COUNT as u8 {
                    if changed.has(slot) {
                        self.planes[i].dirty_slot(slot);
                    }
                }
                self.resolve_intents(i);
                self.check_stage_watches(i, now_ms);
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
            match self.tx.read(link, address, block, index, &mut self.buf[..]) {
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
            .read(link, address, BLK_EVENTS, INDEX_NONE, &mut self.buf[..])
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
        let mut log_due = false;
        let mut retry_limits: Vec<u8> = Vec::new();
        for record in EventRecord::records(&self.buf[..n]).flatten() {
            match record {
                EventRecord::Event(event) => {
                    // What the plane says went wrong is worth the lines
                    // it wrote about it (ADR 0004, "Plane log readout").
                    log_due |= super::log::read_after(event.kind);
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
        if log_due {
            self.planes[i].log.request();
        }
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

    // --- the log ring -------------------------------------------------

    /// Read the LOG block and queue the lines that have not been
    /// reported before (ADR 0004, "Plane log readout").
    ///
    /// The read counts against the rate limit whether or not the
    /// transaction worked, so a plane that is falling off the link is
    /// not asked again on the next poll.
    fn read_log<L: PlaneLink>(&mut self, i: usize, now_ms: u64, link: &mut L) -> bool {
        let address = self.planes[i].plane.address;
        self.planes[i].log.note_read(now_ms);
        let n = match self
            .tx
            .read(link, address, BLK_LOG, INDEX_NONE, &mut self.buf[..])
        {
            Ok(n) => n,
            Err(_) => {
                self.after_failure(i, now_ms);
                return false;
            }
        };
        let lines = super::log::decode_block(&self.buf[..n]);
        for line in self.planes[i].log.accept(lines) {
            self.events.push_back(ClientEvent::PlaneLog {
                address,
                level: line.level,
                text: line.text,
                uptime_ms: line.uptime_ms,
            });
        }
        true
    }

    /// Read a plane's LOG block now, on the caller's link, and say
    /// whether the plane answered (ADR 0004, "As built": `plane_log`
    /// answers from the fresh read, not from the cache).
    ///
    /// This is the one log read that is not on the poll's schedule and
    /// not rate-limited: an operator asking for the ring by hand is not
    /// a fault storm, and the whole point of the command is the lines
    /// the plane has written since the last read. It still counts as a
    /// read, so the automatic ones that follow keep their
    /// [`super::log::READ_PERIOD_MS`] spacing. The lines it brings are
    /// reported as [`ClientEvent::PlaneLog`] exactly as the scheduled
    /// read's are, so nothing is logged twice, and
    /// [`PlaneClient::plane_log`] holds them when this returns.
    ///
    /// False when the address has no plane or the read failed; the
    /// caller then answers from the cache, marked stale.
    pub fn read_plane_log_now<L: PlaneLink>(
        &mut self,
        address: u8,
        now_ms: u64,
        link: &mut L,
    ) -> bool {
        match granite_proto::plane_of_address(address) {
            Some(n) if self.planes[n as usize].present => {
                self.read_log(n as usize, now_ms, link)
            }
            _ => false,
        }
    }

    /// Ask for a plane's LOG block: the on-demand read of ADR 0004
    /// ("Plane log readout"), which the `plane_log` command and
    /// `GET /api/v1/planes/<n>/log` use.
    ///
    /// The read happens on a following poll, at most one per plane per
    /// [`super::log::READ_PERIOD_MS`]; the lines it brings arrive as
    /// [`ClientEvent::PlaneLog`] and in
    /// [`PlaneClient::plane_log`]. False when no plane answers at that
    /// address.
    pub fn request_plane_log(&mut self, address: u8) -> bool {
        match granite_proto::plane_of_address(address) {
            Some(n) if self.planes[n as usize].present => {
                self.planes[n as usize].log.request();
                true
            }
            _ => false,
        }
    }

    /// The log lines the client holds for a plane, oldest first.
    ///
    /// What the last read brought, which is the plane's whole ring;
    /// empty while nothing has been read.
    pub fn plane_log(&self, address: u8) -> &[PlaneLogLine] {
        match granite_proto::plane_of_address(address) {
            Some(n) => self.planes[n as usize].log.lines(),
            None => &[],
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
        // A plane the controller had found is missing: fault bit 0
        // stays up until it answers again (ADR 0004, Modbus map v2).
        self.planes[i].lost = true;
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
        // A job whose cut stage is waiting for this plane's telemetry
        // has nothing left to wait for: no SUMMARY will come, and a
        // write would sit in a queue nobody serves.
        for job in &self.jobs {
            if job.watch.is_some_and(|w| w.address == address) && !hit.contains(&job.id) {
                hit.push(job.id);
            }
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
        // The deadline follows the command: a node whose soft-off
        // window is an hour gets its hour.
        let deadline_ms = now_ms.saturating_add(job_timeout_ms(&command));
        let job = Job {
            id,
            command,
            stages,
            stage: 0,
            outstanding: 0,
            code: None,
            submitted_ms: now_ms,
            deadline_ms,
            hold_until_ms: None,
            watch: None,
            rail_cut: false,
            latched: None,
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

    /// Queue every write of a job's current stage, or hold it back when
    /// the stage asks to wait first.
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
        // The rail cycle of a node with no front panel: the rail stays
        // down for the cycle's off delay before the second write goes
        // out (ADR 0004, "Nodes without a front panel").
        let delay = job.command.stage_delay_ms(stage);
        if delay > 0 {
            job.outstanding = 0;
            job.hold_until_ms = Some(now_ms.saturating_add(delay));
            return;
        }
        // A cut stage asks the slot whether it is needed. The answer is
        // in a SUMMARY newer than this moment, so the stage waits for
        // one (ADR 0004, "`force_off` always ends with the rail down").
        let condition = job.command.stage_condition(stage);
        if !condition.is_always() {
            let node = job.command.nodes().first().copied().unwrap_or(0);
            match node_location(node) {
                Some((address, slot)) => {
                    let window = match condition {
                        StageCondition::HostStillDrawing => {
                            HOST_RUNNING_CONFIRM_MS + STAGE_WATCH_MS
                        }
                        _ => STAGE_WATCH_MS,
                    };
                    job.outstanding = 0;
                    job.watch = Some(StageWatch {
                        condition,
                        address,
                        slot,
                        after_ms: now_ms,
                        until_ms: now_ms.saturating_add(window),
                        above_since_ms: None,
                    });
                    return;
                }
                // Not a node id: there is nothing to look at and
                // nothing to cut.
                None => {
                    let code = job.code;
                    let outcome = code.map_or(JobOutcome::Done, outcome_of);
                    self.finish_job(id, outcome, now_ms);
                    return;
                }
            }
        }
        self.queue_stage(id, stage, steps);
    }

    /// Send the cut stage a watch decided on, and remember that the
    /// rail came down from here.
    fn cut_stage(&mut self, id: JobId, now_ms: u64) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        job.watch = None;
        job.rail_cut = true;
        let stage = job.stage;
        let Some(steps) = job.stages.get(stage).cloned() else {
            self.finish_job(id, JobOutcome::Done, now_ms);
            return;
        };
        self.queue_stage(id, stage, steps);
    }

    /// End a job whose cut stage turned out not to be needed: the rail
    /// is already down, or the host really did shut down.
    fn skip_cut_stage(&mut self, id: JobId, now_ms: u64) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        job.watch = None;
        // The outcome is the one the plane's own sequence earned: the
        // stage that was skipped never ran, so it cannot change it.
        let outcome = job.code.map_or(JobOutcome::Done, outcome_of);
        self.finish_job(id, outcome, now_ms);
    }

    /// A fresh SUMMARY has arrived on plane `i`: let the cut stages
    /// waiting on one of its slots decide.
    ///
    /// Three outcomes per watch, in the order they are tested: the
    /// condition is met and the cut goes out, a reading says the cut is
    /// not needed and the job ends, or nothing is decided yet and the
    /// next SUMMARY is awaited (until the watch expires, see
    /// [`PlaneClient::expire_stage_watches`]).
    fn check_stage_watches(&mut self, i: usize, now_ms: u64) {
        let address = self.planes[i].plane.address;
        let threshold = self.host_running_dw;
        let mut cut: Vec<JobId> = Vec::new();
        let mut skip: Vec<JobId> = Vec::new();
        let mut latched: Vec<(JobId, Option<SlotLatch>)> = Vec::new();
        for job in self.jobs.iter_mut() {
            let Some(watch) = job.watch.as_mut() else {
                continue;
            };
            if watch.address != address {
                continue;
            }
            let Some(node) = self.planes[i].plane.node(watch.slot) else {
                continue;
            };
            // Anything the plane composed before the sequence ended
            // says nothing about how it ended.
            if node.summary_ms == 0 || node.summary_ms <= watch.after_ms {
                continue;
            }
            let enabled = node.status.contains(granite_proto::SlotStatus::ENABLED);
            match watch.condition {
                StageCondition::Always => {}
                // The plane cuts the rail itself when the host LED is
                // still lit after the hold, so a slot that is already
                // off needs nothing from us.
                StageCondition::RailStillUp => {
                    if enabled {
                        cut.push(job.id);
                    } else {
                        skip.push(job.id);
                    }
                }
                // A dark LED on a rail that keeps drawing is a host
                // behind a front panel the plane cannot see. Every
                // reading has to agree: `off` is the graceful verb.
                StageCondition::HostStillDrawing => {
                    let dark = matches!(node.led, granite_proto::LedMode::Off);
                    let drawing = threshold != 0 && node.telemetry.watts > threshold;
                    if !(enabled && dark && drawing) {
                        skip.push(job.id);
                        continue;
                    }
                    let since = *watch.above_since_ms.get_or_insert(now_ms);
                    if now_ms.saturating_sub(since) >= HOST_RUNNING_CONFIRM_MS {
                        cut.push(job.id);
                    }
                }
                // The slot a rail-on verb asked for, on the first
                // reading that is newer than the verb: a latch is the
                // job's failure, under the latch's own name. The unit's
                // hot-swap latch counts too - it holds both of its
                // slots off and no `on` can lift it (ADR 0005
                // decision 3).
                StageCondition::SlotUnlatched => {
                    let unit_held = self.planes[i]
                        .plane
                        .units
                        .get(node.unit() as usize)
                        .is_some_and(|u| {
                            u.status
                                .contains(granite_proto::UnitStatus::HOT_SWAP_LATCHED_OFF)
                        });
                    let found = if node.status.contains(granite_proto::SlotStatus::OV_LATCHED) {
                        Some(SlotLatch::OvLatched)
                    } else if node
                        .status
                        .contains(granite_proto::SlotStatus::POWER_LIMIT_TRIPPED)
                    {
                        Some(SlotLatch::PowerLimitTripped)
                    } else if unit_held {
                        Some(SlotLatch::UnitLatchedOff)
                    } else {
                        None
                    };
                    latched.push((job.id, found));
                }
            }
        }
        for id in cut {
            self.cut_stage(id, now_ms);
        }
        for id in skip {
            self.skip_cut_stage(id, now_ms);
        }
        for (id, found) in latched {
            self.finish_latch_watch(id, found, now_ms);
        }
    }

    /// The watches nobody answered in time: `force_off` cuts anyway,
    /// `off` leaves the rail alone (see
    /// [`StageCondition::cuts_when_unreadable`]), and a rail-on verb
    /// whose slot never reported ends on the plane's own result.
    fn expire_stage_watches(&mut self, now_ms: u64) {
        let late: Vec<(JobId, StageCondition)> = self
            .jobs
            .iter()
            .filter_map(|job| {
                job.watch
                    .filter(|w| now_ms > w.until_ms)
                    .map(|w| (job.id, w.condition))
            })
            .collect();
        for (id, condition) in late {
            if condition.checked_after() {
                self.finish_latch_watch(id, None, now_ms);
            } else if condition.cuts_when_unreadable() {
                self.cut_stage(id, now_ms);
            } else {
                self.skip_cut_stage(id, now_ms);
            }
        }
    }

    /// Put one stage's writes on the plane queues.
    fn queue_stage(&mut self, id: JobId, stage: usize, steps: Vec<Step>) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.outstanding = steps.len();
        }
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

    /// Release the stages whose wait is over.
    fn release_stages(&mut self, now_ms: u64) {
        let due: Vec<JobId> = self
            .jobs
            .iter()
            .filter(|j| j.hold_until_ms.is_some_and(|until| now_ms >= until))
            .map(|j| j.id)
            .collect();
        for id in due {
            let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
                continue;
            };
            job.hold_until_ms = None;
            let stage = job.stage;
            let Some(steps) = job.stages.get(stage).cloned() else {
                continue;
            };
            self.queue_stage(id, stage, steps);
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
        // second time. Such a job ends on its own deadline instead
        // ([`job_timeout_ms`]), which is the harmless direction.
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
        let next = job.stage + 1;
        // `force_off`'s cut stage follows the node-off sequence whatever
        // that answered: the verb promises the rail, not the plane's
        // opinion of it (ADR 0004, "`force_off` always ends with the
        // rail down"). A failure is kept as the job's own code, so a
        // sequence that went wrong is still reported - with `rail_cut`
        // beside it saying the node did end up off.
        let condition = job.command.stage_condition(next);
        if (worst.is_ok() || condition.runs_after_failure()) && next < job.stages.len() {
            job.stage = next;
            // A cut stage may never run at all, so the code the plane's
            // sequence earned is kept: it is what the job reports when
            // the cut turns out not to be needed. `severity` makes that
            // safe - an Ok loses to anything the cut itself reports.
            if worst.is_ok() && condition.is_always() {
                job.code = None;
            }
            self.start_stage(id, now_ms);
            return;
        }
        // A rail-on verb is not done because the plane took its write:
        // a slot whose OV latch is set answers `ok` and stays dark. The
        // job waits for the first SUMMARY newer than this moment and
        // ends on what the slot says (ADR 0004, "As built";
        // [`StageCondition::SlotUnlatched`]).
        if worst.is_ok() && self.watch_for_latch(id, now_ms) {
            return;
        }
        self.finish_job(id, outcome_of(worst), now_ms);
    }

    /// Hold a finished rail-on job back until the slot has reported
    /// whether it is latched, and say whether it was held.
    ///
    /// Nothing is held when the verb is not one that ends with a rail
    /// coming up ([`Command::checks_latch`]) or when the node is not a
    /// place on the link; such a job ends as it always did.
    fn watch_for_latch(&mut self, id: JobId, now_ms: u64) -> bool {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return false;
        };
        if !job.command.checks_latch() {
            return false;
        }
        let node = job.command.nodes().first().copied().unwrap_or(0);
        let Some((address, slot)) = node_location(node) else {
            return false;
        };
        job.outstanding = 0;
        job.watch = Some(StageWatch {
            condition: StageCondition::SlotUnlatched,
            address,
            slot,
            after_ms: now_ms,
            until_ms: now_ms.saturating_add(STAGE_WATCH_MS),
            above_since_ms: None,
        });
        true
    }

    /// End a job whose latch watch has decided.
    ///
    /// A latch is a failure under the latch's own name, whatever the
    /// plane's result said; no latch leaves the outcome the plane's
    /// sequence earned, which is also what a watch that timed out ends
    /// with - an unreadable slot is no proof of a latch.
    fn finish_latch_watch(&mut self, id: JobId, latched: Option<SlotLatch>, now_ms: u64) {
        let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) else {
            return;
        };
        job.watch = None;
        job.latched = latched;
        let outcome = match latched {
            Some(_) => JobOutcome::Failed,
            None => job.code.map_or(JobOutcome::Done, outcome_of),
        };
        self.finish_job(id, outcome, now_ms);
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
        self.note_intent(&job.command, outcome, job.rail_cut, now_ms);
        self.events
            .push_back(ClientEvent::JobFinished { id, outcome, code });
        self.finished.push_back(FinishedJob {
            id,
            command: job.command,
            outcome,
            code,
            submitted_ms: job.submitted_ms,
            finished_ms: now_ms,
            rail_cut: job.rail_cut,
            latched: job.latched,
        });
    }

    /// Give up on jobs that have run past their own deadline.
    fn expire_jobs(&mut self, now_ms: u64) {
        let late: Vec<JobId> = self
            .jobs
            .iter()
            .filter(|j| now_ms > j.deadline_ms)
            .map(|j| j.id)
            .collect();
        for id in late {
            self.finish_job(id, JobOutcome::TimedOut, now_ms);
        }
    }

    // --- intended OFF -------------------------------------------------

    /// What a finished job means for the intended-OFF set.
    ///
    /// `rail_cut` says the controller sent the cut stage itself, which
    /// is a certainty where the verb alone is not: such a job has to
    /// leave the node in the set, or the re-apply of ADR 0004 would let
    /// a plane that lost its intended state bring the rail back.
    fn note_intent(
        &mut self,
        command: &Command,
        outcome: JobOutcome,
        rail_cut: bool,
        now_ms: u64,
    ) {
        let Command::Node(node_command) = command else {
            return;
        };
        if node_command.intends_rail_on() {
            // The user asked for the rail, and that holds whatever the
            // verb's outcome: an `on` with no host to wake ends failed
            // after its LED wait with the rail up, and keeping the node
            // in the set would have the re-apply cut it at once (bench,
            // 2026-11-25). A rail that did not come up gives the
            // re-apply nothing to do either way.
            for node in node_command.nodes() {
                self.intent.remove(node);
            }
            return;
        }
        if !outcome.is_ok() {
            // A rail-off that did not happen is not a state to
            // remember. The re-apply of ADR 0004 needs nothing of us
            // here: it looks at the slot, so a failed attempt is simply
            // made again on the next STATUS.
            return;
        }
        if node_command.intends_rail_off() || rail_cut {
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

    /// Set the host-detect threshold \[0.1 W\] every plane derives its
    /// host axis with (`nodes.host_detect_w`, see
    /// [`super::model::Node::host_seen`] and
    /// [`super::model::DEFAULT_HOST_DETECT_DW`]).
    ///
    /// Unlike the per-node settings, which are read when a command is
    /// dispatched, this one is used on the poll path, so the firmware
    /// pushes it whenever the config changes. The states are derived
    /// again at once so a save shows up without waiting for the next
    /// SUMMARY.
    pub fn set_host_detect_dw(&mut self, deciwatts: u16) {
        for state in &mut self.planes {
            if state.plane.host_detect_dw != deciwatts {
                state.plane.host_detect_dw = deciwatts;
                state.plane.derive_states();
            }
        }
    }

    /// Set the host-running threshold \[0.1 W\] the cut stage of `off`
    /// uses (`nodes.host_running_w`, see
    /// [`StageCondition::HostStillDrawing`] and
    /// [`DEFAULT_HOST_RUNNING_DW`]). `0` turns that check off, and the
    /// rail stays up whenever the plane's sequence ended Ok.
    ///
    /// Pushed by the firmware's `apply_config`, like the host-detect
    /// threshold: a job decides this on the poll path, from the SUMMARY
    /// telemetry, and not where the command was built.
    pub fn set_host_running_dw(&mut self, deciwatts: u16) {
        self.host_running_dw = deciwatts;
    }

    /// The host-running threshold in force \[0.1 W\].
    pub const fn host_running_dw(&self) -> u16 {
        self.host_running_dw
    }

    /// Say which nodes have no front-panel cable, as a bitmap of node
    /// ids (bit `id - 1`, so node 1 is bit 0): `nodes[].front_panel`
    /// turned around, which is what [`crate::config::NodesCfg::no_front_panel_mask`]
    /// gives (ADR 0004, "Nodes without a front panel").
    ///
    /// Like the host-detect threshold, this is a per-node setting the
    /// poll path needs: the host axis of such a node is `unknown`
    /// whatever its LED reads. The verbs themselves do not come through
    /// here - the dispatcher reads the configuration when a command is
    /// built (decision 4).
    pub fn set_no_front_panel(&mut self, mask: u64) {
        for (n, state) in self.planes.iter_mut().enumerate() {
            let slots = (mask >> (n * SLOT_COUNT)) as u8;
            state
                .plane
                .set_no_front_panel(IndexBitmap::from_bits(slots));
        }
    }

    /// The nodes the client was told have no front panel, as the bitmap
    /// [`PlaneClient::set_no_front_panel`] takes.
    pub fn no_front_panel(&self) -> u64 {
        let mut mask = 0u64;
        for (n, state) in self.planes.iter().enumerate() {
            for node in state.plane.nodes.iter() {
                if !node.front_panel {
                    mask |= 1 << (n * SLOT_COUNT + node.slot as usize);
                }
            }
        }
        mask
    }

    /// The host-detect threshold in force \[0.1 W\].
    pub fn host_detect_dw(&self) -> u16 {
        self.planes
            .first()
            .map_or(crate::plane::model::DEFAULT_HOST_DETECT_DW, |s| {
                s.plane.host_detect_dw
            })
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

    /// The addresses of the planes the controller had found and that
    /// stopped answering, lowest first.
    ///
    /// This is what Modbus fault bit 0 reports (`PLANE_LOST`): an
    /// address drops out of the list when it answers again, or when
    /// [`PlaneClient::forget_plane`] gives up on it. A client that has
    /// just started has nothing in it, because it has not found
    /// anything to miss yet.
    pub fn lost_addresses(&self) -> Vec<u8> {
        self.planes
            .iter()
            .filter(|s| s.lost && !s.present)
            .map(|s| s.plane.address)
            .collect()
    }

    /// Stop missing a plane: the address is scanned on as before, but it
    /// no longer counts as lost.
    ///
    /// For an operator who has taken a plane out of the frame on
    /// purpose. True when there was something to forget.
    pub fn forget_plane(&mut self, address: u8) -> bool {
        match granite_proto::plane_of_address(address) {
            Some(n) if self.planes[n as usize].lost => {
                self.planes[n as usize].lost = false;
                true
            }
            _ => false,
        }
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
    (mix(seed ^ 0x9E37_79B9u32.wrapping_mul(n as u32 + 1)) & 0xFF) as u8
}

/// The first SYNC epoch from the client's seed (ADR 0005 decision 7:
/// "random at boot"). The same mix, over a different constant, so the
/// epoch is not a plane's sequence start.
fn epoch_start(seed: u32) -> u32 {
    mix(seed ^ 0x5BF0_3635)
}

/// The usual xorshift-multiply mix: enough to spread one seed over the
/// values the client needs it for.
fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::LinkError;

    /// The dispatcher thread builds a `PlaneClient` in its own frame, and
    /// `lto = true` inlines the constructor there, so every byte of this
    /// type is a byte of that stack - twice over, while the value is
    /// being moved into place. It was 8512 bytes on the host (7808 of
    /// plane states and a 256-byte answer buffer) and 83 % of the
    /// dispatcher's 24 KB stack was in use on a rev C board
    /// (2026-11-04). Keep the big members boxed.
    #[test]
    fn the_client_is_small_enough_to_build_on_a_stack() {
        assert!(
            core::mem::size_of::<PlaneClient>() < 1024,
            "PlaneClient is {} bytes; the big members belong on the heap",
            core::mem::size_of::<PlaneClient>()
        );
    }

    /// A link nothing answers on, which counts the SYNCs it carried and
    /// reports the events it is told to.
    #[derive(Default)]
    struct SilentLink {
        syncs: Vec<u32>,
        raised: u8,
    }

    impl PlaneLink for SilentLink {
        fn write_read(
            &mut self,
            _addr: u8,
            _req: &[u8],
            _resp: &mut [u8],
        ) -> Result<usize, LinkError> {
            Err(LinkError::Nak)
        }

        fn write(&mut self, _addr: u8, _frame: &[u8]) -> Result<(), LinkError> {
            Err(LinkError::Nak)
        }

        fn event_nodes(&mut self) -> u8 {
            core::mem::take(&mut self.raised)
        }

        fn sync(&mut self, epoch: u32) -> Result<(), LinkError> {
            self.syncs.push(epoch);
            Ok(())
        }
    }

    /// ADR 0005 decision 7: one SYNC per second, the epoch one higher
    /// each time and not the same at two boots.
    #[test]
    fn sync_goes_out_once_a_second_with_a_rising_epoch() {
        let mut client = PlaneClient::new(0x1234_5678);
        let first = client.sync_epoch();
        let mut link = SilentLink::default();
        // Ten seconds of 50 ms polls are ten SYNCs, not two hundred.
        for step in 1..=200u64 {
            client.poll(step * 50, &mut link);
        }
        assert_eq!(link.syncs.len(), 10);
        for (i, epoch) in link.syncs.iter().enumerate() {
            assert_eq!(*epoch, first.wrapping_add(i as u32));
        }
        assert_eq!(client.sync_epoch(), first.wrapping_add(10));
        assert_ne!(
            epoch_start(1),
            epoch_start(2),
            "a different seed starts at a different epoch"
        );
        assert_ne!(
            epoch_start(0x1234_5678),
            u32::from(seq_start(0x1234_5678, 0)),
            "the epoch is not a sequence start"
        );
    }

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
