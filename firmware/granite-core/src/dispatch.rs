//! The one dispatcher. MQTT, the HTTP API and Modbus all build a
//! [`Command`] and call [`dispatch`], so the three transports cannot
//! drift apart in what they allow (ADR components 7, 8, 9).
//!
//! The dispatcher never touches hardware other than through the
//! actuator's [`crate::hal::NodeSwitches`]; everything it cannot do
//! itself (probe scan, OTA, reboot, persisting a config section) comes
//! back as a [`SideEffect`] for the firmware to carry out.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::Value;

use crate::actuator::{Actuator, ActuatorEvent, Submission};
use crate::config::{Config, Section};
use crate::hal::NodeSwitches;
use crate::msg::{Command, CommandKind, PROTOCOL_VERSION, Reply};
use crate::observed::Observed;
use crate::rules::RuleEngine;

/// Work the firmware has to do on behalf of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SideEffect {
    /// Re-enumerate the 1-wire bus.
    ProbeScan,
    /// Persist one config section as it now stands in `Config`.
    SaveSection(Section),
    /// Persist one config section through the commit-confirm path,
    /// because getting it wrong can cut the session.
    StageSection(Section),
    /// Pull and install an image.
    Ota {
        /// HTTPS URL.
        url: String,
        /// Expected SHA-256 of the image, hex.
        sha256: String,
    },
    /// Restart.
    Reboot,
    /// Erase everything but the factory namespace, then restart.
    FactoryReset,
}

/// Everything one command produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Dispatched {
    /// The ack to publish or return.
    pub reply: Reply,
    /// Actuator events to publish and to fold into the node tracker.
    pub events: Vec<ActuatorEvent>,
    /// What the firmware still has to do.
    pub effects: Vec<SideEffect>,
}

/// What the dispatcher is allowed to touch.
pub struct DispatchCtx<'a, S: NodeSwitches> {
    /// Monotonic now.
    pub now_ms: u64,
    /// Device id, needed by `factory_reset`'s confirmation.
    pub device_id: &'a str,
    /// The current snapshot.
    pub observed: &'a Observed,
    /// The actuator queue.
    pub actuator: &'a mut Actuator<S>,
    /// The rule engine.
    pub rules: &'a mut RuleEngine,
    /// The live configuration.
    pub config: &'a mut Config,
    /// The reserved signature hook (ADR: `Command.sig`). `None` means
    /// "the transport is the authentication", which is the shipped
    /// behaviour.
    pub verify_sig: Option<&'a dyn Fn(&Command) -> bool>,
}

/// Route one command.
pub fn dispatch<S: NodeSwitches>(cmd: &Command, ctx: &mut DispatchCtx<'_, S>) -> Dispatched {
    let mut events = Vec::new();
    let mut effects = Vec::new();

    if cmd.v != PROTOCOL_VERSION {
        return done(
            Reply::err(
                &cmd.id,
                alloc::format!("unsupported payload version {}", cmd.v),
            ),
            events,
            effects,
        );
    }
    if let Some(verify) = ctx.verify_sig
        && !verify(cmd)
    {
        return done(Reply::err(&cmd.id, "signature rejected"), events, effects);
    }

    let reply = match cmd.action {
        k if k.is_actuator() => {
            let Some(req) = cmd.to_action_request() else {
                return done(Reply::err(&cmd.id, "not an action"), events, effects);
            };
            let (submission, mut evs) = ctx.actuator.submit(&req, ctx.now_ms, ctx.observed);
            events.append(&mut evs);
            match submission {
                Submission::Accepted { .. } => Reply::accepted(&cmd.id),
                Submission::Done(result) => Reply::of_result(&cmd.id, &result),
            }
        }
        CommandKind::RuleAck => match cmd.args.rule {
            None => Reply::err(&cmd.id, "args.rule is required"),
            Some(id) => {
                if ctx.rules.ack(id) {
                    Reply::ok(&cmd.id, "ok")
                } else {
                    Reply::err(&cmd.id, alloc::format!("no rule {id}"))
                }
            }
        },
        CommandKind::ProbeScan => {
            effects.push(SideEffect::ProbeScan);
            Reply::accepted(&cmd.id)
        }
        CommandKind::ConfigGet => match config_get(ctx.config, cmd.args.section) {
            Ok(v) => Reply::ok(&cmd.id, "ok").with_data(v),
            Err(e) => Reply::err(&cmd.id, e),
        },
        CommandKind::ConfigSet => {
            match config_set(ctx.config, cmd.args.section, &cmd.args.config) {
                Ok(sections) => {
                    let mut staged = false;
                    for s in sections {
                        if s.affects_reachability() {
                            staged = true;
                            effects.push(SideEffect::StageSection(s));
                        } else {
                            effects.push(SideEffect::SaveSection(s));
                        }
                    }
                    Reply::ok(&cmd.id, if staged { "staged" } else { "saved" })
                }
                Err(e) => Reply::err(&cmd.id, e),
            }
        }
        CommandKind::Ota => match (&cmd.args.url, &cmd.args.sha256) {
            (Some(url), Some(sha256)) if !url.is_empty() && sha256.len() == 64 => {
                effects.push(SideEffect::Ota {
                    url: url.clone(),
                    sha256: sha256.to_ascii_lowercase(),
                });
                Reply::accepted(&cmd.id)
            }
            _ => Reply::err(
                &cmd.id,
                "args.url and a 64 hex digit args.sha256 are required",
            ),
        },
        CommandKind::Reboot => {
            effects.push(SideEffect::Reboot);
            Reply::accepted(&cmd.id)
        }
        CommandKind::FactoryReset => match &cmd.args.confirm {
            Some(c) if c == ctx.device_id => {
                effects.push(SideEffect::FactoryReset);
                Reply::accepted(&cmd.id)
            }
            _ => Reply::err(&cmd.id, "args.confirm must be the device id"),
        },
        // Every actuator kind is handled by the first arm.
        _ => Reply::err(&cmd.id, "unhandled action"),
    };

    done(reply, events, effects)
}

fn done(reply: Reply, events: Vec<ActuatorEvent>, effects: Vec<SideEffect>) -> Dispatched {
    Dispatched {
        reply,
        events,
        effects,
    }
}

/// `config_get`: one section, or the whole exportable document.
fn config_get(cfg: &Config, section: Option<Section>) -> Result<Value, String> {
    let json = match section {
        Some(s) => cfg.section_json(s).map_err(|e| e.to_string())?,
        None => cfg.export_json().map_err(|e| e.to_string())?,
    };
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

/// `config_set`: one section, or a whole document. Returns the sections
/// that changed and therefore need persisting.
fn config_set(
    cfg: &mut Config,
    section: Option<Section>,
    body: &Option<Value>,
) -> Result<Vec<Section>, String> {
    let Some(body) = body else {
        return Err(String::from("args.config is required"));
    };
    let text = serde_json::to_string(body).map_err(|e| e.to_string())?;
    match section {
        Some(s) => {
            let mut probe = cfg.clone();
            probe
                .set_section_json(s, &text)
                .map_err(|e| e.to_string())?;
            probe.validate().map_err(|errs| errs.join("; "))?;
            *cfg = probe;
            Ok(alloc::vec![s])
        }
        None => {
            let next = Config::import_json(&text).map_err(|e| e.to_string())?;
            let mut changed = Vec::new();
            for s in Section::ALL {
                if cfg.section_json(s).ok() != next.section_json(s).ok() {
                    changed.push(s);
                }
            }
            *cfg = next;
            Ok(changed)
        }
    }
}
