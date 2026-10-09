//! The node actuator: the action table from the ADR as a state machine.
//!
//! The engine is a queue of jobs plus [`Actuator::tick`], which is called
//! with the current monotonic time and the sensor snapshot. It never
//! sleeps and never blocks, so the host tests drive it with a fake clock
//! and a fake [`NodeSwitches`].
//!
//! Two invariants from the ADR are structural here:
//!
//! - One action per node (a second action on a busy node is refused) and
//!   one *physical press* at a time overall. A stagger, a soft-off wait or
//!   a cycle delay does not hold the press lock, so other nodes keep
//!   working while one node takes its 120 s to shut down.
//! - Every press is bounded. The core computes the deadline
//!   (press duration + 500 ms, capped at 12 s), hands it to
//!   [`NodeSwitches::arm_deadline`] so the hardware can drop the expander
//!   reset line on its own, and enforces the same bound in software: a
//!   tick that arrives after the deadline releases every relay and fails
//!   the action with an expander fault.

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::hal::{HalError, NodeSwitches, Switch};
use crate::node::{BootPolicy, LastAction, NodeState, Nodes};
use crate::observed::Observed;
use crate::{NODE_COUNT, NodeId, Target, is_node};

/// Slack added to a press duration to get the deadline.
pub const DEADLINE_SLACK_MS: u32 = 500;
/// Hard cap on a press deadline.
pub const DEADLINE_MAX_MS: u32 = 12_000;
/// Shortest raw `press`.
pub const PRESS_MIN_MS: u32 = 100;
/// Longest raw `press`.
pub const PRESS_MAX_MS: u32 = 10_000;
/// How much longer PWR is held after the LED goes off in `force_off`.
pub const FORCE_OFF_EXTRA_MS: u32 = 500;
/// Most jobs the queue holds (8 nodes plus headroom for a staggered all).
pub const MAX_JOBS: usize = 16;

/// The deadline for a press of `press_ms`: duration + 500 ms, max 12 s.
pub const fn deadline_for(press_ms: u32) -> u32 {
    let d = press_ms.saturating_add(DEADLINE_SLACK_MS);
    if d > DEADLINE_MAX_MS {
        DEADLINE_MAX_MS
    } else {
        d
    }
}

/// The actions from the ADR table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Short PWR press, wait for the LED, no-op if already on.
    On,
    /// Short PWR press (ACPI soft-off), escalating to `force_off`.
    Off,
    /// Hold PWR until the LED goes off plus 500 ms, max `t_hold`.
    ForceOff,
    /// Short RST press; refused when the node is off.
    Reset,
    /// `off` (or `force_off` with `hard`), `t_cycle`, then `on`.
    Cycle,
    /// Raw press of PWR or RST for a given duration, LED ignored.
    Press,
    /// Staggered `on` over the configured node order.
    OnAll,
}

impl ActionKind {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            ActionKind::On => "on",
            ActionKind::Off => "off",
            ActionKind::ForceOff => "force_off",
            ActionKind::Reset => "reset",
            ActionKind::Cycle => "cycle",
            ActionKind::Press => "press",
            ActionKind::OnAll => "on_all",
        }
    }

    /// Parse a wire name.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        Some(match s {
            "on" => ActionKind::On,
            "off" => ActionKind::Off,
            "force_off" => ActionKind::ForceOff,
            "reset" => ActionKind::Reset,
            "cycle" => ActionKind::Cycle,
            "press" => ActionKind::Press,
            "on_all" => ActionKind::OnAll,
            _ => return None,
        })
    }

    /// True for the actions that reason about the power LED, and that are
    /// therefore refused on an `Unknown` node unless forced.
    pub const fn needs_state(self) -> bool {
        matches!(
            self,
            ActionKind::On
                | ActionKind::Off
                | ActionKind::ForceOff
                | ActionKind::Reset
                | ActionKind::Cycle
                | ActionKind::OnAll
        )
    }
}

impl fmt::Display for ActionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-action arguments the ADR defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionArgs {
    /// Act even though the node state is `Unknown`.
    pub force: bool,
    /// `off` returns `shutdown_pending` instead of escalating.
    pub no_escalate: bool,
    /// `cycle` uses `force_off` for the off half.
    pub hard: bool,
    /// Which switch a raw `press` uses (default PWR).
    pub switch: Option<Switch>,
    /// Raw `press` duration in milliseconds.
    pub duration_ms: Option<u32>,
}

/// A queued request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionRequest {
    /// Command id, echoed in events and the ack.
    pub id: String,
    /// Which action.
    pub kind: ActionKind,
    /// Which node, or all.
    pub target: Target,
    /// Arguments.
    pub args: ActionArgs,
}

impl ActionRequest {
    /// A request with default arguments.
    pub fn new(id: impl Into<String>, kind: ActionKind, target: Target) -> Self {
        ActionRequest {
            id: id.into(),
            kind,
            target,
            args: ActionArgs::default(),
        }
    }

    /// Builder: set the arguments.
    pub fn with_args(mut self, args: ActionArgs) -> Self {
        self.args = args;
        self
    }
}

/// Why a request was refused before any relay moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefuseReason {
    /// Node state is `Unknown` and the command did not say `force`.
    UnknownState,
    /// An action is already running on that node.
    Busy,
    /// The job queue is full.
    QueueFull,
    /// `reset` on a node whose LED is off.
    NodeOff,
    /// Target is not a node 1..=8, or `all` where a node is required.
    BadTarget,
    /// A raw `press` duration outside 100 ms-10 s, or missing.
    BadDuration,
}

impl RefuseReason {
    /// Lowercase wire name.
    pub const fn as_str(&self) -> &'static str {
        match self {
            RefuseReason::UnknownState => "unknown_state",
            RefuseReason::Busy => "busy",
            RefuseReason::QueueFull => "queue_full",
            RefuseReason::NodeOff => "node_off",
            RefuseReason::BadTarget => "bad_target",
            RefuseReason::BadDuration => "bad_duration",
        }
    }
}

impl fmt::Display for RefuseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why an action that started did not reach its goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailReason {
    /// The LED never came on within `t_on`.
    PowerOnTimeout,
    /// PWR was held the full `t_hold` and the LED stayed on.
    StillOn,
    /// A press outlived its deadline: the expander was reset under us.
    DeadlineExceeded,
    /// Readback mismatch or bus trouble on the expander.
    ExpanderFault,
    /// Some other HAL failure, with its text.
    Hal(String),
    /// A staggered group had at least one child that did not succeed.
    Partial,
}

impl fmt::Display for FailReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FailReason::PowerOnTimeout => f.write_str("power_on_timeout"),
            FailReason::StillOn => f.write_str("still_on"),
            FailReason::DeadlineExceeded => f.write_str("deadline_exceeded"),
            FailReason::ExpanderFault => f.write_str("expander_fault"),
            FailReason::Hal(m) => write!(f, "hal: {m}"),
            FailReason::Partial => f.write_str("partial"),
        }
    }
}

impl From<HalError> for FailReason {
    fn from(e: HalError) -> Self {
        match e {
            HalError::ExpanderFault => FailReason::ExpanderFault,
            other => FailReason::Hal(other.to_string()),
        }
    }
}

/// How an action ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ActionResult {
    /// Goal reached (or already true).
    Ok,
    /// `off` with `no_escalate`: soft-off was asked for and the node has
    /// not gone down yet.
    ShutdownPending,
    /// Nothing was attempted.
    Refused {
        /// Why.
        reason: RefuseReason,
    },
    /// Something was attempted and did not work.
    Failed {
        /// Why.
        reason: FailReason,
    },
}

impl ActionResult {
    /// Refusal shorthand.
    pub const fn refused(reason: RefuseReason) -> Self {
        ActionResult::Refused { reason }
    }

    /// Failure shorthand.
    pub const fn failed(reason: FailReason) -> Self {
        ActionResult::Failed { reason }
    }

    /// True for `Ok` and `ShutdownPending`.
    pub const fn is_ok(&self) -> bool {
        matches!(self, ActionResult::Ok | ActionResult::ShutdownPending)
    }

    /// Short wire string, as published in the ack's `result` field.
    pub fn as_str(&self) -> String {
        match self {
            ActionResult::Ok => "ok".to_string(),
            ActionResult::ShutdownPending => "shutdown_pending".to_string(),
            ActionResult::Refused { reason } => {
                let mut s = String::from("refused: ");
                s.push_str(reason.as_str());
                s
            }
            ActionResult::Failed { reason } => {
                let mut s = String::from("failed: ");
                s.push_str(&reason.to_string());
                s
            }
        }
    }
}

/// Fault classes the actuator reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// A press was still closed when its deadline passed. This is the
    /// `ExpanderFault` class of the ADR: the hardware timer has (or
    /// should have) dropped the expander reset line.
    PressDeadline,
    /// The expander did not behave (readback mismatch, bus error).
    Expander,
    /// Another HAL error, with its text.
    Hal(String),
}

impl fmt::Display for FaultKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaultKind::PressDeadline => f.write_str("press_deadline"),
            FaultKind::Expander => f.write_str("expander"),
            FaultKind::Hal(m) => write!(f, "hal: {m}"),
        }
    }
}

/// Everything the actuator wants the outside world to know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActuatorEvent {
    /// A job was queued (one per node, also for the children of `on_all`).
    Accepted {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
        /// Action.
        action: ActionKind,
    },
    /// A relay just closed.
    PressStart {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
        /// Which switch.
        switch: Switch,
        /// Planned duration (the maximum, for `force_off`).
        duration_ms: u32,
        /// Deadline handed to the hardware.
        deadline_ms: u32,
    },
    /// A relay just opened.
    PressEnd {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
        /// Which switch.
        switch: Switch,
        /// How long it was actually closed.
        held_ms: u32,
    },
    /// Soft-off ran out of patience; emitted before any escalation.
    SoftOffTimeout {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
    },
    /// Soft-off escalated to `force_off`.
    Escalated {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
    },
    /// A per-node job finished.
    Done {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
        /// Action.
        action: ActionKind,
        /// Outcome.
        result: ActionResult,
    },
    /// A staggered group (`on_all`, boot policy) finished.
    GroupDone {
        /// Command id.
        id: String,
        /// Action.
        action: ActionKind,
        /// Aggregate outcome.
        result: ActionResult,
    },
    /// Something is wrong with the hardware.
    Fault {
        /// Command id, if a job was involved.
        id: Option<String>,
        /// Node, if a job was involved.
        node: Option<NodeId>,
        /// What happened.
        fault: FaultKind,
    },
}

/// Timing configuration, with the ADR defaults.
///
/// Every field has a validation range. The ADR fixes the ranges of
/// `t_short` (100-1000 ms) and `t_hold` (4-10 s); the others are bounded
/// here to values that keep the state machine sane (a `t_soft_off` of a
/// day would pin a node `Busy` forever).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timings {
    /// Short press length.
    pub t_short_ms: u32,
    /// Longest PWR hold for `force_off`.
    pub t_hold_ms: u32,
    /// How long `on` waits for the LED.
    pub t_on_ms: u32,
    /// How long `off` waits before escalating.
    pub t_soft_off_ms: u32,
    /// Pause between the halves of `cycle`.
    pub t_cycle_ms: u32,
    /// Gap between presses in a staggered group.
    pub t_stagger_ms: u32,
    /// Settling time after boot before boot policies are applied.
    pub t_settle_ms: u32,
}

impl Default for Timings {
    fn default() -> Self {
        Timings {
            t_short_ms: 250,
            t_hold_ms: 8_000,
            t_on_ms: 10_000,
            t_soft_off_ms: 120_000,
            t_cycle_ms: 10_000,
            t_stagger_ms: 5_000,
            t_settle_ms: 5_000,
        }
    }
}

/// Inclusive validation range of one timing field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// Field name as it appears in config JSON.
    pub field: &'static str,
    /// Smallest accepted value.
    pub min: u32,
    /// Largest accepted value.
    pub max: u32,
}

/// The validation ranges, in config field order.
pub const TIMING_RANGES: [Range; 7] = [
    Range {
        field: "t_short_ms",
        min: 100,
        max: 1_000,
    },
    Range {
        field: "t_hold_ms",
        min: 4_000,
        max: 10_000,
    },
    Range {
        field: "t_on_ms",
        min: 1_000,
        max: 300_000,
    },
    Range {
        field: "t_soft_off_ms",
        min: 5_000,
        max: 900_000,
    },
    Range {
        field: "t_cycle_ms",
        min: 1_000,
        max: 300_000,
    },
    Range {
        field: "t_stagger_ms",
        min: 0,
        max: 60_000,
    },
    Range {
        field: "t_settle_ms",
        min: 0,
        max: 60_000,
    },
];

/// A timing field outside its range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimingError {
    /// Which field.
    pub range: Range,
    /// What was asked for.
    pub value: u32,
}

impl fmt::Display for TimingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} = {} is outside {}..={}",
            self.range.field, self.value, self.range.min, self.range.max
        )
    }
}

impl Timings {
    /// The fields in the same order as [`TIMING_RANGES`].
    pub const fn as_array(&self) -> [u32; 7] {
        [
            self.t_short_ms,
            self.t_hold_ms,
            self.t_on_ms,
            self.t_soft_off_ms,
            self.t_cycle_ms,
            self.t_stagger_ms,
            self.t_settle_ms,
        ]
    }

    /// Check every field against its range.
    pub fn validate(&self) -> Result<(), TimingError> {
        for (value, range) in self.as_array().into_iter().zip(TIMING_RANGES) {
            if value < range.min || value > range.max {
                return Err(TimingError { range, value });
            }
        }
        Ok(())
    }

    /// Clamp every field into its range. Used when a stored config
    /// predates a range change.
    pub fn clamped(mut self) -> Self {
        let v = self.as_array();
        let mut out = [0u32; 7];
        for i in 0..7 {
            out[i] = v[i].clamp(TIMING_RANGES[i].min, TIMING_RANGES[i].max);
        }
        self.t_short_ms = out[0];
        self.t_hold_ms = out[1];
        self.t_on_ms = out[2];
        self.t_soft_off_ms = out[3];
        self.t_cycle_ms = out[4];
        self.t_stagger_ms = out[5];
        self.t_settle_ms = out[6];
        self
    }
}

/// The default node order for staggered actions: 1..=8.
pub const fn default_order() -> [NodeId; NODE_COUNT] {
    [1, 2, 3, 4, 5, 6, 7, 8]
}

/// What happens when an `AwaitLed` step runs out of time.
#[derive(Debug, Clone, PartialEq, Eq)]
enum OnTimeout {
    Fail(FailReason),
    Escalate,
    ShutdownPending,
}

/// One elementary operation of an action script.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// Close a relay for a fixed time. Needs the press lock.
    Press { sw: Switch, dur_ms: u32 },
    /// Hold PWR until the LED is off plus 500 ms, at most `max_ms`.
    /// Needs the press lock.
    HoldUntilOff { max_ms: u32 },
    /// Wait for the LED to read `want`.
    AwaitLed {
        want: bool,
        timeout_ms: u32,
        on_timeout: OnTimeout,
    },
    /// Wait.
    Delay { ms: u32 },
    /// Verify the LED now reads `want`, fail otherwise.
    CheckLed { want: bool, fail: FailReason },
}

/// Where a job is inside its current step.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Run {
    Idle,
    Pressing {
        sw: Switch,
        started_ms: u64,
        ends_ms: u64,
        deadline_at_ms: u64,
    },
    Holding {
        started_ms: u64,
        max_end_ms: u64,
        deadline_at_ms: u64,
        off_since_ms: Option<u64>,
    },
    Awaiting {
        until_ms: u64,
    },
    Delaying {
        until_ms: u64,
    },
}

#[derive(Debug, Clone)]
struct Job {
    id: String,
    node: NodeId,
    kind: ActionKind,
    sense_ignored: bool,
    start_after_ms: u64,
    group: Option<u32>,
    steps: VecDeque<Step>,
    run: Run,
}

#[derive(Debug, Clone)]
struct Group {
    gid: u32,
    id: String,
    kind: ActionKind,
    remaining: usize,
    failures: usize,
}

/// What [`Actuator::submit`] decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// Work was queued; the final result arrives as a `Done` event.
    Accepted {
        /// How many per-node jobs were queued.
        jobs: usize,
    },
    /// Nothing was queued and the outcome is already known (the node was
    /// in the target state, or the request was refused).
    Done(ActionResult),
}

impl Submission {
    /// The already-known result, if there is one.
    pub fn result(&self) -> Option<&ActionResult> {
        match self {
            Submission::Done(r) => Some(r),
            Submission::Accepted { .. } => None,
        }
    }
}

/// The actuator.
#[derive(Debug)]
pub struct Actuator<S: NodeSwitches> {
    switches: S,
    timings: Timings,
    order: [NodeId; NODE_COUNT],
    jobs: heapless::Vec<Job, MAX_JOBS>,
    groups: Vec<Group>,
    press_owner: Option<NodeId>,
    next_gid: u32,
}

impl<S: NodeSwitches> Actuator<S> {
    /// New actuator with the default timings and node order.
    pub fn new(switches: S) -> Self {
        Actuator {
            switches,
            timings: Timings::default(),
            order: default_order(),
            jobs: heapless::Vec::new(),
            groups: Vec::new(),
            press_owner: None,
            next_gid: 1,
        }
    }

    /// Replace the timings (from the `nodes` config section).
    pub fn set_timings(&mut self, timings: Timings) {
        self.timings = timings;
    }

    /// Current timings.
    pub const fn timings(&self) -> &Timings {
        &self.timings
    }

    /// Replace the node order used by staggered actions. Entries that are
    /// not valid node ids, or repeats, are dropped and the missing nodes
    /// are appended in ascending order.
    pub fn set_order(&mut self, order: [NodeId; NODE_COUNT]) {
        self.order = sanitise_order(order);
    }

    /// Current node order.
    pub const fn order(&self) -> &[NodeId; NODE_COUNT] {
        &self.order
    }

    /// Borrow the switches (the firmware needs this for its fault paths).
    pub fn switches_mut(&mut self) -> &mut S {
        &mut self.switches
    }

    /// Number of jobs in flight.
    pub fn in_flight(&self) -> usize {
        self.jobs.len()
    }

    /// The node currently holding a relay closed, if any.
    pub const fn press_owner(&self) -> Option<NodeId> {
        self.press_owner
    }

    /// True while a job for `node` exists.
    pub fn is_busy(&self, node: NodeId) -> bool {
        self.jobs.iter().any(|j| j.node == node)
    }

    /// Open every relay and drop every job. Used on an expander
    /// reinitialisation and before a reboot.
    pub fn abort_all(&mut self, now_ms: u64) -> Vec<ActuatorEvent> {
        let mut out = Vec::new();
        let _ = self.switches.disarm_deadline();
        if let Err(e) = self.switches.release_all() {
            out.push(ActuatorEvent::Fault {
                id: None,
                node: None,
                fault: fault_of(&e),
            });
        }
        self.press_owner = None;
        let jobs: Vec<Job> = self.jobs.iter().cloned().collect();
        self.jobs.clear();
        for job in jobs {
            let result = ActionResult::failed(FailReason::ExpanderFault);
            out.push(ActuatorEvent::Done {
                id: job.id.clone(),
                node: job.node,
                action: job.kind,
                result: result.clone(),
            });
            if let Some(gid) = job.group {
                self.finish_group_child(gid, &result, &mut out);
            }
        }
        let _ = now_ms;
        out
    }

    /// Queue a request.
    pub fn submit(
        &mut self,
        req: &ActionRequest,
        now_ms: u64,
        observed: &Observed,
    ) -> (Submission, Vec<ActuatorEvent>) {
        let mut out = Vec::new();
        let sub = self.submit_inner(req, now_ms, observed, &mut out);
        (sub, out)
    }

    fn submit_inner(
        &mut self,
        req: &ActionRequest,
        now_ms: u64,
        observed: &Observed,
        out: &mut Vec<ActuatorEvent>,
    ) -> Submission {
        if !req.target.is_valid() {
            return Submission::Done(ActionResult::refused(RefuseReason::BadTarget));
        }

        // The staggered group actions.
        if req.kind == ActionKind::OnAll {
            let targets: Vec<NodeId> = self
                .order
                .iter()
                .copied()
                .filter(|n| observed.node_led(*n) != Some(true))
                .collect();
            return self.submit_group(req, ActionKind::On, &targets, now_ms, observed, out);
        }

        let Some(node) = req.target.node() else {
            // Every other action needs exactly one node.
            return Submission::Done(ActionResult::refused(RefuseReason::BadTarget));
        };
        match self.submit_one(req, req.kind, node, None, now_ms, observed, out) {
            Ok(true) => Submission::Accepted { jobs: 1 },
            Ok(false) => Submission::Done(ActionResult::Ok),
            Err(result) => Submission::Done(result),
        }
    }

    /// Apply the per-node boot policies once, staggered, in node order.
    /// The caller waits `t_settle` after boot before calling this.
    pub fn submit_boot_policy(
        &mut self,
        id: &str,
        policies: &[BootPolicy; NODE_COUNT],
        now_ms: u64,
        observed: &Observed,
    ) -> (Submission, Vec<ActuatorEvent>) {
        let mut out = Vec::new();
        let mut on: Vec<NodeId> = Vec::new();
        let mut off: Vec<NodeId> = Vec::new();
        for node in self.order {
            let idx = node as usize - 1;
            match policies[idx] {
                BootPolicy::Leave => {}
                BootPolicy::On => {
                    if observed.node_led(node) != Some(true) {
                        on.push(node);
                    }
                }
                BootPolicy::Off => {
                    if observed.node_led(node) != Some(false) {
                        off.push(node);
                    }
                }
            }
        }
        if on.is_empty() && off.is_empty() {
            return (Submission::Done(ActionResult::Ok), out);
        }
        let mut jobs = 0;
        let req_on = ActionRequest::new(id, ActionKind::On, Target::All);
        let req_off = ActionRequest::new(id, ActionKind::Off, Target::All);
        let gid = self.open_group(id, ActionKind::OnAll, on.len() + off.len());
        let mut slot = 0usize;
        for (req, kind, list) in [
            (&req_on, ActionKind::On, &on),
            (&req_off, ActionKind::Off, &off),
        ] {
            for node in list {
                let start = now_ms + (slot as u64) * self.timings.t_stagger_ms as u64;
                match self.submit_one(
                    req,
                    kind,
                    *node,
                    Some((gid, start)),
                    now_ms,
                    observed,
                    &mut out,
                ) {
                    Ok(true) => {
                        jobs += 1;
                        slot += 1;
                    }
                    Ok(false) => self.finish_group_child(gid, &ActionResult::Ok, &mut out),
                    Err(result) => {
                        out.push(ActuatorEvent::Done {
                            id: req.id.clone(),
                            node: *node,
                            action: kind,
                            result: result.clone(),
                        });
                        self.finish_group_child(gid, &result, &mut out);
                    }
                }
            }
        }
        if jobs == 0 {
            (Submission::Done(ActionResult::Ok), out)
        } else {
            (Submission::Accepted { jobs }, out)
        }
    }

    fn submit_group(
        &mut self,
        req: &ActionRequest,
        child: ActionKind,
        targets: &[NodeId],
        now_ms: u64,
        observed: &Observed,
        out: &mut Vec<ActuatorEvent>,
    ) -> Submission {
        if targets.is_empty() {
            return Submission::Done(ActionResult::Ok);
        }
        let gid = self.open_group(&req.id, req.kind, targets.len());
        let mut jobs = 0usize;
        let mut slot = 0usize;
        for node in targets {
            let start = now_ms + (slot as u64) * self.timings.t_stagger_ms as u64;
            match self.submit_one(req, child, *node, Some((gid, start)), now_ms, observed, out) {
                Ok(true) => {
                    jobs += 1;
                    slot += 1;
                }
                Ok(false) => self.finish_group_child(gid, &ActionResult::Ok, out),
                Err(result) => {
                    out.push(ActuatorEvent::Done {
                        id: req.id.clone(),
                        node: *node,
                        action: child,
                        result: result.clone(),
                    });
                    self.finish_group_child(gid, &result, out);
                }
            }
        }
        if jobs == 0 {
            // Everything resolved without queueing anything; the group
            // event already carries the aggregate.
            Submission::Done(ActionResult::Ok)
        } else {
            Submission::Accepted { jobs }
        }
    }

    /// `Ok(true)`: queued. `Ok(false)`: already in the target state.
    /// `Err(result)`: refused.
    #[allow(clippy::too_many_arguments)]
    fn submit_one(
        &mut self,
        req: &ActionRequest,
        kind: ActionKind,
        node: NodeId,
        group: Option<(u32, u64)>,
        now_ms: u64,
        observed: &Observed,
        out: &mut Vec<ActuatorEvent>,
    ) -> Result<bool, ActionResult> {
        if !is_node(node) {
            return Err(ActionResult::refused(RefuseReason::BadTarget));
        }
        if self.is_busy(node) {
            return Err(ActionResult::refused(RefuseReason::Busy));
        }
        if self.jobs.len() >= MAX_JOBS {
            return Err(ActionResult::refused(RefuseReason::QueueFull));
        }

        let ignored = observed.node_sense(node).is_ignored();
        let led = observed.node_led(node);
        if kind.needs_state() && !ignored && led.is_none() && !req.args.force {
            return Err(ActionResult::refused(RefuseReason::UnknownState));
        }

        let steps = self.script(kind, &req.args, led, ignored)?;
        if steps.is_empty() {
            return Ok(false);
        }

        let (gid, start_after) = match group {
            Some((gid, start)) => (Some(gid), start),
            None => (None, now_ms),
        };
        let job = Job {
            id: req.id.clone(),
            node,
            kind,
            sense_ignored: ignored,
            start_after_ms: start_after,
            group: gid,
            steps,
            run: Run::Idle,
        };
        self.jobs
            .push(job)
            .map_err(|_| ActionResult::refused(RefuseReason::QueueFull))?;
        out.push(ActuatorEvent::Accepted {
            id: req.id.clone(),
            node,
            action: kind,
        });
        Ok(true)
    }

    /// Compile an action into its step list, given what the LED says now.
    fn script(
        &self,
        kind: ActionKind,
        args: &ActionArgs,
        led: Option<bool>,
        ignored: bool,
    ) -> Result<VecDeque<Step>, ActionResult> {
        let t = &self.timings;
        let mut steps: VecDeque<Step> = VecDeque::new();
        match kind {
            ActionKind::On | ActionKind::OnAll => {
                if !ignored && led == Some(true) {
                    return Ok(steps);
                }
                steps.push_back(Step::Press {
                    sw: Switch::Pwr,
                    dur_ms: t.t_short_ms,
                });
                if !ignored {
                    steps.push_back(Step::AwaitLed {
                        want: true,
                        timeout_ms: t.t_on_ms,
                        on_timeout: OnTimeout::Fail(FailReason::PowerOnTimeout),
                    });
                }
            }
            ActionKind::Off => {
                if !ignored && led == Some(false) {
                    return Ok(steps);
                }
                for s in self.off_steps(args, ignored) {
                    steps.push_back(s);
                }
            }
            ActionKind::ForceOff => {
                if !ignored && led == Some(false) {
                    return Ok(steps);
                }
                for s in self.force_off_steps(ignored) {
                    steps.push_back(s);
                }
            }
            ActionKind::Reset => {
                if !ignored && led == Some(false) {
                    return Err(ActionResult::refused(RefuseReason::NodeOff));
                }
                steps.push_back(Step::Press {
                    sw: Switch::Rst,
                    dur_ms: t.t_short_ms,
                });
            }
            ActionKind::Cycle => {
                let needs_off = ignored || led != Some(false);
                if needs_off {
                    let off = if args.hard {
                        self.force_off_steps(ignored)
                    } else {
                        self.off_steps(args, ignored)
                    };
                    for s in off {
                        steps.push_back(s);
                    }
                    steps.push_back(Step::Delay { ms: t.t_cycle_ms });
                }
                steps.push_back(Step::Press {
                    sw: Switch::Pwr,
                    dur_ms: t.t_short_ms,
                });
                if !ignored {
                    steps.push_back(Step::AwaitLed {
                        want: true,
                        timeout_ms: t.t_on_ms,
                        on_timeout: OnTimeout::Fail(FailReason::PowerOnTimeout),
                    });
                }
            }
            ActionKind::Press => {
                let dur = args.duration_ms.unwrap_or(0);
                if !(PRESS_MIN_MS..=PRESS_MAX_MS).contains(&dur) {
                    return Err(ActionResult::refused(RefuseReason::BadDuration));
                }
                steps.push_back(Step::Press {
                    sw: args.switch.unwrap_or(Switch::Pwr),
                    dur_ms: dur,
                });
            }
        }
        Ok(steps)
    }

    fn off_steps(&self, args: &ActionArgs, ignored: bool) -> Vec<Step> {
        let t = &self.timings;
        let mut v = Vec::new();
        v.push(Step::Press {
            sw: Switch::Pwr,
            dur_ms: t.t_short_ms,
        });
        if !ignored {
            v.push(Step::AwaitLed {
                want: false,
                timeout_ms: t.t_soft_off_ms,
                on_timeout: if args.no_escalate {
                    OnTimeout::ShutdownPending
                } else {
                    OnTimeout::Escalate
                },
            });
        }
        v
    }

    fn force_off_steps(&self, ignored: bool) -> Vec<Step> {
        let t = &self.timings;
        if ignored {
            // No LED to watch: hold PWR for the full t_hold.
            return alloc::vec![Step::Press {
                sw: Switch::Pwr,
                dur_ms: t.t_hold_ms,
            }];
        }
        alloc::vec![
            Step::HoldUntilOff {
                max_ms: t.t_hold_ms
            },
            Step::CheckLed {
                want: false,
                fail: FailReason::StillOn,
            },
        ]
    }

    fn open_group(&mut self, id: &str, kind: ActionKind, children: usize) -> u32 {
        let gid = self.next_gid;
        self.next_gid = self.next_gid.wrapping_add(1).max(1);
        self.groups.push(Group {
            gid,
            id: String::from(id),
            kind,
            remaining: children,
            failures: 0,
        });
        gid
    }

    fn finish_group_child(
        &mut self,
        gid: u32,
        result: &ActionResult,
        out: &mut Vec<ActuatorEvent>,
    ) {
        let Some(pos) = self.groups.iter().position(|g| g.gid == gid) else {
            return;
        };
        {
            let g = &mut self.groups[pos];
            g.remaining = g.remaining.saturating_sub(1);
            if !result.is_ok() {
                g.failures += 1;
            }
            if g.remaining > 0 {
                return;
            }
        }
        let g = self.groups.remove(pos);
        let result = if g.failures == 0 {
            ActionResult::Ok
        } else {
            ActionResult::failed(FailReason::Partial)
        };
        out.push(ActuatorEvent::GroupDone {
            id: g.id,
            action: g.kind,
            result,
        });
    }

    /// Advance every job. Call this at least every 50 ms; it is cheap and
    /// idempotent when nothing is due.
    pub fn tick(&mut self, now_ms: u64, observed: &Observed) -> Vec<ActuatorEvent> {
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < self.jobs.len() {
            let mut job = self.jobs.remove(i);
            let finished = self.step_job(&mut job, now_ms, observed, &mut out);
            match finished {
                Some(result) => {
                    out.push(ActuatorEvent::Done {
                        id: job.id.clone(),
                        node: job.node,
                        action: job.kind,
                        result: result.clone(),
                    });
                    if let Some(gid) = job.group {
                        self.finish_group_child(gid, &result, &mut out);
                    }
                }
                None => {
                    // Put it back where it was so queue order is stable.
                    let _ = self.jobs.insert(i, job);
                    i += 1;
                }
            }
        }
        out
    }

    /// Returns `Some(result)` when the job is done.
    fn step_job(
        &mut self,
        job: &mut Job,
        now_ms: u64,
        observed: &Observed,
        out: &mut Vec<ActuatorEvent>,
    ) -> Option<ActionResult> {
        if now_ms < job.start_after_ms {
            return None;
        }
        loop {
            match job.run.clone() {
                Run::Idle => {
                    let Some(step) = job.steps.front().cloned() else {
                        return Some(ActionResult::Ok);
                    };
                    match step {
                        Step::Press { sw, dur_ms } => {
                            if !self.take_press(job.node) {
                                return None;
                            }
                            let deadline = deadline_for(dur_ms);
                            if let Err(e) = self.start_press(job.node, sw, deadline) {
                                return Some(self.fail_press(job, e, out));
                            }
                            out.push(ActuatorEvent::PressStart {
                                id: job.id.clone(),
                                node: job.node,
                                switch: sw,
                                duration_ms: dur_ms,
                                deadline_ms: deadline,
                            });
                            job.run = Run::Pressing {
                                sw,
                                started_ms: now_ms,
                                ends_ms: now_ms + dur_ms as u64,
                                deadline_at_ms: now_ms + deadline as u64,
                            };
                        }
                        Step::HoldUntilOff { max_ms } => {
                            if !self.take_press(job.node) {
                                return None;
                            }
                            let deadline = deadline_for(max_ms);
                            if let Err(e) = self.start_press(job.node, Switch::Pwr, deadline) {
                                return Some(self.fail_press(job, e, out));
                            }
                            out.push(ActuatorEvent::PressStart {
                                id: job.id.clone(),
                                node: job.node,
                                switch: Switch::Pwr,
                                duration_ms: max_ms,
                                deadline_ms: deadline,
                            });
                            job.run = Run::Holding {
                                started_ms: now_ms,
                                max_end_ms: now_ms + max_ms as u64,
                                deadline_at_ms: now_ms + deadline as u64,
                                off_since_ms: None,
                            };
                        }
                        Step::AwaitLed { timeout_ms, .. } => {
                            job.run = Run::Awaiting {
                                until_ms: now_ms + timeout_ms as u64,
                            };
                        }
                        Step::Delay { ms } => {
                            job.run = Run::Delaying {
                                until_ms: now_ms + ms as u64,
                            };
                        }
                        Step::CheckLed { want, fail } => {
                            job.steps.pop_front();
                            if !job.sense_ignored && observed.node_led(job.node) == Some(!want) {
                                return Some(ActionResult::failed(fail));
                            }
                            job.run = Run::Idle;
                        }
                    }
                }
                Run::Pressing {
                    sw,
                    started_ms,
                    ends_ms,
                    deadline_at_ms,
                } => {
                    if now_ms >= deadline_at_ms {
                        return Some(self.blow_deadline(job, out));
                    }
                    if now_ms < ends_ms {
                        return None;
                    }
                    if let Err(e) = self.end_press(job.node, sw) {
                        return Some(self.fail_press(job, e, out));
                    }
                    out.push(ActuatorEvent::PressEnd {
                        id: job.id.clone(),
                        node: job.node,
                        switch: sw,
                        held_ms: (now_ms - started_ms) as u32,
                    });
                    job.steps.pop_front();
                    job.run = Run::Idle;
                }
                Run::Holding {
                    started_ms,
                    max_end_ms,
                    deadline_at_ms,
                    off_since_ms,
                } => {
                    if now_ms >= deadline_at_ms {
                        return Some(self.blow_deadline(job, out));
                    }
                    let off_since = match off_since_ms {
                        Some(t) => Some(t),
                        None => {
                            if observed.node_led(job.node) == Some(false) {
                                Some(now_ms)
                            } else {
                                None
                            }
                        }
                    };
                    let done = match off_since {
                        Some(t) => now_ms >= t + FORCE_OFF_EXTRA_MS as u64 || now_ms >= max_end_ms,
                        None => now_ms >= max_end_ms,
                    };
                    if !done {
                        job.run = Run::Holding {
                            started_ms,
                            max_end_ms,
                            deadline_at_ms,
                            off_since_ms: off_since,
                        };
                        return None;
                    }
                    if let Err(e) = self.end_press(job.node, Switch::Pwr) {
                        return Some(self.fail_press(job, e, out));
                    }
                    out.push(ActuatorEvent::PressEnd {
                        id: job.id.clone(),
                        node: job.node,
                        switch: Switch::Pwr,
                        held_ms: (now_ms - started_ms) as u32,
                    });
                    job.steps.pop_front();
                    job.run = Run::Idle;
                }
                Run::Awaiting { until_ms } => {
                    let Some(Step::AwaitLed {
                        want,
                        on_timeout,
                        timeout_ms: _,
                    }) = job.steps.front().cloned()
                    else {
                        // Should not happen; treat as finished.
                        job.run = Run::Idle;
                        continue;
                    };
                    if observed.node_led(job.node) == Some(want) {
                        job.steps.pop_front();
                        job.run = Run::Idle;
                        continue;
                    }
                    if now_ms < until_ms {
                        return None;
                    }
                    job.steps.pop_front();
                    job.run = Run::Idle;
                    match on_timeout {
                        OnTimeout::Fail(reason) => return Some(ActionResult::failed(reason)),
                        OnTimeout::ShutdownPending => {
                            out.push(ActuatorEvent::SoftOffTimeout {
                                id: job.id.clone(),
                                node: job.node,
                            });
                            return Some(ActionResult::ShutdownPending);
                        }
                        OnTimeout::Escalate => {
                            out.push(ActuatorEvent::SoftOffTimeout {
                                id: job.id.clone(),
                                node: job.node,
                            });
                            out.push(ActuatorEvent::Escalated {
                                id: job.id.clone(),
                                node: job.node,
                            });
                            for s in self.force_off_steps(job.sense_ignored).into_iter().rev() {
                                job.steps.push_front(s);
                            }
                        }
                    }
                }
                Run::Delaying { until_ms } => {
                    if now_ms < until_ms {
                        return None;
                    }
                    job.steps.pop_front();
                    job.run = Run::Idle;
                }
            }
        }
    }

    fn take_press(&mut self, node: NodeId) -> bool {
        match self.press_owner {
            None => {
                self.press_owner = Some(node);
                true
            }
            Some(owner) => owner == node,
        }
    }

    fn start_press(&mut self, node: NodeId, sw: Switch, deadline_ms: u32) -> Result<(), HalError> {
        self.switches.arm_deadline(deadline_ms)?;
        self.switches.assert(node, sw)
    }

    fn end_press(&mut self, node: NodeId, sw: Switch) -> Result<(), HalError> {
        let r1 = self.switches.release(node, sw);
        self.press_owner = None;
        let r2 = self.switches.disarm_deadline();
        // Dropping the expander reset line between presses costs one I2C
        // reconfiguration but means no relay can be closed while the queue
        // is merely waiting. Strictly safer than holding it high across a
        // stagger, which is what the ADR allows.
        let r3 = self.switches.release_all();
        r1.and(r2).and(r3)
    }

    fn fail_press(&mut self, job: &Job, e: HalError, out: &mut Vec<ActuatorEvent>) -> ActionResult {
        out.push(ActuatorEvent::Fault {
            id: Some(job.id.clone()),
            node: Some(job.node),
            fault: fault_of(&e),
        });
        self.press_owner = None;
        let _ = self.switches.disarm_deadline();
        let _ = self.switches.release_all();
        ActionResult::failed(FailReason::from(e))
    }

    fn blow_deadline(&mut self, job: &Job, out: &mut Vec<ActuatorEvent>) -> ActionResult {
        out.push(ActuatorEvent::Fault {
            id: Some(job.id.clone()),
            node: Some(job.node),
            fault: FaultKind::PressDeadline,
        });
        self.press_owner = None;
        let _ = self.switches.disarm_deadline();
        let _ = self.switches.release_all();
        ActionResult::failed(FailReason::DeadlineExceeded)
    }
}

fn fault_of(e: &HalError) -> FaultKind {
    match e {
        HalError::ExpanderFault => FaultKind::PressDeadline,
        HalError::Bus => FaultKind::Expander,
        other => FaultKind::Hal(other.to_string()),
    }
}

/// Drop invalid and repeated node ids, append whatever is missing.
pub fn sanitise_order(order: [NodeId; NODE_COUNT]) -> [NodeId; NODE_COUNT] {
    let mut out = [0u8; NODE_COUNT];
    let mut n = 0usize;
    for node in order {
        if is_node(node) && !out[..n].contains(&node) {
            out[n] = node;
            n += 1;
        }
    }
    for node in 1..=NODE_COUNT as NodeId {
        if n == NODE_COUNT {
            break;
        }
        if !out[..n].contains(&node) {
            out[n] = node;
            n += 1;
        }
    }
    out
}

/// Fold actuator events into the node tracker: busy markers, last action,
/// last reset time. The firmware calls this once per tick.
pub fn apply_to_nodes(nodes: &mut Nodes, events: &[ActuatorEvent], now_ms: u64) {
    for ev in events {
        match ev {
            ActuatorEvent::Accepted { id, node, action } => {
                nodes.mark_busy(*node, *action, now_ms);
                nodes.note_action(
                    *node,
                    LastAction {
                        id: id.clone(),
                        action: *action,
                        started_ms: now_ms,
                        finished_ms: None,
                        result: None,
                    },
                );
            }
            ActuatorEvent::PressEnd { node, switch, .. } => {
                if *switch == Switch::Rst {
                    nodes.note_reset(*node, now_ms);
                }
            }
            ActuatorEvent::Done {
                id,
                node,
                action,
                result,
            } => {
                nodes.clear_busy(*node);
                let started = nodes
                    .last_action(*node)
                    .filter(|a| a.id == *id)
                    .map_or(now_ms, |a| a.started_ms);
                nodes.note_action(
                    *node,
                    LastAction {
                        id: id.clone(),
                        action: *action,
                        started_ms: started,
                        finished_ms: Some(now_ms),
                        result: Some(result.as_str()),
                    },
                );
            }
            _ => {}
        }
    }
}

/// Convenience for publishers: is this node in a state where an action
/// would be refused for being unknown?
pub fn refuses_unknown(state: &NodeState) -> bool {
    matches!(state, NodeState::Unknown)
}
