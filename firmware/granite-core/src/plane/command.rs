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
//! | off | 0x51 node off, timeout [`OFF_TIMEOUT_S`], escalate 3, then the conditional cut |
//! | force_off | 0x51 node off, timeout 0, escalate 3, then the rail |
//! | force_button_off | 0x51 node off, timeout 0, escalate 1: the hold and nothing else |
//! | force_power_off | 0x40 enable off: a hard cut ([`HARD_CUT_TEXT`]) |
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
//! The plane's own cut depends on the host LED, and an open front-panel
//! cable reads exactly like a dark LED, so neither `off` nor `force_off`
//! ends at the plane's answer: each gets a second stage that cuts the
//! rail itself, under a [`StageCondition`] the client evaluates against
//! the slot's own telemetry (ADR 0004, "`force_off` always ends with the
//! rail down"). `force_button_off` and `force_power_off` are the two
//! halves of `force_off` an operator can ask for explicitly: the hold
//! alone, and the rail alone.
//!
//! A node whose settings say it has no front panel
//! ([`NodeOptions::front_panel`]) gets another table, because there is
//! no button to press and no LED to wait for (ADR 0004, "Nodes without a
//! front panel"):
//!
//! | Verb | Plane command |
//! |---|---|
//! | on | 0x40 enable on |
//! | off, force_off | 0x40 enable off: a hard cut ([`HARD_CUT_TEXT`]) |
//! | cycle | 0x40 off, [`CYCLE_OFF_S`], 0x40 on, as two stages |
//! | on_all | 0x4B stagger as usual, then 0x40 enable on per slot |
//! | reset, press | refused by the dispatcher, never a job |
//!
//! The plane is told nothing about any of this: the choice of command is
//! the whole mechanism, so plane-logic and ADR 0003 do not change.
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

/// How long a slot has to keep drawing more than
/// `nodes.host_running_w` before `off` takes the host to be still
/// running and cuts the rail \[ms\] (ADR 0004, "`force_off` always ends
/// with the rail down").
///
/// Two seconds is two SUMMARY reads at the client's 1 Hz poll, so the
/// decision rests on more than one sample and on telemetry the client
/// polls anyway.
pub const HOST_RUNNING_CONFIRM_MS: u64 = 2_000;

/// The default of `nodes.host_running_w` \[0.1 W\]: 15.0 W.
///
/// Above a host's standby draw plus the module's own idle loss (about
/// 2 W for a PM-19V-250W-B) and below the lightest running host. The
/// host-detect threshold is far too low for this: a host in standby with
/// a BMC draws 5-15 W.
pub const DEFAULT_HOST_RUNNING_DW: u16 = 150;

/// What a job's stage asks the client to check before it goes out
/// (ADR 0004, "`force_off` always ends with the rail down").
///
/// Almost every stage is [`StageCondition::Always`]: the writes go out
/// as soon as the stage before them finished. The two cut stages are
/// not, because whether the controller has to cut a rail at all is a
/// question about the slot, and only the client holds the answer - it is
/// the one reading SUMMARY. The condition says what to look for; the
/// client's stage watch (see `client::StageWatch`) waits for telemetry
/// newer than the stage it follows and then either queues the writes or
/// ends the job without them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageCondition {
    /// Nothing to check: the stage goes out at once.
    Always,
    /// Only if the slot still reports its rail enabled: the second stage
    /// of `force_off`. A plane that cut the rail itself (the host LED
    /// was still lit after the hold) leaves nothing to do.
    RailStillUp,
    /// Only if the rail is up, the LED is dark and the slot keeps
    /// drawing more than `nodes.host_running_w` for
    /// [`HOST_RUNNING_CONFIRM_MS`]: the second stage of `off`.
    ///
    /// A threshold of `0` turns the check off, and so leaves the rail
    /// up. `off` is the graceful verb, so every sample has to agree: a
    /// draw that falls, a rail that goes down or an LED that lights
    /// ends the job without a cut.
    HostStillDrawing,
}

impl StageCondition {
    /// True for the one condition that needs no telemetry.
    pub const fn is_always(self) -> bool {
        matches!(self, StageCondition::Always)
    }

    /// True when the stage runs even though the stage before it did not
    /// end Ok.
    ///
    /// `force_off` promises the rail, not the plane's opinion of it:
    /// its cut stage follows the node-off sequence whatever that
    /// answered. `off` only looks at a slot whose sequence ended Ok.
    pub const fn runs_after_failure(self) -> bool {
        matches!(self, StageCondition::RailStillUp)
    }

    /// True when the stage goes out anyway if no fresh telemetry ever
    /// arrives.
    ///
    /// `force_off` may not be fooled by the cabling, so a slot it
    /// cannot read is cut; `off` may not cut a host on a guess, so it
    /// leaves the rail up.
    pub const fn cuts_when_unreadable(self) -> bool {
        matches!(self, StageCondition::RailStillUp)
    }
}

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
    /// Whether the node has a front panel at all (ADR 0004, "Nodes
    /// without a front panel"). `false` turns every verb that would
    /// press a button into a rail-only plane command; the plane is told
    /// nothing, because the choice of command is the whole mechanism.
    pub front_panel: bool,
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
        front_panel: true,
    };

    /// The options one node's configured settings ask for.
    ///
    /// `front_panel = false` ignores `led_sense`: a node with no
    /// front-panel cable has no LED to trust, and the LED timeout goes
    /// to zero because no verb of its own waits for a light.
    pub fn of_settings(settings: &NodeSettings) -> Self {
        let front_panel = settings.front_panel;
        NodeOptions {
            led_timeout_s: if settings.led_sense && front_panel {
                ON_TIMEOUT_S
            } else {
                0
            },
            soft_off_s: settings.t_soft_off_s,
            escalate: settings.escalation.as_wire(),
            front_panel,
        }
    }

    /// The same options for a node that has no front panel.
    pub const fn without_front_panel(mut self) -> Self {
        self.front_panel = false;
        self.led_timeout_s = 0;
        self
    }
}

/// What an ack, an HTTP job result and an `action_done` event say about a
/// verb that cut a node's rail with no shutdown: `off`, `force_off` and
/// `cycle` on a node with no front panel (ADR 0004, "Nodes without a
/// front panel") and `force_power_off` on any node (ADR 0004, "Two
/// explicit verbs").
pub const HARD_CUT_TEXT: &str = "rail cut, no shutdown";

/// How long a conditional stage waits for telemetry newer than the
/// stage before it \[ms\].
///
/// SUMMARY arrives once a second per plane, so this is three chances at
/// a fresh reading. What happens when none comes is the verb's to
/// decide: `force_off` cuts anyway (it may not be fooled by a cable),
/// `off` does not (it may not cut a host on a guess).
pub const STAGE_WATCH_MS: u64 = 3_000;

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
    /// Hold PWR five seconds; cut the rail if the host is still lit,
    /// and cut it from here if the slot is still enabled afterwards.
    ForceOff {
        /// Node id 1-64.
        node: NodeId,
        /// Timings and escalation from the node's settings.
        opts: NodeOptions,
    },
    /// Hold PWR five seconds and nothing else: the rail is left alone,
    /// whatever the LED says (ADR 0004, "Two explicit verbs").
    ForceButtonOff {
        /// Node id 1-64.
        node: NodeId,
        /// Timings from the node's settings; the escalation is the
        /// hold, whatever the node configured.
        opts: NodeOptions,
    },
    /// Cut the rail now, no button: the same write as `power_off`,
    /// reported as a hard cut (ADR 0004, "Two explicit verbs").
    ForcePowerOff {
        /// Node id 1-64.
        node: NodeId,
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
        /// Of those, the ones with no front panel: they get the rail
        /// (0x40 enable) instead of a node-on, because there is no
        /// button to press and no LED to wait for (ADR 0004, "Nodes
        /// without a front panel").
        rail_only: Vec<NodeId>,
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

    /// `force_button_off` with the default timings.
    pub const fn force_button_off(node: NodeId) -> Self {
        NodeCommand::ForceButtonOff {
            node,
            opts: NodeOptions::DEFAULT,
        }
    }

    /// `force_power_off`: the rail, now.
    pub const fn force_power_off(node: NodeId) -> Self {
        NodeCommand::ForcePowerOff { node }
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
            rail_only: Vec::new(),
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
            rail_only: Vec::new(),
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
            NodeCommand::ForceButtonOff { node, .. } => NodeCommand::ForceButtonOff { node, opts },
            NodeCommand::Cycle { node, .. } => NodeCommand::Cycle { node, opts },
            NodeCommand::OnAll {
                nodes,
                stagger,
                rail_only,
                ..
            } => NodeCommand::OnAll {
                nodes,
                stagger,
                rail_only,
                opts,
            },
            other => other,
        }
    }

    /// `on_all` where some of the set have no front panel: those slots
    /// get the rail and nothing else (ADR 0004, "Nodes without a front
    /// panel").
    pub fn with_rail_only(self, rail_only: Vec<NodeId>) -> Self {
        match self {
            NodeCommand::OnAll {
                nodes,
                stagger,
                opts,
                ..
            } => NodeCommand::OnAll {
                nodes,
                stagger,
                rail_only,
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
            NodeCommand::ForceButtonOff { .. } => "force_button_off",
            NodeCommand::ForcePowerOff { .. } => "force_power_off",
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
            | NodeCommand::ForceButtonOff { node, .. }
            | NodeCommand::ForcePowerOff { node }
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
    /// `off` on a node with a front panel is not listed: whether it ends
    /// with the rail cut depends on the host, so the client decides from
    /// the slot's state once the sequence has finished. On a node
    /// without one it *is* the rail cut ([`Self::hard_cut`]), so the
    /// intent is certain.
    ///
    /// `force_off` is listed whatever the cabling: it ends with the rail
    /// down or it does not end Ok (ADR 0004, "`force_off` always ends
    /// with the rail down"), and a job that ended with the rail cut has
    /// to leave the node in the intended-OFF set. `force_button_off`
    /// never touches the rail, so it is not here.
    pub const fn intends_rail_off(&self) -> bool {
        match self {
            NodeCommand::PowerOff { .. }
            | NodeCommand::ForcePowerOff { .. }
            | NodeCommand::ForceOff { .. } => true,
            NodeCommand::Off { opts, .. } => !opts.front_panel,
            _ => false,
        }
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
    ///
    /// Only `off` on a node with a front panel: it cuts the rail when
    /// the slot keeps drawing (the [`StageCondition::HostStillDrawing`]
    /// stage) and leaves it up otherwise, and which of the two happened
    /// is the slot's answer, not the verb's. `force_off` is certain
    /// ([`Self::intends_rail_off`]).
    pub const fn may_cut_rail(&self) -> bool {
        match self {
            NodeCommand::Off { opts, .. } => opts.front_panel,
            _ => false,
        }
    }

    /// True for a verb that takes the host's power away without even
    /// trying a shutdown: `off`, `force_off` and `cycle` on a node with
    /// no front panel (ADR 0004, "Nodes without a front panel") and
    /// `force_power_off` on any node (ADR 0004, "Two explicit verbs").
    ///
    /// The ack, the HTTP job result and the `action_done` event say so,
    /// with [`HARD_CUT_TEXT`]. `power_off` is not listed: a verb that
    /// names the rail is the operator asking for exactly this.
    /// `force_off` on a node with a front panel is not either - it holds
    /// PWR first, and that it had to cut afterwards is `rail_cut`
    /// ([`FinishedJob::rail_cut`]), not a hard cut.
    pub const fn hard_cut(&self) -> bool {
        match self {
            NodeCommand::ForcePowerOff { .. } => true,
            NodeCommand::Off { opts, .. }
            | NodeCommand::ForceOff { opts, .. }
            | NodeCommand::Cycle { opts, .. } => !opts.front_panel,
            _ => false,
        }
    }

    /// What the client has to check before stage `stage` of this command
    /// goes out (ADR 0004, "`force_off` always ends with the rail
    /// down").
    ///
    /// Only the cut stages of `off` and `force_off` on a node with a
    /// front panel have one; every other stage of every other verb goes
    /// out as soon as the stage before it finished.
    pub const fn stage_condition(&self, stage: usize) -> StageCondition {
        match self {
            NodeCommand::ForceOff { opts, .. } if opts.front_panel && stage == 1 => {
                StageCondition::RailStillUp
            }
            NodeCommand::Off { opts, .. }
                if opts.front_panel
                    && stage == 1
                    && matches!(opts.escalate, Escalate::HoldPwrThenCutRail) =>
            {
                StageCondition::HostStillDrawing
            }
            _ => StageCondition::Always,
        }
    }

    /// How long the client waits after the previous stage before stage
    /// `stage` of this command goes out \[ms\].
    ///
    /// Zero for everything the plane sequences itself. The one exception
    /// is `cycle` on a node without a front panel: no plane command cuts
    /// a rail, waits and brings it back without a press, so the
    /// controller runs the off, the [`CYCLE_OFF_S`] wait and the on as
    /// one job of two stages.
    pub const fn stage_delay_ms(&self, stage: usize) -> u64 {
        match self {
            NodeCommand::Cycle { opts, .. } if !opts.front_panel && stage == 1 => {
                CYCLE_OFF_S as u64 * 1_000
            }
            _ => 0,
        }
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
        // A node with no front panel runs the rail and nothing else: one
        // enable write, or two with the cycle's wait between them.
        if let NodeCommand::On { opts, .. }
        | NodeCommand::Off { opts, .. }
        | NodeCommand::ForceOff { opts, .. }
        | NodeCommand::Cycle { opts, .. } = self
            && !opts.front_panel
        {
            return self.stage_delay_ms(1) + SEQUENCE_SLACK_MS;
        }
        // What a conditional cut stage adds: the wait for telemetry
        // newer than the sequence, the confirm window where there is
        // one, and the cut itself.
        let watch = STAGE_WATCH_MS + CUT_SETTLE_MS;
        match self {
            // Wait for the LED, press PWR, wait again (CTRL 0x50).
            NodeCommand::On { opts, .. } => 2 * led(opts) + press + SEQUENCE_SLACK_MS,
            // Short press, wait for the host, hold PWR, cut the rail,
            // then the controller's own look at the slot's draw.
            NodeCommand::Off { opts, .. } => {
                press + u64::from(opts.soft_off_s) * 1_000 + HOLD_MS + CUT_SETTLE_MS
                    + HOST_RUNNING_CONFIRM_MS
                    + watch
                    + SEQUENCE_SLACK_MS
            }
            // The same without the wait: the hold starts at once, and
            // the rail comes down from here if it is still up.
            NodeCommand::ForceOff { .. } => HOLD_MS + CUT_SETTLE_MS + watch + SEQUENCE_SLACK_MS,
            // The hold and nothing else.
            NodeCommand::ForceButtonOff { .. } => HOLD_MS + SEQUENCE_SLACK_MS,
            NodeCommand::ForcePowerOff { .. } => SEQUENCE_SLACK_MS,
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
        // A node with no front panel: the verbs of decision 4 become the
        // rail (ADR 0004, "Nodes without a front panel"). `cycle` is two
        // stages with [`CYCLE_OFF_S`] between them
        // ([`Self::stage_delay_ms`]); `reset` and `press` never get this
        // far, because the dispatcher refuses them.
        if let NodeCommand::On { opts, .. }
        | NodeCommand::Off { opts, .. }
        | NodeCommand::ForceOff { opts, .. }
        | NodeCommand::Cycle { opts, .. } = self
            && !opts.front_panel
        {
            let enable = |on: bool| alloc::vec![Step::one(address, Ctrl::Enable { slot, on })];
            return match self {
                NodeCommand::On { .. } => alloc::vec![enable(true)],
                NodeCommand::Cycle { .. } => alloc::vec![enable(false), enable(true)],
                _ => alloc::vec![enable(false)],
            };
        }
        // `off` and `force_off` on a node with a front panel end with a
        // cut stage the client only sends if the slot asks for it
        // ([`Self::stage_condition`]): the plane's own cut reads the
        // host LED, and an open front-panel cable reads like a dark LED
        // (ADR 0004, "`force_off` always ends with the rail down").
        let cut = || alloc::vec![Step::one(address, Ctrl::Enable { slot, on: false })];
        match self {
            NodeCommand::ForceOff { opts, .. } => {
                return alloc::vec![
                    alloc::vec![Step::one(
                        address,
                        Ctrl::NodeOff {
                            slot,
                            timeout_s: 0,
                            escalate: opts.escalate,
                        }
                    )],
                    cut(),
                ];
            }
            NodeCommand::Off { opts, .. } => {
                let first = alloc::vec![Step::one(
                    address,
                    Ctrl::NodeOff {
                        slot,
                        timeout_s: opts.soft_off_s,
                        escalate: opts.escalate,
                    }
                )];
                // Escalation `none` or `hold` asked for no cut, so the
                // verb is the one write it always was.
                return if matches!(opts.escalate, Escalate::HoldPwrThenCutRail) {
                    alloc::vec![first, cut()]
                } else {
                    alloc::vec![first]
                };
            }
            _ => {}
        }
        let command = match self {
            NodeCommand::On { opts, .. } => Ctrl::NodeOn {
                slot,
                led_timeout_s: opts.led_timeout_s,
            },
            // The hold, with the plane's escalation pinned to it: this
            // verb never cuts a rail, whatever the node configured.
            NodeCommand::ForceButtonOff { .. } => Ctrl::NodeOff {
                slot,
                timeout_s: 0,
                escalate: Escalate::HoldPwr,
            },
            NodeCommand::ForcePowerOff { .. } => Ctrl::Enable { slot, on: false },
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
            // `on_all` staggers; `off` and `force_off` carry the cut
            // stage and both are built above.
            NodeCommand::OnAll { .. } | NodeCommand::Off { .. } | NodeCommand::ForceOff { .. } => {
                unreachable!("handled above")
            }
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
        let (opts, cut, rail_only) = match self {
            NodeCommand::OnAll {
                opts,
                stagger,
                rail_only,
                ..
            } => (*opts, stagger.as_slice(), rail_only.as_slice()),
            _ => (NodeOptions::default(), &[] as &[NodeId], &[] as &[NodeId]),
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
            // No front panel: the rail is the whole verb for this slot.
            on.push(if rail_only.contains(&node) {
                Step::one(address, Ctrl::Enable { slot, on: true })
            } else {
                Step::one(
                    address,
                    Ctrl::NodeOn {
                        slot,
                        led_timeout_s: opts.led_timeout_s,
                    },
                )
            });
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

    /// How long the client waits before stage `stage` goes out \[ms\]
    /// (see [`NodeCommand::stage_delay_ms`]).
    pub const fn stage_delay_ms(&self, stage: usize) -> u64 {
        match self {
            Command::Node(c) => c.stage_delay_ms(stage),
            Command::Plane(_) => 0,
        }
    }

    /// What the client checks before stage `stage` goes out (see
    /// [`NodeCommand::stage_condition`]).
    pub const fn stage_condition(&self, stage: usize) -> StageCondition {
        match self {
            Command::Node(c) => c.stage_condition(stage),
            Command::Plane(_) => StageCondition::Always,
        }
    }

    /// True when it cut a node's rail with no shutdown (see
    /// [`NodeCommand::hard_cut`]).
    pub const fn hard_cut(&self) -> bool {
        match self {
            Command::Node(c) => c.hard_cut(),
            Command::Plane(_) => false,
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
    /// The controller cut the slot's rail itself, as the second stage of
    /// an `off` or a `force_off` whose plane sequence left it up
    /// (ADR 0004, "`force_off` always ends with the rail down").
    ///
    /// Reported as `"rail_cut": true` in the ack, the HTTP job result
    /// and the `action_done` event. Not the same as
    /// [`NodeCommand::hard_cut`]: a hard cut never tried a shutdown,
    /// while this one held PWR first and then found the rail still up.
    pub rail_cut: bool,
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
        // And both end with the cut stage the client only sends if the
        // slot asks for it.
        assert_eq!(off.len(), 2);
        assert_eq!(force.len(), 2);
        assert_eq!(off[1][0].commands[0], Ctrl::Enable { slot: 0, on: false });
        assert_eq!(
            force[1][0].commands[0],
            Ctrl::Enable { slot: 0, on: false }
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

    /// ADR 0004, "Nodes without a front panel": the verbs of decision 4
    /// become rail-only plane commands, and nothing presses a button.
    #[test]
    fn a_node_without_a_front_panel_gets_the_rail_only() {
        let opts = NodeOptions::DEFAULT.without_front_panel();
        assert_eq!(opts.led_timeout_s, 0, "there is no LED to wait for");

        // Node 2 is plane 0x30, slot 1.
        let on = NodeCommand::on(2).with_opts(opts).stages();
        assert_eq!(on.len(), 1);
        assert_eq!(
            on[0][0].commands[0],
            Ctrl::Enable { slot: 1, on: true },
            "`on` is the rail, not the node-on sequence"
        );

        for verb in [NodeCommand::off(2), NodeCommand::force_off(2)] {
            let stages = verb.with_opts(opts).stages();
            assert_eq!(stages.len(), 1);
            assert_eq!(
                stages[0][0].commands[0],
                Ctrl::Enable { slot: 1, on: false }
            );
        }

        // `cycle` is the controller's own two stages with the cycle's
        // off delay between them.
        let cycle = NodeCommand::cycle(2).with_opts(opts);
        let stages = cycle.stages();
        assert_eq!(stages.len(), 2);
        assert_eq!(
            stages[0][0].commands[0],
            Ctrl::Enable { slot: 1, on: false }
        );
        assert_eq!(stages[1][0].commands[0], Ctrl::Enable { slot: 1, on: true });
        assert_eq!(cycle.stage_delay_ms(0), 0);
        assert_eq!(cycle.stage_delay_ms(1), u64::from(CYCLE_OFF_S) * 1_000);
        assert!(cycle.worst_case_ms() > u64::from(CYCLE_OFF_S) * 1_000);

        // The three of them are a hard cut; `on` and `power_off` are
        // not, and a node that has a front panel never is.
        assert!(NodeCommand::off(2).with_opts(opts).hard_cut());
        assert!(NodeCommand::force_off(2).with_opts(opts).hard_cut());
        assert!(cycle.hard_cut());
        assert!(!NodeCommand::on(2).with_opts(opts).hard_cut());
        assert!(!NodeCommand::PowerOff { node: 2 }.hard_cut());
        assert!(!NodeCommand::off(2).hard_cut());

        // A rail cut is a certain rail-off intent, not a "look at the
        // slot afterwards" (see `super::intent`).
        assert!(NodeCommand::off(2).with_opts(opts).intends_rail_off());
        assert!(!NodeCommand::off(2).with_opts(opts).may_cut_rail());
        assert!(NodeCommand::off(2).may_cut_rail(), "with a front panel");

        // The verbs that were rail-only all along do not change, and a
        // rail-only `on` still asks for the rail.
        assert_eq!(
            NodeCommand::PowerOn { node: 2 }.stages()[0][0].commands[0],
            Ctrl::Enable { slot: 1, on: true }
        );
        assert!(NodeCommand::on(2).with_opts(opts).intends_rail_on());
    }

    /// `on_all` keeps its stagger; the nodes with no front panel get the
    /// rail where the others get a node-on.
    #[test]
    fn on_all_gives_a_rail_only_node_the_rail() {
        let stages = NodeCommand::on_all_of(alloc::vec![1, 2], alloc::vec![1, 2])
            .with_rail_only(alloc::vec![2])
            .stages();
        assert_eq!(stages.len(), 2);
        assert_eq!(
            stages[0][0].commands[0],
            Ctrl::StaggerOn {
                slots: IndexBitmap::I0 | IndexBitmap::I1,
                delay_ms: STAGGER_MS
            },
            "both slots are staggered, front panel or not"
        );
        assert_eq!(
            stages[1][0].commands[0],
            Ctrl::NodeOn {
                slot: 0,
                led_timeout_s: ON_TIMEOUT_S
            }
        );
        assert_eq!(
            stages[1][1].commands[0],
            Ctrl::Enable { slot: 1, on: true },
            "node 2 has no button and no LED"
        );
    }

    /// ADR 0004, "`force_off` always ends with the rail down": the cut
    /// stage and the condition the client checks before it sends it.
    #[test]
    fn the_cut_stage_is_conditional() {
        // `force_off`: the rail, if the slot still says it is enabled.
        let force = NodeCommand::force_off(1);
        assert_eq!(force.stage_condition(0), StageCondition::Always);
        assert_eq!(force.stage_condition(1), StageCondition::RailStillUp);
        assert!(force.stage_condition(1).runs_after_failure());
        assert!(force.stage_condition(1).cuts_when_unreadable());

        // `off`: the rail, if the host keeps drawing behind a dark LED.
        let off = NodeCommand::off(1);
        assert_eq!(off.stage_condition(1), StageCondition::HostStillDrawing);
        assert!(!off.stage_condition(1).runs_after_failure());
        assert!(
            !off.stage_condition(1).cuts_when_unreadable(),
            "`off` never cuts a host on a guess"
        );

        // An operator who asked for no cut gets no cut stage at all.
        for escalate in [Escalate::None, Escalate::HoldPwr] {
            let opts = NodeOptions {
                escalate,
                ..NodeOptions::DEFAULT
            };
            let held = NodeCommand::off(1).with_opts(opts);
            assert_eq!(held.stages().len(), 1, "{escalate:?} is one write");
            assert_eq!(held.stage_condition(1), StageCondition::Always);
        }

        // A node with no front panel cuts the rail as its whole verb,
        // so there is nothing conditional about it.
        let bare = NodeOptions::DEFAULT.without_front_panel();
        assert_eq!(
            NodeCommand::force_off(2).with_opts(bare).stages().len(),
            1,
            "the rail is the verb"
        );
        assert_eq!(
            NodeCommand::force_off(2)
                .with_opts(bare)
                .stage_condition(1),
            StageCondition::Always
        );

        // The deadline covers the confirm window and the watch.
        assert!(
            NodeCommand::off(1).worst_case_ms()
                > u64::from(OFF_TIMEOUT_S) * 1_000 + HOST_RUNNING_CONFIRM_MS + STAGE_WATCH_MS
        );
        assert!(NodeCommand::force_off(1).worst_case_ms() > HOLD_MS + STAGE_WATCH_MS);
    }

    /// ADR 0004, "Two explicit verbs": the two halves of `force_off` an
    /// operator can ask for by name.
    #[test]
    fn the_explicit_force_verbs_are_the_two_halves() {
        // The hold, and only the hold: escalation 1, whatever the node
        // configured, and no second stage.
        let button = NodeCommand::force_button_off(1);
        let stages = button.stages();
        assert_eq!(stages.len(), 1, "no cut stage");
        assert_eq!(
            stages[0][0].commands[0],
            Ctrl::NodeOff {
                slot: 0,
                timeout_s: 0,
                escalate: Escalate::HoldPwr
            }
        );
        assert_eq!(button.verb(), "force_button_off");
        assert!(!button.hard_cut(), "it tried a shutdown");
        // Even a node configured to cut keeps its rail under this verb.
        let cutting = NodeCommand::force_button_off(1).with_opts(NodeOptions {
            escalate: Escalate::HoldPwrThenCutRail,
            ..NodeOptions::DEFAULT
        });
        assert_eq!(
            cutting.stages()[0][0].commands[0],
            Ctrl::NodeOff {
                slot: 0,
                timeout_s: 0,
                escalate: Escalate::HoldPwr
            }
        );

        // The rail, and only the rail, reported as a hard cut.
        let rail = NodeCommand::force_power_off(1);
        let stages = rail.stages();
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0][0].commands[0], Ctrl::Enable { slot: 0, on: false });
        assert_eq!(rail.verb(), "force_power_off");
        assert!(rail.hard_cut(), "no shutdown was even tried");
        assert!(
            !NodeCommand::PowerOff { node: 1 }.hard_cut(),
            "`power_off` stays the plain rail verb"
        );
    }

    #[test]
    fn intent_follows_the_verb() {
        assert!(NodeCommand::PowerOff { node: 1 }.intends_rail_off());
        assert!(!NodeCommand::off(1).intends_rail_off());
        assert!(NodeCommand::off(1).may_cut_rail());
        // `force_off` is certain now: its cut stage ends with the rail
        // down, so the intent needs no look at the slot afterwards
        // (ADR 0004, "`force_off` always ends with the rail down").
        assert!(NodeCommand::force_off(1).intends_rail_off());
        assert!(!NodeCommand::force_off(1).may_cut_rail());
        assert!(NodeCommand::force_power_off(1).intends_rail_off());
        assert!(!NodeCommand::force_button_off(1).intends_rail_off());
        assert!(!NodeCommand::force_button_off(1).may_cut_rail());
        assert!(NodeCommand::PowerOn { node: 1 }.intends_rail_on());
        assert!(NodeCommand::on_all(alloc::vec![1]).intends_rail_on());
        assert_eq!(
            Command::Node(NodeCommand::on_all(alloc::vec![1, 9])).addresses(),
            alloc::vec![0x30, 0x31]
        );
    }
}
