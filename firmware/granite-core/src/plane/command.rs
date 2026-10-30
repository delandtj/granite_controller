//! The ADR 0004 verbs as jobs on the plane link.
//!
//! Decision 4 of controller ADR 0004 is the whole table: every rev C
//! verb keeps its name and becomes one plane command, and five new
//! verbs come with the plane. The translation lives in
//! [`NodeCommand::stages`] and [`PlaneCommand::stages`]:
//!
//! | Verb | Plane command |
//! |---|---|
//! | on | 0x50 node on, LED timeout [`ON_TIMEOUT_S`] |
//! | off | 0x51 node off, timeout [`OFF_TIMEOUT_S`], escalate 3 |
//! | force_off | 0x51 node off, timeout 0, escalate 3 |
//! | reset | 0x46 press RST |
//! | cycle | 0x47 power-cycle |
//! | press | 0x45 / 0x46 press PWR / RST |
//! | on_all | 0x4B stagger on over the rails that are down, then 0x50 node on per slot |
//! | power_on / power_off | 0x40 enable |
//! | clear_latch | 0x41 clear the OV latch |
//! | unit_reset | 0x4A unit hot-swap reset |
//! | plane_reboot | 0x4C reboot |
//!
//! `force_off` is `off` with the timeout at zero: the plane skips the
//! short press, holds PWR for five seconds and cuts the rail if the
//! host is still lit, so the node always ends up off (ADR 0004
//! decision 4, user 2026-10-29).
//!
//! A job is one or more **stages**, each a list of per-plane CTRL
//! writes. Almost every verb is one stage of one write; `on_all` is
//! two, because the stagger has to finish before each slot gets its
//! PWR press (one, when no rail needs staggering). A stage ends when every write in it has a final result,
//! and the job's outcome is the worst of them (ADR 0004 decision 6:
//! commands complete asynchronously and the ack follows the plane's
//! result).

use alloc::vec::Vec;

use granite_proto::{
    Command as Ctrl, Escalate, IndexBitmap, PRESS_MS_MAX, PRESS_MS_MIN, ResultCode,
};

use crate::NodeId;
use crate::hal::Switch;
use crate::node::NodeSettings;

use super::node_location;

/// How long the plane waits for the host's LED in `on` and in the `on`
/// half of `cycle` \[s\].
///
/// This is patience for the LED, not for a boot: a board lights its
/// power LED a second or two after its rail comes up or its button is
/// pressed. The plane waits this long for the light, presses PWR once
/// if the host stayed dark, and waits again (ADR 0003 CTRL 0x50), so a
/// node whose rail was already up and whose host is down gets its
/// press this many seconds after the verb.
pub const ON_TIMEOUT_S: u16 = 15;

/// How long `off` waits for the host to go dark before it escalates
/// \[s\] (the rev C soft-off window).
pub const OFF_TIMEOUT_S: u16 = 120;

/// How long `cycle` leaves the rail down \[s\].
pub const CYCLE_OFF_S: u16 = 5;

/// Default press of `reset` and of a bare `press` \[ms\].
pub const PRESS_MS: u16 = 300;

/// Delay between slots of a staggered `on_all` \[ms\].
pub const STAGGER_MS: u16 = 2_000;

/// How long the plane holds PWR when `off` or `force_off` escalates
/// \[ms\] (ADR 0003 CTRL 0x51 escalate 1 and 3, "hold PWR 5 s"; the
/// plane's own `HOLD_PRESS_MS`).
pub const HOLD_MS: u64 = 5_000;

/// How long the plane waits after cutting a rail before it calls the
/// node off \[ms\] (the plane's `CUT_SETTLE_MS`).
pub const CUT_SETTLE_MS: u64 = 1_000;

/// What a sequence may take beyond the waits its options spell out
/// \[ms\]: the plane's own 20 Hz cycle, the link retries of ADR 0003
/// decision 4, and the controller's poll period.
///
/// Not a timing requirement - only slack, so that a deadline derived
/// from a node's settings is never tighter than the sequence it has to
/// cover.
pub const SEQUENCE_SLACK_MS: u64 = 5_000;

/// A job identifier: unique for the life of the client.
pub type JobId = u32;

/// One CTRL write of a job: the commands that go to one plane under
/// one sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The plane's address.
    pub address: u8,
    /// The commands, in wire order.
    pub commands: Vec<Ctrl>,
    /// A result is read back for this write.
    ///
    /// False for the reboot: the plane answers `ok` and resets inside
    /// the same cycle, so the result can never be read. A delivered
    /// write is the whole outcome there.
    pub awaits_result: bool,
}

impl Step {
    /// One command to one plane, whose result is read back.
    pub fn one(address: u8, command: Ctrl) -> Self {
        Step {
            address,
            commands: alloc::vec![command],
            awaits_result: true,
        }
    }

    /// One command whose outcome is the delivery of the write.
    pub fn fire_and_forget(address: u8, command: Ctrl) -> Self {
        Step {
            awaits_result: false,
            ..Step::one(address, command)
        }
    }
}

/// The per-node settings a verb carries onto the plane (ADR 0004
/// decisions 2 and 4).
///
/// The plane runs the waiting and the escalation, so the controller only
/// has to hand it the numbers the operator configured: how long to wait
/// for the host LED, how long a soft off waits before it escalates, and
/// how far it goes. [`Default`] is the ADR's: wait
/// [`ON_TIMEOUT_S`] for the light, [`OFF_TIMEOUT_S`] for the host, then
/// hold PWR and cut the rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeOptions {
    /// How long the plane waits for the host LED \[s\]. `0` means the
    /// LED is not trusted on this node: no waiting.
    pub led_timeout_s: u16,
    /// How long `off` waits for the host to go dark \[s\].
    pub soft_off_s: u16,
    /// How far `off` and `force_off` escalate.
    pub escalate: Escalate,
}

impl Default for NodeOptions {
    fn default() -> Self {
        NodeOptions::DEFAULT
    }
}

impl NodeOptions {
    /// The ADR defaults, usable in a `const` context.
    pub const DEFAULT: NodeOptions = NodeOptions {
        led_timeout_s: ON_TIMEOUT_S,
        soft_off_s: OFF_TIMEOUT_S,
        escalate: Escalate::HoldPwrThenCutRail,
    };

    /// The options one node's configured settings ask for.
    pub fn of_settings(settings: &NodeSettings) -> Self {
        NodeOptions {
            led_timeout_s: if settings.led_sense { ON_TIMEOUT_S } else { 0 },
            soft_off_s: settings.t_soft_off_s,
            escalate: settings.escalation.as_wire(),
        }
    }
}

/// What a command on one node or a set of them asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeCommand {
    /// Rail on if needed, then PWR if the host stays dark.
    On {
        /// Node id 1-64.
        node: NodeId,
        /// Timings and escalation from the node's settings.
        opts: NodeOptions,
    },
    /// Soft off: short PWR, wait, then escalate.
    Off {
        /// Node id 1-64.
        node: NodeId,
        /// Timings and escalation from the node's settings.
        opts: NodeOptions,
    },
    /// Hold PWR five seconds; cut the rail if the host is still lit.
    ForceOff {
        /// Node id 1-64.
        node: NodeId,
        /// Timings and escalation from the node's settings.
        opts: NodeOptions,
    },
    /// Press RST.
    Reset {
        /// Node id 1-64.
        node: NodeId,
    },
    /// Rail off, wait, rail on, PWR if the host stays dark.
    Cycle {
        /// Node id 1-64.
        node: NodeId,
        /// Timings and escalation from the node's settings.
        opts: NodeOptions,
    },
    /// A raw press of one button.
    Press {
        /// Node id 1-64.
        node: NodeId,
        /// PWR or RST.
        switch: Switch,
        /// Hold time \[ms\], 50-10000.
        ms: u16,
    },
    /// Staggered `on` over a set of nodes, grouped per plane.
    OnAll {
        /// The nodes to bring up, in any order; duplicates are ignored.
        nodes: Vec<NodeId>,
        /// Of those, the ones whose rail is down: the only slots the
        /// plane's stagger (0x4B) may name. 0x4B takes every slot it is
        /// given off before releasing them one by one (ADR 0002
        /// decision 6), so a node that is already powered has to stay
        /// out of it - `on_all` is a staggered `on` (ADR 0004
        /// decision 4), not a power cycle.
        stagger: Vec<NodeId>,
        /// Timings of the nodes in the set. One set of options for the
        /// group: a staggered `on_all` is one plane command per plane
        /// and cannot carry a timeout per slot.
        opts: NodeOptions,
    },
    /// The rail only, no buttons.
    PowerOn {
        /// Node id 1-64.
        node: NodeId,
    },
    /// The rail only, no buttons.
    PowerOff {
        /// Node id 1-64.
        node: NodeId,
    },
    /// Clear the slot's overvoltage latch. Never automatic.
    ClearLatch {
        /// Node id 1-64.
        node: NodeId,
    },
}

impl NodeCommand {
    /// `on` with the default timings.
    pub const fn on(node: NodeId) -> Self {
        NodeCommand::On {
            node,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// `off` with the default timings.
    pub const fn off(node: NodeId) -> Self {
        NodeCommand::Off {
            node,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// `force_off` with the default escalation.
    pub const fn force_off(node: NodeId) -> Self {
        NodeCommand::ForceOff {
            node,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// `cycle` with the default timings.
    pub const fn cycle(node: NodeId) -> Self {
        NodeCommand::Cycle {
            node,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// `on_all` over a set of nodes with the default timings, staggering
    /// every one of them: what a caller that knows nothing about the
    /// rails asks for.
    pub fn on_all(nodes: Vec<NodeId>) -> Self {
        NodeCommand::OnAll {
            stagger: nodes.clone(),
            nodes,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// `on_all` where the caller knows which rails are down: only those
    /// slots are staggered, the rest are left powered and only get their
    /// `on` (ADR 0004 decision 4).
    pub fn on_all_of(nodes: Vec<NodeId>, stagger: Vec<NodeId>) -> Self {
        NodeCommand::OnAll {
            nodes,
            stagger,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// The same verb with other timings: what the dispatcher does with
    /// the node's configured settings.
    pub fn with_opts(self, opts: NodeOptions) -> Self {
        match self {
            NodeCommand::On { node, .. } => NodeCommand::On { node, opts },
            NodeCommand::Off { node, .. } => NodeCommand::Off { node, opts },
            NodeCommand::ForceOff { node, .. } => NodeCommand::ForceOff { node, opts },
            NodeCommand::Cycle { node, .. } => NodeCommand::Cycle { node, opts },
            NodeCommand::OnAll {
                nodes, stagger, ..
            } => NodeCommand::OnAll {
                nodes,
                stagger,
                opts,
            },
            other => other,
        }
    }

    /// Lowercase wire name, the rev C verb where there is one.
    pub const fn verb(&self) -> &'static str {
        match self {
            NodeCommand::On { .. } => "on",
            NodeCommand::Off { .. } => "off",
            NodeCommand::ForceOff { .. } => "force_off",
            NodeCommand::Reset { .. } => "reset",
            NodeCommand::Cycle { .. } => "cycle",
            NodeCommand::Press { .. } => "press",
            NodeCommand::OnAll { .. } => "on_all",
            NodeCommand::PowerOn { .. } => "power_on",
            NodeCommand::PowerOff { .. } => "power_off",
            NodeCommand::ClearLatch { .. } => "clear_latch",
        }
    }

    /// The nodes this command acts on.
    pub fn nodes(&self) -> Vec<NodeId> {
        match self {
            NodeCommand::OnAll { nodes, .. } => {
                let mut out: Vec<NodeId> = Vec::new();
                for node in nodes {
                    if super::is_plane_node(*node) && !out.contains(node) {
                        out.push(*node);
                    }
                }
                out.sort_unstable();
                out
            }
            NodeCommand::On { node, .. }
            | NodeCommand::Off { node, .. }
            | NodeCommand::ForceOff { node, .. }
            | NodeCommand::Reset { node }
            | NodeCommand::Cycle { node, .. }
            | NodeCommand::Press { node, .. }
            | NodeCommand::PowerOn { node }
            | NodeCommand::PowerOff { node }
            | NodeCommand::ClearLatch { node } => alloc::vec![*node],
        }
    }

    /// True for a verb whose end state is "the rail is off", which is
    /// what the intended-OFF set remembers (see [`super::intent`]).
    ///
    /// `off` and `force_off` are not listed: whether they end with the
    /// rail cut depends on the host, so the client decides from the
    /// slot's state once the sequence has finished.
    pub const fn intends_rail_off(&self) -> bool {
        matches!(self, NodeCommand::PowerOff { .. })
    }

    /// True for a verb that ends with the rail up, which clears the
    /// node from the intended-OFF set.
    pub const fn intends_rail_on(&self) -> bool {
        matches!(
            self,
            NodeCommand::On { .. }
                | NodeCommand::OnAll { .. }
                | NodeCommand::PowerOn { .. }
                | NodeCommand::Cycle { .. }
                | NodeCommand::ClearLatch { .. }
        )
    }

    /// True for a verb that may end with the rail cut, so the client
    /// looks at the slot once it is done.
    pub const fn may_cut_rail(&self) -> bool {
        matches!(self, NodeCommand::Off { .. } | NodeCommand::ForceOff { .. })
    }

    /// The longest the plane can take over this command \[ms\], from the
    /// options it carries.
    ///
    /// This is what a job deadline and an ack timeout are built from
    /// (see [`super::client::job_timeout_ms`]): a node whose
    /// `t_soft_off_s` is an hour has to be allowed its hour, and a node
    /// that only wants a press must not keep an ack waiting for one.
    pub fn worst_case_ms(&self) -> u64 {
        let led = |opts: &NodeOptions| u64::from(opts.led_timeout_s) * 1_000;
        let press = u64::from(PRESS_MS);
        match self {
            // Wait for the LED, press PWR, wait again (CTRL 0x50).
            NodeCommand::On { opts, .. } => 2 * led(opts) + press + SEQUENCE_SLACK_MS,
            // Short press, wait for the host, hold PWR, cut the rail.
            NodeCommand::Off { opts, .. } => {
                press + u64::from(opts.soft_off_s) * 1_000 + HOLD_MS + CUT_SETTLE_MS
                    + SEQUENCE_SLACK_MS
            }
            // The same without the wait: the hold starts at once.
            NodeCommand::ForceOff { .. } => HOLD_MS + CUT_SETTLE_MS + SEQUENCE_SLACK_MS,
            // Rail down, the off delay, rail up, then an `on`.
            NodeCommand::Cycle { opts, .. } => {
                u64::from(CYCLE_OFF_S) * 1_000 + 2 * led(opts) + press + SEQUENCE_SLACK_MS
            }
            NodeCommand::Reset { .. } => press + SEQUENCE_SLACK_MS,
            NodeCommand::Press { ms, .. } => u64::from(*ms) + SEQUENCE_SLACK_MS,
            // The stagger walks the slots of one plane, then every node
            // runs its own `on`. Planes run side by side, so the worst
            // case is the whole set on one plane.
            NodeCommand::OnAll { nodes, opts, .. } => {
                nodes.len() as u64 * u64::from(STAGGER_MS)
                    + 2 * led(opts)
                    + press
                    + SEQUENCE_SLACK_MS
            }
            NodeCommand::PowerOn { .. }
            | NodeCommand::PowerOff { .. }
            | NodeCommand::ClearLatch { .. } => SEQUENCE_SLACK_MS,
        }
    }

    /// The stages of CTRL writes this command becomes.
    ///
    /// Empty when nothing it names is a node id; the client turns that
    /// into an immediate failure rather than a write.
    pub fn stages(&self) -> Vec<Vec<Step>> {
        if let NodeCommand::OnAll { .. } = self {
            return self.stagger_stages();
        }
        let nodes = self.nodes();
        let Some((address, slot)) = nodes.first().and_then(|n| node_location(*n)) else {
            return Vec::new();
        };
        let command = match self {
            NodeCommand::On { opts, .. } => Ctrl::NodeOn {
                slot,
                led_timeout_s: opts.led_timeout_s,
            },
            NodeCommand::Off { opts, .. } => Ctrl::NodeOff {
                slot,
                timeout_s: opts.soft_off_s,
                escalate: opts.escalate,
            },
            NodeCommand::ForceOff { opts, .. } => Ctrl::NodeOff {
                slot,
                timeout_s: 0,
                escalate: opts.escalate,
            },
            NodeCommand::Reset { .. } => Ctrl::PressRst { slot, ms: PRESS_MS },
            NodeCommand::Cycle { opts, .. } => Ctrl::PowerCycle {
                slot,
                off_delay_s: CYCLE_OFF_S,
                press_if_dark: true,
                led_timeout_s: opts.led_timeout_s,
            },
            NodeCommand::Press { switch, ms, .. } => {
                let ms = (*ms).clamp(PRESS_MS_MIN, PRESS_MS_MAX);
                match switch {
                    Switch::Pwr => Ctrl::PressPwr { slot, ms },
                    Switch::Rst => Ctrl::PressRst { slot, ms },
                }
            }
            NodeCommand::PowerOn { .. } => Ctrl::Enable { slot, on: true },
            NodeCommand::PowerOff { .. } => Ctrl::Enable { slot, on: false },
            NodeCommand::ClearLatch { .. } => Ctrl::ClearOvLatch { slot },
            NodeCommand::OnAll { .. } => unreachable!("handled above"),
        };
        alloc::vec![alloc::vec![Step::one(address, command)]]
    }

    /// `on_all`: one stagger per plane over the rails that are down,
    /// then one node-on per slot.
    ///
    /// The stagger (0x4B) takes every named slot down and brings it
    /// back [`STAGGER_MS`] apart, so the inrush of eight boards does
    /// not land at once; the node-on stage then presses PWR on the
    /// hosts that stayed dark. They have to be two stages: a node-on
    /// sent while the stagger still holds a slot off would release the
    /// rail behind the stagger's back.
    ///
    /// Only the slots in `stagger` are named to 0x4B. A node whose rail
    /// is already up gets its node-on and keeps its power: the stagger
    /// is there to spread inrush, and a slot that is already carrying a
    /// board has none left to spread.
    fn stagger_stages(&self) -> Vec<Vec<Step>> {
        let (opts, cut) = match self {
            NodeCommand::OnAll { opts, stagger, .. } => (*opts, stagger.as_slice()),
            _ => (NodeOptions::default(), &[] as &[NodeId]),
        };
        let mut stagger: Vec<Step> = Vec::new();
        let mut on: Vec<Step> = Vec::new();
        for node in self.nodes() {
            let Some((address, slot)) = node_location(node) else {
                continue;
            };
            if cut.contains(&node) {
                match stagger.iter_mut().find(|s| s.address == address) {
                    Some(step) => {
                        if let Some(Ctrl::StaggerOn { slots, .. }) = step.commands.first_mut() {
                            *slots = slots.plus(slot);
                        }
                    }
                    None => stagger.push(Step::one(
                        address,
                        Ctrl::StaggerOn {
                            slots: IndexBitmap::NONE.plus(slot),
                            delay_ms: STAGGER_MS,
                        },
                    )),
                }
            }
            on.push(Step::one(
                address,
                Ctrl::NodeOn {
                    slot,
                    led_timeout_s: opts.led_timeout_s,
                },
            ));
        }
        if on.is_empty() {
            return Vec::new();
        }
        if stagger.is_empty() {
            // Every rail is already up: nothing to spread, just the
            // presses.
            return alloc::vec![on];
        }
        alloc::vec![stagger, on]
    }
}

/// What a command on a plane or one of its units asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaneCommand {
    /// Let the plane's hold on a unit's hot-swap go after it latched a
    /// faulted unit off. A deliberate act: `on` never does it
    /// (ADR 0004, "What an Expert Would Ask"). The plane answers about
    /// a second later with whether the unit came back.
    UnitReset {
        /// The plane's address.
        address: u8,
        /// Unit 0-3.
        unit: u8,
    },
    /// Reboot the plane. The slots keep their state: the plane saves
    /// its intended state first and its backup registers survive a
    /// commanded reset (ADR 0002 decision 3).
    Reboot {
        /// The plane's address.
        address: u8,
    },
}

impl PlaneCommand {
    /// Lowercase wire name.
    pub const fn verb(&self) -> &'static str {
        match self {
            PlaneCommand::UnitReset { .. } => "unit_reset",
            PlaneCommand::Reboot { .. } => "plane_reboot",
        }
    }

    /// The longest the plane can take over this command \[ms\].
    ///
    /// A unit reset is a 100 ms pulse and a reboot never reports at all
    /// (the write's delivery is the outcome), so both are slack only.
    pub const fn worst_case_ms(&self) -> u64 {
        SEQUENCE_SLACK_MS
    }

    /// The plane this command goes to.
    pub const fn address(&self) -> u8 {
        match self {
            PlaneCommand::UnitReset { address, .. } | PlaneCommand::Reboot { address } => *address,
        }
    }

    /// The stages of CTRL writes this command becomes.
    pub fn stages(&self) -> Vec<Vec<Step>> {
        if granite_proto::plane_of_address(self.address()).is_none() {
            return Vec::new();
        }
        let step = match self {
            PlaneCommand::UnitReset { unit, .. } => {
                if (*unit as usize) >= granite_proto::UNIT_COUNT {
                    return Vec::new();
                }
                Step::one(self.address(), Ctrl::UnitHotSwapReset { unit: *unit })
            }
            // The plane resets before its result can be read.
            PlaneCommand::Reboot { .. } => Step::fire_and_forget(self.address(), Ctrl::Reboot),
        };
        alloc::vec![alloc::vec![step]]
    }
}

/// What the client was asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// A node verb.
    Node(NodeCommand),
    /// A plane or unit verb.
    Plane(PlaneCommand),
}

impl Command {
    /// Lowercase wire name of the verb.
    pub const fn verb(&self) -> &'static str {
        match self {
            Command::Node(c) => c.verb(),
            Command::Plane(c) => c.verb(),
        }
    }

    /// The nodes it acts on (none for a plane verb).
    pub fn nodes(&self) -> Vec<NodeId> {
        match self {
            Command::Node(c) => c.nodes(),
            Command::Plane(_) => Vec::new(),
        }
    }

    /// The planes it touches.
    pub fn addresses(&self) -> Vec<u8> {
        match self {
            Command::Node(c) => {
                let mut out: Vec<u8> = Vec::new();
                for node in c.nodes() {
                    if let Some((address, _)) = node_location(node)
                        && !out.contains(&address)
                    {
                        out.push(address);
                    }
                }
                out
            }
            Command::Plane(c) => alloc::vec![c.address()],
        }
    }

    /// The stages of CTRL writes it becomes.
    pub fn stages(&self) -> Vec<Vec<Step>> {
        match self {
            Command::Node(c) => c.stages(),
            Command::Plane(c) => c.stages(),
        }
    }

    /// The longest the plane can take over it \[ms\].
    pub fn worst_case_ms(&self) -> u64 {
        match self {
            Command::Node(c) => c.worst_case_ms(),
            Command::Plane(c) => c.worst_case_ms(),
        }
    }
}

impl From<NodeCommand> for Command {
    fn from(c: NodeCommand) -> Self {
        Command::Node(c)
    }
}

impl From<PlaneCommand> for Command {
    fn from(c: PlaneCommand) -> Self {
        Command::Plane(c)
    }
}

/// How a job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobOutcome {
    /// Every plane step finished with [`ResultCode::Ok`].
    Done,
    /// A plane refused it or a sequence failed; the result code says
    /// which.
    Failed,
    /// The node, unit or plane is not there. Nothing was written
    /// (ADR 0004 decision 6: those fail at once).
    Absent,
    /// The write could never be delivered, or the plane stopped
    /// answering before it reported a result.
    LinkLost,
    /// The client gave up waiting (see
    /// [`super::client::job_timeout_ms`]).
    TimedOut,
}

impl JobOutcome {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            JobOutcome::Done => "done",
            JobOutcome::Failed => "failed",
            JobOutcome::Absent => "absent",
            JobOutcome::LinkLost => "link_lost",
            JobOutcome::TimedOut => "timed_out",
        }
    }

    /// True for the one outcome that did what was asked.
    pub const fn is_ok(self) -> bool {
        matches!(self, JobOutcome::Done)
    }
}

impl core::fmt::Display for JobOutcome {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A job that has ended, waiting to be drained into an MQTT ack or an
/// HTTP job result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedJob {
    /// The id [`super::PlaneClient::submit`] gave out.
    pub id: JobId,
    /// What was asked for.
    pub command: Command,
    /// How it ended.
    pub outcome: JobOutcome,
    /// The worst result code a plane reported, if one did.
    pub code: Option<ResultCode>,
    /// When the client accepted it \[ms\].
    pub submitted_ms: u64,
    /// When it ended \[ms\].
    pub finished_ms: u64,
}

impl FinishedJob {
    /// True when the job did what it was asked.
    pub const fn is_ok(&self) -> bool {
        self.outcome.is_ok()
    }
}

/// How bad a result code is, so a stage of several writes can be
/// answered with one: an error beats "accepted", which beats "ok". The
/// plane's link handler ranks them the same way.
pub(crate) const fn severity(code: ResultCode) -> u8 {
    match code {
        ResultCode::Ok => 0,
        ResultCode::Accepted => 1,
        _ => 2,
    }
}

/// The outcome a final result code maps to.
pub(crate) const fn outcome_of(code: ResultCode) -> JobOutcome {
    match code {
        ResultCode::Ok => JobOutcome::Done,
        ResultCode::NotPresent => JobOutcome::Absent,
        _ => JobOutcome::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verbs_become_the_commands_of_the_table() {
        // Node 10 is plane 0x31, slot 1.
        assert_eq!(node_location(10), Some((0x31, 1)));
        let stages = NodeCommand::on(10).stages();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].len(), 1);
        assert_eq!(stages[0][0].address, 0x31);
        assert_eq!(
            stages[0][0].commands,
            alloc::vec![Ctrl::NodeOn {
                slot: 1,
                led_timeout_s: ON_TIMEOUT_S
            }]
        );

        // force_off is off with the timeout at zero.
        let off = NodeCommand::off(1).stages();
        let force = NodeCommand::force_off(1).stages();
        assert_eq!(
            off[0][0].commands[0],
            Ctrl::NodeOff {
                slot: 0,
                timeout_s: OFF_TIMEOUT_S,
                escalate: Escalate::HoldPwrThenCutRail
            }
        );
        assert_eq!(
            force[0][0].commands[0],
            Ctrl::NodeOff {
                slot: 0,
                timeout_s: 0,
                escalate: Escalate::HoldPwrThenCutRail
            }
        );

        // A press out of range is clamped rather than refused by the
        // plane: the verb's own range is the ADR's.
        let press = NodeCommand::Press {
            node: 2,
            switch: Switch::Rst,
            ms: 20_000,
        }
        .stages();
        assert_eq!(
            press[0][0].commands[0],
            Ctrl::PressRst {
                slot: 1,
                ms: PRESS_MS_MAX
            }
        );

        assert!(NodeCommand::on(0).stages().is_empty());
        assert!(NodeCommand::on(65).stages().is_empty());
    }

    #[test]
    fn on_all_staggers_per_plane_and_then_presses() {
        let stages = NodeCommand::on_all(alloc::vec![1, 3, 9, 3]).stages();
        assert_eq!(stages.len(), 2, "stagger, then the presses");
        assert_eq!(stages[0].len(), 2, "one stagger per plane");
        assert_eq!(stages[0][0].address, 0x30);
        assert_eq!(
            stages[0][0].commands[0],
            Ctrl::StaggerOn {
                slots: IndexBitmap::I0 | IndexBitmap::I2,
                delay_ms: STAGGER_MS
            }
        );
        assert_eq!(stages[0][1].address, 0x31);
        assert_eq!(stages[1].len(), 3, "one node-on per slot, duplicates gone");
    }

    /// Bug: `on_all` named every slot to the stagger, so a running node
    /// lost its rail and had to boot again. Only the rails that are
    /// down are staggered (ADR 0004 decision 4: a staggered `on`).
    #[test]
    fn on_all_only_staggers_the_rails_that_are_down() {
        // Nodes 1 and 3 want the verb; only node 3's rail is off.
        let stages = NodeCommand::on_all_of(alloc::vec![1, 3], alloc::vec![3]).stages();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].len(), 1, "one stagger, for one plane");
        assert_eq!(
            stages[0][0].commands[0],
            Ctrl::StaggerOn {
                slots: IndexBitmap::NONE.plus(2),
                delay_ms: STAGGER_MS
            },
            "node 1's slot is not in the bitmap"
        );
        assert_eq!(stages[1].len(), 2, "both still get their node-on");

        // Nothing to stagger: the presses alone, and no slot is cut.
        let presses = NodeCommand::on_all_of(alloc::vec![1, 3], Vec::new()).stages();
        assert_eq!(presses.len(), 1, "no stagger stage at all");
        assert_eq!(presses[0].len(), 2);
        assert!(
            presses[0]
                .iter()
                .all(|step| matches!(step.commands[0], Ctrl::NodeOn { .. }))
        );

        // And a group with nothing in it is no job.
        assert!(
            NodeCommand::on_all_of(Vec::new(), Vec::new())
                .stages()
                .is_empty()
        );
    }

    #[test]
    fn plane_verbs_check_their_arguments() {
        assert!(
            PlaneCommand::UnitReset {
                address: 0x30,
                unit: 4
            }
            .stages()
            .is_empty()
        );
        assert!(PlaneCommand::Reboot { address: 0x40 }.stages().is_empty());
        let reboot = PlaneCommand::Reboot { address: 0x33 }.stages();
        assert_eq!(reboot[0][0].commands[0], Ctrl::Reboot);
    }

    #[test]
    fn intent_follows_the_verb() {
        assert!(NodeCommand::PowerOff { node: 1 }.intends_rail_off());
        assert!(!NodeCommand::off(1).intends_rail_off());
        assert!(NodeCommand::off(1).may_cut_rail());
        assert!(NodeCommand::force_off(1).may_cut_rail());
        assert!(NodeCommand::PowerOn { node: 1 }.intends_rail_on());
        assert!(NodeCommand::on_all(alloc::vec![1]).intends_rail_on());
        assert_eq!(
            Command::Node(NodeCommand::on_all(alloc::vec![1, 9])).addresses(),
            alloc::vec![0x30, 0x31]
        );
    }
}
