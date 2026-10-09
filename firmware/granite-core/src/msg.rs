//! The command vocabulary and the published payloads.
//!
//! One `Command` enum shared by MQTT, the HTTP API and (reduced) Modbus,
//! exactly as the ADR's MQTT table spells it out:
//!
//! ```json
//! {"v":1,"id":"c-7","action":"on","target":3,"args":{}}
//! ```
//!
//! Every payload carries `v`, so a client can tell which vocabulary it is
//! talking to; the HTTP API carries the same number in its `/api/v1`
//! prefix.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::actuator::{ActionArgs, ActionKind, ActionRequest, ActionResult, ActuatorEvent};
use crate::config::{NodesCfg, Section};
use crate::hal::{BootReason, Switch, rom_id_hex};
use crate::node::LastAction;
use crate::observed::Observed;
use crate::rules::{Fired, RuleAction};
use crate::{DRY_COUNT, NodeId, Target};

/// Version stamped into every payload.
pub const PROTOCOL_VERSION: u8 = 1;

/// The last-will payload. The ADR fixes the shape.
pub const LWT_PAYLOAD: &str = "{\"v\":1,\"online\":false}";

const fn default_v() -> u8 {
    PROTOCOL_VERSION
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Everything a client can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Power a node on.
    On,
    /// Soft-off a node, escalating to `force_off`.
    Off,
    /// Hold PWR until the node is down.
    ForceOff,
    /// Short RST press.
    Reset,
    /// Off, wait, on.
    Cycle,
    /// Raw press.
    Press,
    /// Staggered on over every node.
    OnAll,
    /// Acknowledge a `manual` rule (`args.rule`).
    RuleAck,
    /// Re-enumerate the 1-wire bus.
    ProbeScan,
    /// Read the configuration (`args.section` for one section).
    ConfigGet,
    /// Write the configuration (`args.section` plus `args.config`).
    ConfigSet,
    /// Pull and install a firmware image (`args.url`, `args.sha256`).
    Ota,
    /// Restart the controller.
    Reboot,
    /// Erase everything but the factory namespace (`args.confirm`).
    FactoryReset,
}

impl CommandKind {
    /// Wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            CommandKind::On => "on",
            CommandKind::Off => "off",
            CommandKind::ForceOff => "force_off",
            CommandKind::Reset => "reset",
            CommandKind::Cycle => "cycle",
            CommandKind::Press => "press",
            CommandKind::OnAll => "on_all",
            CommandKind::RuleAck => "rule_ack",
            CommandKind::ProbeScan => "probe_scan",
            CommandKind::ConfigGet => "config_get",
            CommandKind::ConfigSet => "config_set",
            CommandKind::Ota => "ota",
            CommandKind::Reboot => "reboot",
            CommandKind::FactoryReset => "factory_reset",
        }
    }

    /// The actuator action behind this command, if it is one.
    pub const fn action(self) -> Option<ActionKind> {
        Some(match self {
            CommandKind::On => ActionKind::On,
            CommandKind::Off => ActionKind::Off,
            CommandKind::ForceOff => ActionKind::ForceOff,
            CommandKind::Reset => ActionKind::Reset,
            CommandKind::Cycle => ActionKind::Cycle,
            CommandKind::Press => ActionKind::Press,
            CommandKind::OnAll => ActionKind::OnAll,
            _ => return None,
        })
    }

    /// The command that runs an actuator action.
    pub const fn of_action(a: ActionKind) -> Self {
        match a {
            ActionKind::On => CommandKind::On,
            ActionKind::Off => CommandKind::Off,
            ActionKind::ForceOff => CommandKind::ForceOff,
            ActionKind::Reset => CommandKind::Reset,
            ActionKind::Cycle => CommandKind::Cycle,
            ActionKind::Press => CommandKind::Press,
            ActionKind::OnAll => CommandKind::OnAll,
        }
    }

    /// True when the command may change node power.
    pub const fn is_actuator(self) -> bool {
        self.action().is_some()
    }
}

impl core::fmt::Display for CommandKind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `args` object. Every field is optional; unknown fields are
/// ignored so an older firmware does not choke on a newer client.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Args {
    /// Act on a node whose state is `Unknown`.
    #[serde(skip_serializing_if = "is_false")]
    pub force: bool,
    /// `off` returns `shutdown_pending` instead of escalating.
    #[serde(skip_serializing_if = "is_false")]
    pub no_escalate: bool,
    /// `cycle` uses `force_off` for the off half.
    #[serde(skip_serializing_if = "is_false")]
    pub hard: bool,
    /// Which switch a raw `press` uses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub switch: Option<Switch>,
    /// Raw `press` duration, 100 ms to 10 s.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// Rule id for `rule_ack`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<u8>,
    /// Section for `config_get` / `config_set`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub section: Option<Section>,
    /// Document for `config_set`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
    /// Image URL for `ota`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Expected image digest for `ota`, hex.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Device id, required by `factory_reset`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<String>,
}

impl Args {
    /// True when nothing is set.
    pub fn is_empty(&self) -> bool {
        *self == Args::default()
    }

    /// The subset the actuator cares about.
    pub fn to_action_args(&self) -> ActionArgs {
        ActionArgs {
            force: self.force,
            no_escalate: self.no_escalate,
            hard: self.hard,
            switch: self.switch,
            duration_ms: self.duration_ms,
        }
    }
}

/// A command as it arrives on `.../cmd`, over the HTTP API, or built from
/// a Modbus register write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// Payload version.
    #[serde(default = "default_v")]
    pub v: u8,
    /// Client-chosen id, echoed in the ack.
    pub id: String,
    /// What to do.
    pub action: CommandKind,
    /// On what. Defaults to `all`.
    #[serde(default)]
    pub target: Target,
    /// Arguments.
    #[serde(default, skip_serializing_if = "Args::is_empty")]
    pub args: Args,
    /// Reserved: a per-command signature. The dispatcher has one hook for
    /// it and ignores it today (ADR, "Why trust the broker").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<String>,
}

impl Command {
    /// A command with default args and target `all`.
    pub fn new(id: impl Into<String>, action: CommandKind) -> Self {
        Command {
            v: PROTOCOL_VERSION,
            id: id.into(),
            action,
            target: Target::All,
            args: Args::default(),
            sig: None,
        }
    }

    /// Builder: set the target.
    pub fn with_target(mut self, target: Target) -> Self {
        self.target = target;
        self
    }

    /// Builder: set the args.
    pub fn with_args(mut self, args: Args) -> Self {
        self.args = args;
        self
    }

    /// Parse from JSON.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| e.to_string())
    }

    /// Serialise to JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }

    /// The actuator request behind this command, if it is an action.
    pub fn to_action_request(&self) -> Option<ActionRequest> {
        let kind = self.action.action()?;
        Some(ActionRequest {
            id: self.id.clone(),
            kind,
            target: if kind == ActionKind::OnAll {
                Target::All
            } else {
                self.target
            },
            args: self.args.to_action_args(),
        })
    }
}

/// The ack published on `.../ack/<id>` and returned by the HTTP API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    /// Payload version.
    #[serde(default = "default_v")]
    pub v: u8,
    /// Id of the command this answers.
    pub id: String,
    /// Whether the command was accepted or completed successfully.
    pub ok: bool,
    /// Short outcome string ("accepted", "ok", "shutdown_pending", ...).
    pub result: Option<String>,
    /// Error text when `ok` is false.
    pub error: Option<String>,
    /// Payload for the commands that return data (`config_get`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Reply {
    /// A successful reply.
    pub fn ok(id: impl Into<String>, result: impl Into<String>) -> Self {
        Reply {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: true,
            result: Some(result.into()),
            error: None,
            data: None,
        }
    }

    /// The reply for a queued action: the final outcome arrives later as
    /// an `action_done` event and the ack on completion.
    pub fn accepted(id: impl Into<String>) -> Self {
        Reply::ok(id, "accepted")
    }

    /// A failed reply.
    pub fn err(id: impl Into<String>, error: impl Into<String>) -> Self {
        Reply {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: false,
            result: None,
            error: Some(error.into()),
            data: None,
        }
    }

    /// Builder: attach data.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// The reply for a finished action.
    pub fn of_result(id: impl Into<String>, result: &ActionResult) -> Self {
        let text = result.as_str();
        if result.is_ok() {
            Reply::ok(id, text)
        } else {
            Reply::err(id, text)
        }
    }

    /// Serialise to JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// What happened, as published on `.../event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    /// A node's reported state changed.
    NodeState {
        /// Node.
        node: NodeId,
        /// New state name.
        state: String,
        /// Debounced LED, if known.
        led: Option<bool>,
    },
    /// A command was queued. Emitted for long actions on receipt.
    Accepted {
        /// Command id.
        id: String,
        /// Action.
        action: ActionKind,
        /// Target.
        target: Target,
    },
    /// An action finished successfully.
    ActionDone {
        /// Command id.
        id: String,
        /// Node, absent for a group.
        node: Option<NodeId>,
        /// Action.
        action: ActionKind,
        /// Outcome string.
        result: String,
    },
    /// An action did not reach its goal.
    ActionFailed {
        /// Command id.
        id: String,
        /// Node, absent for a group.
        node: Option<NodeId>,
        /// Action.
        action: ActionKind,
        /// Why.
        error: String,
    },
    /// Soft-off ran out of patience. Emitted before any escalation.
    SoftOffTimeout {
        /// Command id.
        id: String,
        /// Node.
        node: NodeId,
    },
    /// A rule fired.
    RuleFired {
        /// Rule id.
        rule: u8,
        /// Rule name.
        name: String,
        /// What it asked for.
        action: String,
        /// On what.
        target: Target,
        /// The value that tripped it.
        value: i32,
        /// Whether it needs an ack before it can fire again.
        needs_ack: bool,
    },
    /// Hardware trouble.
    Fault {
        /// Fault class.
        fault: String,
        /// Node, when a job was involved.
        node: Option<NodeId>,
        /// Command id, when a job was involved.
        id: Option<String>,
    },
    /// Firmware update progress.
    Ota {
        /// `started`, `downloading`, `verified`, `pending_verify`,
        /// `valid`, `rolled_back`, `failed`.
        phase: String,
        /// Percent, when known.
        progress: Option<u8>,
        /// Free text.
        detail: Option<String>,
    },
    /// Something the HTTP API wants on the record: a login, a failed
    /// login, a lockout, a recovery attempt, a password or key change
    /// (ADR components 8 and 13).
    Security {
        /// `login`, `login_failed`, `login_lockout`, `password_set`,
        /// `password_changed`, `token_created`, `token_deleted`,
        /// `fleet_key_set`, `device_cert_set`, `recover_accepted`,
        /// `recover_failed`.
        what: String,
        /// Free text.
        detail: Option<String>,
        /// Peer address, when the transport knows it.
        peer: Option<String>,
    },
    /// Configuration changed, or a section fell back to defaults.
    Config {
        /// Which section, when it is about one.
        section: Option<Section>,
        /// `set`, `fallback`, `imported`, `staged`, `confirmed`,
        /// `reverted`, `factory_reset`.
        change: String,
        /// Free text.
        detail: Option<String>,
    },
}

/// An event with its timestamp.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Payload version.
    #[serde(default = "default_v")]
    pub v: u8,
    /// Monotonic milliseconds since boot.
    pub ts: u64,
    /// What happened.
    #[serde(flatten)]
    pub kind: EventKind,
}

impl Event {
    /// Wrap a kind.
    pub fn new(ts: u64, kind: EventKind) -> Self {
        Event {
            v: PROTOCOL_VERSION,
            ts,
            kind,
        }
    }

    /// Serialise to JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// Translate an actuator event into a published event. Returns `None` for
/// the internal press bookkeeping, which only goes to the log.
pub fn event_from_actuator(ev: &ActuatorEvent, ts: u64) -> Option<Event> {
    let kind = match ev {
        ActuatorEvent::Accepted { id, node, action } => EventKind::Accepted {
            id: id.clone(),
            action: *action,
            target: Target::Node(*node),
        },
        ActuatorEvent::SoftOffTimeout { id, node } => EventKind::SoftOffTimeout {
            id: id.clone(),
            node: *node,
        },
        ActuatorEvent::Escalated { id, node } => EventKind::ActionDone {
            id: id.clone(),
            node: Some(*node),
            action: ActionKind::Off,
            result: String::from("escalated"),
        },
        ActuatorEvent::Done {
            id,
            node,
            action,
            result,
        } => {
            if result.is_ok() {
                EventKind::ActionDone {
                    id: id.clone(),
                    node: Some(*node),
                    action: *action,
                    result: result.as_str(),
                }
            } else {
                EventKind::ActionFailed {
                    id: id.clone(),
                    node: Some(*node),
                    action: *action,
                    error: result.as_str(),
                }
            }
        }
        ActuatorEvent::GroupDone { id, action, result } => {
            if result.is_ok() {
                EventKind::ActionDone {
                    id: id.clone(),
                    node: None,
                    action: *action,
                    result: result.as_str(),
                }
            } else {
                EventKind::ActionFailed {
                    id: id.clone(),
                    node: None,
                    action: *action,
                    error: result.as_str(),
                }
            }
        }
        ActuatorEvent::Fault { id, node, fault } => EventKind::Fault {
            fault: fault.to_string(),
            node: *node,
            id: id.clone(),
        },
        ActuatorEvent::PressStart { .. } | ActuatorEvent::PressEnd { .. } => return None,
    };
    Some(Event::new(ts, kind))
}

/// Translate a fired rule into a published event.
pub fn event_from_rule(f: &Fired, ts: u64) -> Event {
    let action = match f.action {
        RuleAction::Act { kind } => String::from(kind.as_str()),
        RuleAction::Event => String::from("event"),
    };
    Event::new(
        ts,
        EventKind::RuleFired {
            rule: f.rule_id,
            name: f.name.clone(),
            action,
            target: f.target,
            value: f.value,
            needs_ack: f.rearm == crate::rules::Rearm::Manual,
        },
    )
}

/// The retained `.../status` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Payload version.
    #[serde(default = "default_v")]
    pub v: u8,
    /// True in the live payload, false in the last will.
    pub online: bool,
    /// Firmware version string.
    pub fw: String,
    /// Current IPv4 address.
    pub ip: String,
    /// Seconds since boot.
    pub uptime_s: u32,
    /// Why this boot happened.
    pub boot_reason: String,
}

impl Status {
    /// The live payload.
    pub fn online(fw: &str, ip: &str, uptime_s: u32, boot_reason: BootReason) -> Self {
        Status {
            v: PROTOCOL_VERSION,
            online: true,
            fw: String::from(fw),
            ip: String::from(ip),
            uptime_s,
            boot_reason: String::from(boot_reason.as_str()),
        }
    }

    /// Serialise to JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// One node inside the `.../state` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeStatePayload {
    /// Node id.
    pub node: NodeId,
    /// User name.
    pub name: String,
    /// `unknown`, `off`, `on` or `busy`.
    pub state: String,
    /// Debounced LED, if known.
    pub led: Option<bool>,
    /// What is running, while `busy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_action: Option<ActionKind>,
    /// Last action on this node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_action: Option<LastAction>,
    /// When the controller last reset it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reset_ms: Option<u64>,
}

/// One probe inside the `.../state` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbePayload {
    /// Modbus slot, 1-based.
    pub slot: u8,
    /// ROM id, 16 hex digits.
    pub rom: String,
    /// User name.
    pub name: String,
    /// Degrees Celsius, `null` when the probe did not answer.
    pub temp_c: Option<f32>,
}

/// The retained `.../state` payload: the full snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// Payload version.
    #[serde(default = "default_v")]
    pub v: u8,
    /// Monotonic milliseconds of the snapshot.
    pub ts: u64,
    /// Nodes, 1..=8.
    pub nodes: Vec<NodeStatePayload>,
    /// Probes in slot order.
    pub probes: Vec<ProbePayload>,
    /// Board temperature in degrees Celsius.
    pub board_temp_c: Option<f32>,
    /// Bus voltage in volts.
    pub vin_v: Option<f32>,
    /// Dry contacts, index 0 is input 1; `null` when never read.
    pub dry_in: Option<Vec<bool>>,
    /// Ethernet link.
    pub link_up: bool,
    /// MQTT session.
    pub mqtt_connected: bool,
    /// Seconds since boot.
    pub uptime_s: u32,
}

impl State {
    /// Build the payload from the snapshot plus the names in config.
    pub fn from_observed(o: &Observed, nodes_cfg: &NodesCfg, ts: u64) -> Self {
        let nodes = (1..=crate::NODE_COUNT as NodeId)
            .map(|n| {
                let obs = &o.nodes[n as usize - 1];
                let busy_action = match &obs.state {
                    crate::node::NodeState::Busy { action, .. } => Some(*action),
                    _ => None,
                };
                NodeStatePayload {
                    node: n,
                    name: nodes_cfg.nodes[n as usize - 1].name.clone(),
                    state: String::from(obs.state.as_str()),
                    led: obs.led,
                    busy_action,
                    last_action: obs.last_action.clone(),
                    last_reset_ms: obs.last_reset_ms,
                }
            })
            .collect();
        let probes = o
            .probes
            .iter()
            .enumerate()
            .map(|(i, p)| ProbePayload {
                slot: (i + 1) as u8,
                rom: rom_id_hex(p.rom),
                name: p.name.clone(),
                temp_c: p.centi_c.map(|c| c as f32 / 100.0),
            })
            .collect();
        State {
            v: PROTOCOL_VERSION,
            ts,
            nodes,
            probes,
            board_temp_c: o.board_temp.value.map(|c| c as f32 / 100.0),
            vin_v: o.vin_mv.value.map(|mv| mv as f32 / 1000.0),
            dry_in: o.dry_in.value.map(|bits| {
                (0..DRY_COUNT)
                    .map(|i| bits & (1 << i) != 0)
                    .collect::<Vec<bool>>()
            }),
            link_up: o.link_up.value,
            mqtt_connected: o.mqtt_connected.value,
            uptime_s: o.uptime_s.value,
        }
    }

    /// Serialise to JSON.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
}

/// The topic tree of one device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topics {
    base: String,
}

impl Topics {
    /// `<root>/<site>/<device>`.
    pub fn new(base: impl Into<String>) -> Self {
        Topics { base: base.into() }
    }

    /// The base.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Retained status, with the last will on the same topic.
    pub fn status(&self) -> String {
        format!("{}/status", self.base)
    }

    /// Retained full state.
    pub fn state(&self) -> String {
        format!("{}/state", self.base)
    }

    /// Events.
    pub fn event(&self) -> String {
        format!("{}/event", self.base)
    }

    /// Log lines.
    pub fn log(&self) -> String {
        format!("{}/log", self.base)
    }

    /// Inbound commands.
    pub fn cmd(&self) -> String {
        format!("{}/cmd", self.base)
    }

    /// Ack for one command id.
    pub fn ack(&self, id: &str) -> String {
        format!("{}/ack/{}", self.base, id)
    }
}
