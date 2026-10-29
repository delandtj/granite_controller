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

use crate::config::{NodesCfg, Section};
use crate::hal::{BootReason, Switch, rom_id_hex};
use crate::node::LastJob;
use crate::observed::{Observed, PlaneObs};
use crate::plane::event::{ClientEvent, kind_name};
use crate::plane::{JobOutcome, NodeOptions};
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
///
/// The rev C verbs keep their names and their meaning (ADR 0004
/// decision 4), so an MQTT or API client written for the relay
/// controller keeps working; `power_on`, `power_off`, `clear_latch`,
/// `unit_reset` and `plane_reboot` come with the plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// Rail on if needed, then PWR if the host stays dark.
    On,
    /// Soft-off, escalating per the node's setting.
    Off,
    /// Hold PWR, then cut the rail if the host is still lit.
    ForceOff,
    /// Short RST press.
    Reset,
    /// Rail off, wait, rail on, PWR if the host stays dark.
    Cycle,
    /// Raw press of one button.
    Press,
    /// Staggered `on` over a set of nodes.
    OnAll,
    /// The rail only, no buttons.
    PowerOn,
    /// The rail only, no buttons.
    PowerOff,
    /// Clear a slot's overvoltage latch.
    ClearLatch,
    /// Re-arm a unit board's own hot-swap latch.
    UnitReset,
    /// Reboot a plane.
    PlaneReboot,
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
    /// Every verb that goes to a plane, in table order (ADR 0004
    /// decision 4).
    pub const PLANE_VERBS: [CommandKind; 12] = [
        CommandKind::On,
        CommandKind::Off,
        CommandKind::ForceOff,
        CommandKind::Reset,
        CommandKind::Cycle,
        CommandKind::Press,
        CommandKind::OnAll,
        CommandKind::PowerOn,
        CommandKind::PowerOff,
        CommandKind::ClearLatch,
        CommandKind::UnitReset,
        CommandKind::PlaneReboot,
    ];

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
            CommandKind::PowerOn => "power_on",
            CommandKind::PowerOff => "power_off",
            CommandKind::ClearLatch => "clear_latch",
            CommandKind::UnitReset => "unit_reset",
            CommandKind::PlaneReboot => "plane_reboot",
            CommandKind::RuleAck => "rule_ack",
            CommandKind::ProbeScan => "probe_scan",
            CommandKind::ConfigGet => "config_get",
            CommandKind::ConfigSet => "config_set",
            CommandKind::Ota => "ota",
            CommandKind::Reboot => "reboot",
            CommandKind::FactoryReset => "factory_reset",
        }
    }

    /// Parse a verb name.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        let name = s.trim();
        [
            CommandKind::On,
            CommandKind::Off,
            CommandKind::ForceOff,
            CommandKind::Reset,
            CommandKind::Cycle,
            CommandKind::Press,
            CommandKind::OnAll,
            CommandKind::PowerOn,
            CommandKind::PowerOff,
            CommandKind::ClearLatch,
            CommandKind::UnitReset,
            CommandKind::PlaneReboot,
            CommandKind::RuleAck,
            CommandKind::ProbeScan,
            CommandKind::ConfigGet,
            CommandKind::ConfigSet,
            CommandKind::Ota,
            CommandKind::Reboot,
            CommandKind::FactoryReset,
        ]
        .into_iter()
        .find(|k| k.as_str() == name)
    }

    /// True when the verb becomes a plane job: it names a node, a unit
    /// or a plane and finishes asynchronously (ADR 0004 decision 6).
    pub const fn is_plane_verb(self) -> bool {
        matches!(
            self,
            CommandKind::On
                | CommandKind::Off
                | CommandKind::ForceOff
                | CommandKind::Reset
                | CommandKind::Cycle
                | CommandKind::Press
                | CommandKind::OnAll
                | CommandKind::PowerOn
                | CommandKind::PowerOff
                | CommandKind::ClearLatch
                | CommandKind::UnitReset
                | CommandKind::PlaneReboot
        )
    }

    /// True when the verb acts on a node rather than on a board.
    pub const fn is_node_verb(self) -> bool {
        self.is_plane_verb() && !matches!(self, CommandKind::UnitReset | CommandKind::PlaneReboot)
    }

    /// The target this verb needs: a node, a unit or a plane.
    pub const fn wants(self) -> TargetKind {
        match self {
            CommandKind::UnitReset => TargetKind::Unit,
            CommandKind::PlaneReboot => TargetKind::Plane,
            _ => TargetKind::Node,
        }
    }
}

/// What kind of thing a verb acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    /// One node, or every present node.
    Node,
    /// One unit board.
    Unit,
    /// One plane.
    Plane,
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
    /// Submit the command even when the controller believes the node is
    /// not there. Accepted and carried for rev C clients; the plane has
    /// the last word on presence and refuses what is not there.
    #[serde(skip_serializing_if = "is_false")]
    pub force: bool,
    /// `off` and `force_off` do not escalate: the short press, or the
    /// hold, and nothing after it (escalation `none`).
    #[serde(skip_serializing_if = "is_false")]
    pub no_escalate: bool,
    /// `cycle` does not wait for the host's LED on the way back up.
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

    /// The press duration this command asks for \[ms\], clamped to what
    /// a plane accepts.
    pub fn press_ms(&self) -> u16 {
        let ms = self
            .duration_ms
            .unwrap_or(u32::from(crate::plane::PRESS_MS));
        ms.clamp(
            u32::from(granite_proto::PRESS_MS_MIN),
            u32::from(granite_proto::PRESS_MS_MAX),
        ) as u16
    }

    /// The switch a raw `press` uses; PWR unless the client said RST.
    pub fn press_switch(&self) -> Switch {
        self.switch.unwrap_or(Switch::Pwr)
    }

    /// The node options these args ask for, on top of the node's own
    /// settings.
    pub fn apply_to(&self, mut opts: NodeOptions) -> NodeOptions {
        if self.no_escalate {
            opts.escalate = granite_proto::Escalate::None;
        }
        if self.hard {
            opts.led_timeout_s = 0;
        }
        opts
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
    /// Short outcome string ("accepted", "ok", "done", "absent", ...).
    pub result: Option<String>,
    /// Error text when `ok` is false.
    pub error: Option<String>,
    /// The plane job this command became, when it became one (ADR 0004
    /// decision 6): the ack that follows the plane's result carries the
    /// same number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<crate::plane::JobId>,
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
            job: None,
            data: None,
        }
    }

    /// The reply for a queued command: the final outcome arrives later
    /// as an `action_done` event and the ack on completion (ADR 0004
    /// decision 6).
    pub fn accepted(id: impl Into<String>) -> Self {
        Reply::ok(id, "accepted")
    }

    /// Builder: note the plane job this command became.
    pub fn with_job(mut self, job: crate::plane::JobId) -> Self {
        self.job = Some(job);
        self
    }

    /// A failed reply.
    pub fn err(id: impl Into<String>, error: impl Into<String>) -> Self {
        Reply {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ok: false,
            result: None,
            error: Some(error.into()),
            job: None,
            data: None,
        }
    }

    /// Builder: attach data.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }

    /// The reply for a job the plane has finished.
    pub fn of_outcome(id: impl Into<String>, outcome: JobOutcome, detail: Option<&str>) -> Self {
        if outcome.is_ok() {
            Reply::ok(id, outcome.as_str())
        } else {
            match detail {
                Some(d) => Reply::err(id, alloc::format!("{}: {d}", outcome.as_str())),
                None => Reply::err(id, outcome.as_str()),
            }
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
    /// A node's state changed on one of its two axes (ADR 0004
    /// decision 3).
    NodeState {
        /// Node.
        node: NodeId,
        /// The rail: `off`, `on`, `starting`, `fault`, `absent`,
        /// `unknown`.
        power: String,
        /// The host: `off`, `on`, `sleep`, `unknown`.
        host: String,
    },
    /// A command was accepted and became a plane job.
    Accepted {
        /// Command id.
        id: String,
        /// Verb.
        action: CommandKind,
        /// Target.
        target: Target,
        /// The plane job it became.
        job: crate::plane::JobId,
    },
    /// A plane job finished as asked.
    ActionDone {
        /// Command id.
        id: String,
        /// Node, absent for a unit, a plane or a group.
        node: Option<NodeId>,
        /// Verb.
        action: CommandKind,
        /// Outcome string ("done").
        result: String,
    },
    /// A plane job did not reach its goal.
    ActionFailed {
        /// Command id.
        id: String,
        /// Node, absent for a unit, a plane or a group.
        node: Option<NodeId>,
        /// Verb.
        action: CommandKind,
        /// Why ("failed: not_present", "absent", "link_lost", ...).
        error: String,
    },
    /// Something the plane client learned: a plane found, lost or
    /// rebooted, a slot or unit event the plane raised, a config set
    /// pushed or refused, a plane firmware update, the OFF commands
    /// re-applied after a plane lost its intended state (ADR 0004
    /// decision 7: "new event kinds for the plane events").
    ///
    /// `what` is the kind: the plane's own event names (`slot_on`,
    /// `ov_latched`, `unit_hot_swap_latched_off`, `sequence_failed`,
    /// ...) and the client's (`plane_found`, `plane_lost`,
    /// `plane_rebooted`, `address_collision`, `events_lost`,
    /// `config_pushed`, `config_refused`, `update_confirmed`,
    /// `update_failed`, `intended_off_reapplied`).
    Plane {
        /// Which kind.
        what: String,
        /// The plane's address, when it is about one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        address: Option<u8>,
        /// The node, when it is about one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node: Option<NodeId>,
        /// The unit, when it is about one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unit: Option<u8>,
        /// Free text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
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
        /// The value that tripped it; null when the rule tripped on a
        /// missing value (`"missing": "trip"`).
        value: Option<i32>,
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
        /// `fleet_key_set`, `device_cert_set`, `mqtt_credentials_set`,
        /// `recover_accepted`, `recover_failed`.
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

/// Translate one of the plane client's events into a published event
/// (ADR 0004 decision 7).
///
/// A finished job is not one of these: the firmware answers it with the
/// ack and the `action_done` / `action_failed` event of the command that
/// asked for it, which [`crate::jobs`] builds.
pub fn event_from_client(ev: &ClientEvent, ts: u64) -> Option<Event> {
    use alloc::string::ToString as _;

    if matches!(ev, ClientEvent::JobFinished { .. }) {
        return None;
    }
    let detail = match ev {
        ClientEvent::PlaneFound {
            firmware,
            boot_count,
            ..
        } => Some(format!(
            "firmware {}, boot {boot_count}",
            crate::observed::version_string(*firmware)
        )),
        ClientEvent::PlaneRebooted {
            boot_count,
            reset_cause,
            intended_state_lost,
            ..
        } => Some(format!(
            "boot {boot_count}, reset cause {reset_cause}{}",
            if *intended_state_lost {
                ", intended state lost"
            } else {
                ""
            }
        )),
        ClientEvent::EventsLost { count, .. } => Some(format!("{count} events lost")),
        ClientEvent::ConfigPushed { hash, .. } => Some(format!("hash {hash:#010x}")),
        ClientEvent::UpdateConfirmed { version, .. } => {
            Some(crate::observed::version_string(*version))
        }
        ClientEvent::UpdateFailed {
            version, reason, ..
        } => Some(format!(
            "{} at {}",
            crate::observed::version_string(*version),
            reason.as_str()
        )),
        ClientEvent::IntendedOffReapplied { nodes, .. } => Some(
            nodes
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<String>>()
                .join(","),
        ),
        ClientEvent::Node { kind, .. } | ClientEvent::Plane { kind, .. } => {
            Some(String::from(kind_name(*kind)))
        }
        ClientEvent::Unit { kind, unit, .. } => Some(format!("unit {unit}: {}", kind_name(*kind))),
        _ => None,
    };
    Some(Event::new(
        ts,
        EventKind::Plane {
            what: String::from(ev.as_str()),
            address: ev.address(),
            node: ev.node(),
            unit: match ev {
                ClientEvent::Unit { unit, .. } => Some(*unit),
                _ => None,
            },
            detail,
        },
    ))
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

/// One node inside the `.../state` payload (ADR 0004 decision 7).
///
/// The telemetry is rounded: 0.1 V, 0.1 A, whole watts, whole degrees,
/// whole watt-hours. That is what keeps a publisher that republishes
/// "on change" from firing on sensor noise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeStatePayload {
    /// Node id 1-64.
    pub node: NodeId,
    /// User name.
    pub name: String,
    /// Plane number 0-7.
    pub plane: u8,
    /// Slot on the plane, 0-7.
    pub slot: u8,
    /// Unit board, 0-3.
    pub unit: u8,
    /// The rail: `off`, `on`, `starting`, `fault`, `absent`, `unknown`.
    pub power: String,
    /// The host: `off`, `on`, `sleep`, `unknown`.
    pub host: String,
    /// True while the plane runs a sequence on this slot.
    #[serde(skip_serializing_if = "is_false")]
    pub busy: bool,
    /// Volts.
    pub v: f32,
    /// Amperes.
    pub a: f32,
    /// Watts.
    pub w: u16,
    /// Lowest power seen since the last min / max clear \[W\].
    pub w_min: u16,
    /// Highest power seen since the last min / max clear \[W\].
    pub w_max: u16,
    /// Slot temperature in degrees Celsius.
    pub temp_c: i16,
    /// Energy since the plane's first boot, in watt-hours.
    pub energy_wh: u64,
    /// The plane's module class code.
    pub module_class: u8,
    /// Which faults are set, by name; absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faults: Vec<String>,
    /// Last job the controller gave this node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_job: Option<LastJob>,
}

/// One unit board inside the `.../state` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitPayload {
    /// Unit index 0-3.
    pub unit: u8,
    /// True when the unit answers on the chain.
    pub present: bool,
    /// True when its own hot-swap latched off.
    #[serde(skip_serializing_if = "is_false")]
    pub latched_off: bool,
    /// Input volts.
    pub v: f32,
    /// Input amperes.
    pub a: f32,
    /// Input watts.
    pub w: u16,
    /// Input temperature in degrees Celsius.
    pub temp_c: i16,
}

/// One plane inside the `.../state` payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanePayload {
    /// Plane number 0-7.
    pub plane: u8,
    /// Its I2C address.
    pub address: u8,
    /// What kind of plane it says it is.
    #[serde(rename = "type")]
    pub plane_type: String,
    /// Firmware version.
    pub firmware: String,
    /// Core input (bus) volts.
    pub bus_v: f32,
    /// Core temperature in degrees Celsius.
    pub temp_c: i16,
    /// Boot count.
    pub boot_count: u32,
    /// Seconds since the plane booted.
    pub uptime_s: u32,
    /// What the plane says about its running image.
    pub image_state: String,
    /// True while a sequence runs on any of its slots.
    #[serde(skip_serializing_if = "is_false")]
    pub sequence_running: bool,
    /// True while it says its intended state is lost.
    #[serde(skip_serializing_if = "is_false")]
    pub intended_state_lost: bool,
    /// The unit boards.
    pub units: Vec<UnitPayload>,
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
    /// Every present node, by id (ADR 0004 decision 2: a node that is
    /// not present is not listed).
    pub nodes: Vec<NodeStatePayload>,
    /// Every plane that answers.
    pub planes: Vec<PlanePayload>,
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
        let nodes: Vec<NodeStatePayload> = o
            .nodes
            .iter()
            .map(|obs| NodeStatePayload {
                node: obs.node,
                name: nodes_cfg.name_of(obs.node),
                plane: obs.plane,
                slot: obs.slot,
                unit: obs.unit,
                power: String::from(obs.power.as_str()),
                host: String::from(obs.host.as_str()),
                busy: obs.busy,
                v: f32::from(obs.telemetry.volts_tenths()) / 10.0,
                a: f32::from(obs.telemetry.amps_tenths()) / 10.0,
                w: obs.telemetry.watts_whole(),
                w_min: div_round_u16(obs.telemetry.watts_min, 10),
                w_max: div_round_u16(obs.telemetry.watts_max, 10),
                temp_c: obs.telemetry.temp_whole_c(),
                energy_wh: obs.telemetry.energy_wh(),
                module_class: obs.telemetry.module_class,
                faults: fault_names(&obs.faults),
                last_job: obs.last_job.clone(),
            })
            .collect();
        let planes: Vec<PlanePayload> = o.planes.iter().map(plane_payload).collect();
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
            planes,
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

/// Round a plane's deciwatt figure to whole watts.
fn div_round_u16(value: u16, by: u16) -> u16 {
    (value + by / 2) / by
}

/// The names of the faults that are set.
fn fault_names(f: &crate::observed::NodeFaultsObs) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (set, name) in [
        (f.absent, "absent"),
        (f.ov_latched, "ov_latched"),
        (f.power_limit, "power_limit"),
        (f.retry_limit, "retry_limit"),
        (f.unit_latched_off, "unit_latched_off"),
        (f.cross_check, "cross_check"),
    ] {
        if set {
            out.push(String::from(name));
        }
    }
    out
}

/// One plane of the state payload.
fn plane_payload(p: &PlaneObs) -> PlanePayload {
    PlanePayload {
        plane: p.plane,
        address: p.address,
        plane_type: p.plane_type.clone(),
        firmware: p.firmware.clone(),
        bus_v: f32::from(p.bus_volts_tenths()) / 10.0,
        temp_c: div_round_i16(p.core_temperature, 10),
        boot_count: p.boot_count,
        uptime_s: p.uptime_s,
        image_state: p.image_state.clone(),
        sequence_running: p.sequence_running,
        intended_state_lost: p.intended_state_lost,
        units: p
            .units
            .iter()
            .map(|u| UnitPayload {
                unit: u.unit,
                present: u.present,
                latched_off: u.latched_off,
                v: f32::from(u.volts) / 100.0,
                a: f32::from(u.amps_tenths()) / 10.0,
                w: u.watts_whole(),
                temp_c: div_round_i16(u.temperature, 10),
            })
            .collect(),
    }
}

/// Round a deci-degree reading to whole degrees.
fn div_round_i16(value: i16, by: i16) -> i16 {
    if value >= 0 {
        (value + by / 2) / by
    } else {
        -((-value + by / 2) / by)
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
