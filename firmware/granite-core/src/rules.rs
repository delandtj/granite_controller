//! The standalone rule engine: up to 16 rules evaluated against
//! [`Observed`] once a second, whether or not the broker is reachable.
//!
//! Thresholds are in the source's own unit, so nothing has to guess a
//! scale: temperatures in centi-degrees Celsius, `vin` in millivolts,
//! booleans as 0/1, `uptime_s` in seconds.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::actuator::ActionKind;
use crate::observed::Observed;
use crate::{DRY_COUNT, NODE_COUNT, PROBE_SLOTS, Target};

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
    /// Bus voltage in millivolts.
    Vin,
    /// Dry contact, 1-based: 1 = closed.
    DryIn {
        /// Input 1..=4.
        n: u8,
    },
    /// Node power LED, 1-based: 1 = on.
    NodeOn {
        /// Node 1..=8.
        n: u8,
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
            Source::NodeOn { n } => {
                if n == 0 || n as usize > NODE_COUNT {
                    return None;
                }
                o.node_led(n).map(i32::from)
            }
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
            Source::Probe { .. } | Source::ProbeMax | Source::BoardTemp => "centi-degrees C",
            Source::Vin => "mV",
            Source::DryIn { .. } | Source::NodeOn { .. } => "0/1",
            Source::MqttConnected | Source::LinkUp => "0/1",
            Source::UptimeS => "s",
        }
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
    /// Run an actuator action on the target.
    Act {
        /// Which action.
        kind: ActionKind,
    },
    /// Publish an event only.
    Event,
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
    /// The value that tripped it.
    pub value: i32,
    /// Whether an ack is needed before it can fire again.
    pub rearm: Rearm,
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
            let Some(v) = value else {
                st.since_ms = None;
                continue;
            };

            let cond = match rule.op {
                Op::Gt => v > rule.threshold,
                Op::Lt => v < rule.threshold,
                Op::Eq => v == rule.threshold,
                Op::Changed => previous.is_some_and(|p| p != v),
            };

            // Re-arm first, so a rule that cleared in the same tick in
            // which it would fire again behaves predictably.
            if !st.armed && rule.rearm == Rearm::Auto && cleared(&rule, v) {
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
                value: v,
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
                kind: ActionKind::ForceOff
            },
            target: Target::All,
            rearm: Rearm::Auto,
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
                kind: ActionKind::ForceOff
            },
            target: Target::All,
            rearm: Rearm::Manual,
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
        },
    ]
}
