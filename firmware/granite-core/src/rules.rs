//! The standalone rule engine: up to 16 rules evaluated against
//! [`Observed`] once a second, whether or not the broker is reachable.
//!
//! Thresholds are in the source's own unit, so nothing has to guess a
//! scale: temperatures in centi-degrees Celsius, voltages in millivolts,
//! currents in milliamperes, power in deciwatts (0.1 W), booleans as
//! 0/1, `uptime_s` in seconds. The node, plane and unit sources are the
//! ones ADR 0004 decision 7 adds; they read the same snapshot the
//! publishers do, so a rule never waits for a plane transaction.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::msg::{Args, Command, CommandKind};
use crate::observed::Observed;
use crate::{DRY_COUNT, MAX_NODES, PROBE_SLOTS, Target};

/// Most rules the engine holds (ADR: 16).
pub const MAX_RULES: usize = 16;

/// What a rule looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum Source {
    /// One probe slot, 1-based, in centi-degrees C.
    Probe {
        /// Slot 1..=8.
        n: u8,
    },
    /// Hottest probe that answered, centi-degrees C.
    ProbeMax,
    /// TMP1075, centi-degrees C.
    BoardTemp,
    /// Controller input voltage in millivolts.
    Vin,
    /// Dry contact, 1-based: 1 = closed.
    DryIn {
        /// Input 1..=4.
        n: u8,
    },
    /// 1 while a node's rail carries it (power axis `on`).
    NodeOn {
        /// Node 1..=64.
        n: u8,
    },
    /// 1 while a node's host LED is lit (host axis `on`).
    NodeHostOn {
        /// Node 1..=64.
        n: u8,
    },
    /// A node's power axis as a code: 0 unknown, 1 off, 2 on,
    /// 3 starting, 4 fault, 5 absent.
    NodePower {
        /// Node 1..=64.
        n: u8,
    },
    /// A node's host axis as a code: 0 unknown, 1 off, 2 on, 3 sleep.
    NodeHost {
        /// Node 1..=64.
        n: u8,
    },
    /// A node's slot voltage in millivolts.
    NodeVolts {
        /// Node 1..=64.
        n: u8,
    },
    /// A node's slot current in milliamperes.
    NodeAmps {
        /// Node 1..=64.
        n: u8,
    },
    /// A node's slot power in deciwatts.
    NodeWatts {
        /// Node 1..=64.
        n: u8,
    },
    /// A node's slot temperature in centi-degrees C.
    NodeTemp {
        /// Node 1..=64.
        n: u8,
    },
    /// A plane's core input (bus) voltage in millivolts.
    PlaneBusVolts {
        /// Plane 0..=7.
        p: u8,
    },
    /// A unit board's input current in milliamperes.
    UnitAmps {
        /// Plane 0..=7.
        p: u8,
        /// Unit 0..=3.
        u: u8,
    },
    /// A unit board's input power in deciwatts.
    UnitWatts {
        /// Plane 0..=7.
        p: u8,
        /// Unit 0..=3.
        u: u8,
    },
    /// 1 while the MQTT session is up.
    MqttConnected,
    /// 1 while Ethernet has link.
    LinkUp,
    /// Seconds since boot.
    UptimeS,
}

impl Source {
    /// Read the source out of a snapshot. `None` means "no value", which
    /// makes every comparison false.
    pub fn read(&self, o: &Observed) -> Option<i32> {
        match *self {
            Source::Probe { n } => {
                if n == 0 || n as usize > PROBE_SLOTS {
                    return None;
                }
                o.probe(n as usize - 1)?.centi_c.map(i32::from)
            }
            Source::ProbeMax => o.probe_max().map(i32::from),
            Source::BoardTemp => o.board_temp.value.map(i32::from),
            Source::Vin => o.vin_mv.value.map(|mv| mv as i32),
            Source::DryIn { n } => {
                if n == 0 || n as usize > DRY_COUNT {
                    return None;
                }
                o.dry(n).map(i32::from)
            }
            Source::NodeOn { n } => node(o, n).map(|x| i32::from(x.is_on())),
            Source::NodeHostOn { n } => {
                node(o, n).map(|x| i32::from(x.host == crate::plane::model::HostState::On))
            }
            Source::NodePower { n } => node(o, n).map(|x| i32::from(power_code(x.power))),
            Source::NodeHost { n } => node(o, n).map(|x| i32::from(host_code(x.host))),
            Source::NodeVolts { n } => node(o, n).map(|x| i32::from(x.telemetry.volts) * 10),
            Source::NodeAmps { n } => node(o, n).map(|x| i32::from(x.telemetry.amps)),
            Source::NodeWatts { n } => node(o, n).map(|x| i32::from(x.telemetry.watts)),
            Source::NodeTemp { n } => node(o, n).map(|x| i32::from(x.telemetry.temperature) * 10),
            Source::PlaneBusVolts { p } => o.plane(p).map(|plane| i32::from(plane.bus_volts) * 10),
            Source::UnitAmps { p, u } => o.unit(p, u).map(|unit| i32::from(unit.amps)),
            Source::UnitWatts { p, u } => o.unit(p, u).map(|unit| i32::from(unit.watts)),
            Source::MqttConnected => {
                if o.mqtt_connected.ts_ms == 0 {
                    None
                } else {
                    Some(i32::from(o.mqtt_connected.value))
                }
            }
            Source::LinkUp => {
                if o.link_up.ts_ms == 0 {
                    None
                } else {
                    Some(i32::from(o.link_up.value))
                }
            }
            Source::UptimeS => Some(o.uptime_s.value as i32),
        }
    }

    /// Unit hint for the UI and the generated documentation.
    pub const fn unit(&self) -> &'static str {
        match self {
            Source::Probe { .. }
            | Source::ProbeMax
            | Source::BoardTemp
            | Source::NodeTemp { .. } => "centi-degrees C",
            Source::Vin | Source::NodeVolts { .. } | Source::PlaneBusVolts { .. } => "mV",
            Source::NodeAmps { .. } | Source::UnitAmps { .. } => "mA",
            Source::NodeWatts { .. } | Source::UnitWatts { .. } => "0.1 W",
            Source::NodePower { .. } => "0 unknown, 1 off, 2 on, 3 starting, 4 fault, 5 absent",
            Source::NodeHost { .. } => "0 unknown, 1 off, 2 on, 3 sleep",
            Source::DryIn { .. }
            | Source::NodeOn { .. }
            | Source::NodeHostOn { .. }
            | Source::MqttConnected
            | Source::LinkUp => "0/1",
            Source::UptimeS => "s",
        }
    }

    /// True when the ids the source names are in range.
    pub const fn is_valid(&self) -> bool {
        match *self {
            Source::Probe { n } => n != 0 && n as usize <= PROBE_SLOTS,
            Source::DryIn { n } => n != 0 && n as usize <= DRY_COUNT,
            Source::NodeOn { n }
            | Source::NodeHostOn { n }
            | Source::NodePower { n }
            | Source::NodeHost { n }
            | Source::NodeVolts { n }
            | Source::NodeAmps { n }
            | Source::NodeWatts { n }
            | Source::NodeTemp { n } => n != 0 && n as usize <= MAX_NODES,
            Source::PlaneBusVolts { p } => crate::is_plane(p),
            Source::UnitAmps { p, u } | Source::UnitWatts { p, u } => {
                crate::is_plane(p) && crate::is_unit(u)
            }
            _ => true,
        }
    }
}

/// One node of a snapshot, if it is a node id and the node is there.
fn node(o: &Observed, n: u8) -> Option<&crate::observed::NodeObs> {
    if n == 0 || n as usize > MAX_NODES {
        return None;
    }
    o.node(n)
}

/// The power axis as the Modbus map and the rules encode it.
pub const fn power_code(power: crate::plane::model::PowerState) -> u8 {
    use crate::plane::model::PowerState as P;
    match power {
        P::Unknown => 0,
        P::Off => 1,
        P::On => 2,
        P::Starting => 3,
        P::Fault => 4,
        P::Absent => 5,
    }
}

/// The host axis as the Modbus map and the rules encode it.
pub const fn host_code(host: crate::plane::model::HostState) -> u8 {
    use crate::plane::model::HostState as H;
    match host {
        H::Unknown => 0,
        H::Off => 1,
        H::On => 2,
        H::Sleep => 3,
    }
}

/// The comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// Greater than the threshold.
    #[serde(rename = ">")]
    Gt,
    /// Less than the threshold.
    #[serde(rename = "<")]
    Lt,
    /// Equal to the threshold (how booleans are compared).
    #[serde(rename = "==")]
    Eq,
    /// The value differs from the previous evaluation. `hold_s` and
    /// `hysteresis` do not apply; it is true for one tick.
    #[serde(rename = "changed")]
    Changed,
}

impl Op {
    /// Wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Op::Gt => ">",
            Op::Lt => "<",
            Op::Eq => "==",
            Op::Changed => "changed",
        }
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a rule does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RuleAction {
    /// Run a verb on the target. Any of the node, unit or plane verbs
    /// of ADR 0004 decision 4 (decision 7: "new actions are the new
    /// verbs").
    Act {
        /// Which verb.
        kind: CommandKind,
    },
    /// Publish an event only.
    Event,
}

/// What a rule does when its source has no value (a probe that stopped
/// answering, a sensor not read yet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// The condition is false; the rule waits for a value.
    #[default]
    Ignore,
    /// The condition counts as true, `hold_s` included: for a safety rule
    /// whose sensor going quiet is as bad as the reading it guards.
    /// Not for `changed`, which needs two values.
    Trip,
}

/// When a fired rule may fire again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rearm {
    /// Re-arms by itself once the condition has cleared past the
    /// hysteresis.
    #[default]
    Auto,
    /// Stays fired until [`RuleEngine::ack`].
    Manual,
}

/// One rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    /// Stable id, 1..=255, unique within the set.
    pub id: u8,
    /// Disabled rules are never evaluated.
    pub enabled: bool,
    /// Optional human label.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// What to look at.
    #[serde(flatten)]
    pub source: Source,
    /// How to compare.
    pub op: Op,
    /// Right-hand side, in the source's unit.
    pub threshold: i32,
    /// How far the value must come back before an `auto` rule re-arms.
    pub hysteresis: i32,
    /// The condition must hold this long before the rule fires.
    pub hold_s: u32,
    /// What to do.
    #[serde(flatten)]
    pub action: RuleAction,
    /// Which node(s) the action applies to.
    pub target: Target,
    /// Re-arm policy.
    pub rearm: Rearm,
    /// What a missing value means.
    pub missing: Missing,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            id: 0,
            enabled: false,
            name: String::new(),
            source: Source::ProbeMax,
            op: Op::Gt,
            threshold: 0,
            hysteresis: 0,
            hold_s: 0,
            action: RuleAction::Event,
            target: Target::All,
            rearm: Rearm::Auto,
            missing: Missing::Ignore,
        }
    }
}

/// A rule that just fired.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fired {
    /// Which rule.
    pub rule_id: u8,
    /// Its name, for the event payload.
    pub name: String,
    /// What to do.
    pub action: RuleAction,
    /// On what.
    pub target: Target,
    /// The value that tripped it; `None` for a missing value that trips.
    pub value: Option<i32>,
    /// Whether an ack is needed before it can fire again.
    pub rearm: Rearm,
}

impl Fired {
    /// The command this firing submits, if it acts.
    ///
    /// `force_off` is the safety action (leak, over-temperature), so it
    /// carries `force`: a node the controller is unsure about is still
    /// commanded off, because a stale snapshot is no reason to leave a
    /// node running in a leak. Every other verb follows the usual rules.
    pub fn command(&self) -> Option<Command> {
        let RuleAction::Act { kind } = self.action else {
            return None;
        };
        let safety = kind == CommandKind::ForceOff;
        Some(
            Command::new(alloc::format!("rule-{}", self.rule_id), kind)
                .with_target(self.target)
                .with_args(Args {
                    force: safety,
                    ..Args::default()
                }),
        )
    }
}

#[derive(Debug, Clone, Default)]
struct RuleState {
    since_ms: Option<u64>,
    armed: bool,
    last_value: Option<i32>,
}

/// The engine.
#[derive(Debug, Clone, Default)]
pub struct RuleEngine {
    rules: Vec<Rule>,
    state: Vec<RuleState>,
}

impl RuleEngine {
    /// Empty engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Engine over a rule set; anything past [`MAX_RULES`] is dropped.
    pub fn with_rules(rules: Vec<Rule>) -> Self {
        let mut e = Self::default();
        e.set_rules(rules);
        e
    }

    /// Replace the rule set, keeping the latch state of rules whose id
    /// and definition did not change.
    pub fn set_rules(&mut self, rules: Vec<Rule>) {
        let old = core::mem::take(&mut self.rules);
        let old_state = core::mem::take(&mut self.state);
        for (i, rule) in rules.into_iter().enumerate() {
            if i >= MAX_RULES {
                break;
            }
            let keep = old
                .iter()
                .position(|r| r.id == rule.id && *r == rule)
                .and_then(|p| old_state.get(p).cloned());
            self.rules.push(rule);
            self.state.push(keep.unwrap_or(RuleState {
                since_ms: None,
                armed: true,
                last_value: None,
            }));
        }
    }

    /// The rules.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// True while rule `id` is armed (has not fired, or has been acked).
    pub fn is_armed(&self, id: u8) -> Option<bool> {
        self.rules
            .iter()
            .position(|r| r.id == id)
            .map(|i| self.state[i].armed)
    }

    /// Acknowledge a `manual` rule so it can fire again. Returns false if
    /// there is no such rule.
    pub fn ack(&mut self, id: u8) -> bool {
        match self.rules.iter().position(|r| r.id == id) {
            Some(i) => {
                self.state[i].armed = true;
                self.state[i].since_ms = None;
                true
            }
            None => false,
        }
    }

    /// Evaluate every enabled rule.
    pub fn tick(&mut self, now_ms: u64, o: &Observed) -> Vec<Fired> {
        let mut fired = Vec::new();
        for i in 0..self.rules.len() {
            let rule = self.rules[i].clone();
            let value = rule.source.read(o);
            let st = &mut self.state[i];
            let previous = st.last_value;
            st.last_value = value;

            if !rule.enabled {
                st.since_ms = None;
                continue;
            }
            let cond = match (value, rule.missing) {
                (Some(v), _) => match rule.op {
                    Op::Gt => v > rule.threshold,
                    Op::Lt => v < rule.threshold,
                    Op::Eq => v == rule.threshold,
                    Op::Changed => previous.is_some_and(|p| p != v),
                },
                (None, Missing::Trip) if rule.op != Op::Changed => true,
                (None, _) => {
                    st.since_ms = None;
                    continue;
                }
            };

            // Re-arm first, so a rule that cleared in the same tick in
            // which it would fire again behaves predictably. A missing
            // value never clears anything.
            if !st.armed && rule.rearm == Rearm::Auto && value.is_some_and(|v| cleared(&rule, v)) {
                st.armed = true;
            }

            if !cond {
                st.since_ms = None;
                continue;
            }

            let since = *st.since_ms.get_or_insert(now_ms);
            let held_ms = now_ms.saturating_sub(since);
            let need_ms = rule.hold_s as u64 * 1_000;
            if rule.op != Op::Changed && held_ms < need_ms {
                continue;
            }
            if !st.armed {
                continue;
            }
            st.armed = false;
            st.since_ms = if rule.op == Op::Changed {
                None
            } else {
                Some(since)
            };
            fired.push(Fired {
                rule_id: rule.id,
                name: rule.name.clone(),
                action: rule.action,
                target: rule.target,
                value,
                rearm: rule.rearm,
            });
        }
        fired
    }
}

/// Has the value come back far enough for an `auto` rule to re-arm?
fn cleared(rule: &Rule, v: i32) -> bool {
    let h = rule.hysteresis.abs();
    match rule.op {
        Op::Gt => v < rule.threshold.saturating_sub(h),
        Op::Lt => v > rule.threshold.saturating_add(h),
        Op::Eq => v != rule.threshold,
        Op::Changed => true,
    }
}

/// The three examples the ADR ships as defaults, all disabled.
pub fn default_rules() -> Vec<Rule> {
    alloc::vec![
        Rule {
            id: 1,
            enabled: false,
            name: String::from("probe over temperature"),
            source: Source::ProbeMax,
            op: Op::Gt,
            threshold: 7_000,
            hysteresis: 500,
            hold_s: 30,
            action: RuleAction::Act {
                kind: CommandKind::ForceOff
            },
            target: Target::All,
            rearm: Rearm::Auto,
            // No probe answering is no temperature watch at all.
            missing: Missing::Trip,
        },
        Rule {
            id: 2,
            enabled: false,
            name: String::from("leak float closed"),
            source: Source::DryIn { n: 1 },
            op: Op::Eq,
            threshold: 1,
            hysteresis: 0,
            hold_s: 2,
            action: RuleAction::Act {
                kind: CommandKind::ForceOff
            },
            target: Target::All,
            rearm: Rearm::Manual,
            missing: Missing::Ignore,
        },
        Rule {
            id: 3,
            enabled: false,
            name: String::from("bus voltage low"),
            source: Source::Vin,
            op: Op::Lt,
            threshold: 15_000,
            hysteresis: 500,
            hold_s: 5,
            action: RuleAction::Event,
            target: Target::All,
            rearm: Rearm::Auto,
            missing: Missing::Ignore,
        },
    ]
}
