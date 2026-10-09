//! Shared test rig: a fake expander, a fake clock and a scripted set of
//! power LEDs, so every action can be driven to completion in a loop.
#![allow(dead_code)]

use std::collections::BTreeSet;

use granite_core::NodeId;
use granite_core::actuator::{
    ActionRequest, ActionResult, Actuator, ActuatorEvent, Submission, Timings,
};
use granite_core::hal::{HalError, HalResult, NodeSwitches, Switch};
use granite_core::node::{NodeState, SenseMode};
use granite_core::observed::Observed;

/// Every call the actuator made on the expander.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Assert(NodeId, Switch),
    Release(NodeId, Switch),
    ReleaseAll,
    Arm(u32),
    Disarm,
}

/// A fake U14.
#[derive(Debug, Default)]
pub struct FakeSwitches {
    pub ops: Vec<Op>,
    pub closed: BTreeSet<(NodeId, u8)>,
    pub max_concurrent: usize,
    pub armed_ms: Option<u32>,
    /// When set, the next `assert` fails with this error.
    pub fail_next_assert: Option<HalError>,
}

fn sw_bit(sw: Switch) -> u8 {
    match sw {
        Switch::Pwr => 0,
        Switch::Rst => 1,
    }
}

impl NodeSwitches for FakeSwitches {
    fn assert(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        if let Some(e) = self.fail_next_assert.take() {
            return Err(e);
        }
        self.ops.push(Op::Assert(node, sw));
        self.closed.insert((node, sw_bit(sw)));
        self.max_concurrent = self.max_concurrent.max(self.closed.len());
        Ok(())
    }

    fn release(&mut self, node: NodeId, sw: Switch) -> HalResult<()> {
        self.ops.push(Op::Release(node, sw));
        self.closed.remove(&(node, sw_bit(sw)));
        Ok(())
    }

    fn release_all(&mut self) -> HalResult<()> {
        self.ops.push(Op::ReleaseAll);
        self.closed.clear();
        Ok(())
    }

    fn arm_deadline(&mut self, ms: u32) -> HalResult<()> {
        self.ops.push(Op::Arm(ms));
        self.armed_ms = Some(ms);
        Ok(())
    }

    fn disarm_deadline(&mut self) -> HalResult<()> {
        self.ops.push(Op::Disarm);
        self.armed_ms = None;
        Ok(())
    }
}

/// Tick granularity of the rig, in milliseconds.
pub const TICK_MS: u64 = 10;

/// How a node's LED reacts to being pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Respond {
    /// The LED never changes.
    Never,
    /// The LED goes on `ms` after the first press ends.
    OnAfter(u64),
    /// The LED goes off `ms` after the first press ends.
    OffAfter(u64),
    /// Off after the first press, then on after the second one. What a
    /// cooperative motherboard does during a `cycle`.
    OffThenOn(u64, u64),
}

/// The rig.
pub struct Rig {
    pub act: Actuator<FakeSwitches>,
    pub obs: Observed,
    pub now: u64,
    pub events: Vec<ActuatorEvent>,
    /// Scheduled LED changes: (at_ms, node, value).
    pub sched: Vec<(u64, NodeId, Option<bool>)>,
    /// Reaction script per node.
    respond: [Respond; 8],
    presses_seen: [usize; 8],
}

impl Rig {
    pub fn new() -> Self {
        Rig {
            act: Actuator::new(FakeSwitches::default()),
            obs: Observed::new(),
            now: 0,
            events: Vec::new(),
            sched: Vec::new(),
            respond: [Respond::Never; 8],
            presses_seen: [0; 8],
        }
    }

    pub fn timings(&self) -> Timings {
        *self.act.timings()
    }

    pub fn set_timings(&mut self, t: Timings) {
        self.act.set_timings(t);
    }

    /// Set the debounced LED of a node right now.
    pub fn set_led(&mut self, node: NodeId, value: Option<bool>) {
        let i = node as usize - 1;
        self.obs.nodes[i].led = value;
        self.obs.nodes[i].state = match value {
            None => NodeState::Unknown,
            Some(true) => NodeState::On,
            Some(false) => NodeState::Off,
        };
        self.obs.nodes[i].ts_ms = self.now;
    }

    /// Set every LED.
    pub fn set_all_leds(&mut self, value: Option<bool>) {
        for n in 1..=8 {
            self.set_led(n, value);
        }
    }

    pub fn set_sense(&mut self, node: NodeId, mode: SenseMode) {
        self.obs.nodes[node as usize - 1].sense = mode;
    }

    pub fn set_respond(&mut self, node: NodeId, r: Respond) {
        self.respond[node as usize - 1] = r;
    }

    pub fn schedule_led(&mut self, at_ms: u64, node: NodeId, value: Option<bool>) {
        self.sched.push((at_ms, node, value));
    }

    pub fn submit(&mut self, req: &ActionRequest) -> Submission {
        let (sub, mut evs) = self.act.submit(req, self.now, &self.obs);
        self.events.append(&mut evs);
        sub
    }

    /// Advance to `t_ms` in [`TICK_MS`] steps, applying the schedule and
    /// the per-node reaction script on the way.
    pub fn run_to(&mut self, t_ms: u64) {
        while self.now < t_ms {
            self.now = (self.now + TICK_MS).min(t_ms);
            self.apply_sched();
            let evs = self.act.tick(self.now, &self.obs);
            self.react(&evs);
            self.events.extend(evs);
        }
    }

    /// Advance by `ms`.
    pub fn run_for(&mut self, ms: u64) {
        self.run_to(self.now + ms);
    }

    /// Jump straight to `t_ms` with a single tick. Used to simulate a
    /// starved actuator task and blow a press deadline.
    pub fn jump_to(&mut self, t_ms: u64) {
        self.now = t_ms;
        self.apply_sched();
        let evs = self.act.tick(self.now, &self.obs);
        self.react(&evs);
        self.events.extend(evs);
    }

    fn apply_sched(&mut self) {
        let now = self.now;
        let due: Vec<(u64, NodeId, Option<bool>)> = self
            .sched
            .iter()
            .filter(|(t, ..)| *t <= now)
            .copied()
            .collect();
        self.sched.retain(|(t, ..)| *t > now);
        for (_, node, value) in due {
            self.set_led(node, value);
        }
    }

    /// Turn the reaction script into scheduled LED changes when a press
    /// ends.
    fn react(&mut self, evs: &[ActuatorEvent]) {
        for ev in evs {
            if let ActuatorEvent::PressEnd { node, .. } = ev {
                let i = *node as usize - 1;
                self.presses_seen[i] += 1;
                let nth = self.presses_seen[i];
                let now = self.now;
                match self.respond[i] {
                    Respond::Never => {}
                    Respond::OnAfter(d) => {
                        if nth == 1 {
                            self.sched.push((now + d, *node, Some(true)));
                        }
                    }
                    Respond::OffAfter(d) => {
                        if nth == 1 {
                            self.sched.push((now + d, *node, Some(false)));
                        }
                    }
                    Respond::OffThenOn(d1, d2) => {
                        if nth == 1 {
                            self.sched.push((now + d1, *node, Some(false)));
                        } else if nth == 2 {
                            self.sched.push((now + d2, *node, Some(true)));
                        }
                    }
                }
            }
        }
    }

    /// The `Done` event for a node, if the job finished.
    pub fn result_of(&self, node: NodeId) -> Option<ActionResult> {
        self.events.iter().rev().find_map(|e| match e {
            ActuatorEvent::Done {
                node: n, result, ..
            } if *n == node => Some(result.clone()),
            _ => None,
        })
    }

    /// The aggregate result of a group command.
    pub fn group_result(&self, id: &str) -> Option<ActionResult> {
        self.events.iter().rev().find_map(|e| match e {
            ActuatorEvent::GroupDone { id: i, result, .. } if i == id => Some(result.clone()),
            _ => None,
        })
    }

    /// `(node, switch, duration_ms, deadline_ms, at_ms)` is not recorded;
    /// this returns the press starts in order with their switch.
    pub fn press_starts(&self) -> Vec<(NodeId, Switch, u32, u32)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                ActuatorEvent::PressStart {
                    node,
                    switch,
                    duration_ms,
                    deadline_ms,
                    ..
                } => Some((*node, *switch, *duration_ms, *deadline_ms)),
                _ => None,
            })
            .collect()
    }

    /// Actual press lengths in order.
    pub fn press_ends(&self) -> Vec<(NodeId, Switch, u32)> {
        self.events
            .iter()
            .filter_map(|e| match e {
                ActuatorEvent::PressEnd {
                    node,
                    switch,
                    held_ms,
                    ..
                } => Some((*node, *switch, *held_ms)),
                _ => None,
            })
            .collect()
    }

    pub fn has_soft_off_timeout(&self, node: NodeId) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, ActuatorEvent::SoftOffTimeout { node: n, .. } if *n == node))
    }

    pub fn has_escalated(&self, node: NodeId) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e, ActuatorEvent::Escalated { node: n, .. } if *n == node))
    }

    pub fn faults(&self) -> Vec<&ActuatorEvent> {
        self.events
            .iter()
            .filter(|e| matches!(e, ActuatorEvent::Fault { .. }))
            .collect()
    }
}
