//! The one dispatcher. MQTT, the HTTP API and Modbus all build a
//! [`Command`] and call [`dispatch`], so the three transports cannot
//! drift apart in what they allow (ADR 0001 components 7, 8, 9).
//!
//! Since rev D the dispatcher touches no hardware at all (ADR 0004
//! decision 6): it checks the target, turns the verb into a plane
//! command with the node's configured timings, hands it to the
//! [`PlaneClient`] and answers `accepted` with the job id. The job
//! finishes later, and the ack that carries the plane's result is built
//! from [`crate::jobs`]. A command for a node, unit or plane that is not
//! there fails here, before any write: the plane client itself finishes
//! such a job at once, so the reply is a refusal rather than an ack.
//!
//! Everything the dispatcher cannot do itself (probe scan, OTA, reboot,
//! persisting a config section) still comes back as a [`SideEffect`] for
//! the firmware to carry out.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::Value;

use crate::config::{Config, Section};
use crate::jobs::JobLog;
use crate::msg::{Command, CommandKind, PROTOCOL_VERSION, Reply, TargetKind};
use crate::observed::Observed;
use crate::plane::{
    Command as PlaneJob, JobId, NodeCommand, NodeOptions, PlaneClient, PlaneCommand,
};
use crate::rules::RuleEngine;
use crate::{NodeId, Target};

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
    /// The plane job it became, if it became one.
    pub job: Option<JobId>,
    /// What the firmware still has to do.
    pub effects: Vec<SideEffect>,
}

/// What the dispatcher is allowed to touch.
pub struct DispatchCtx<'a> {
    /// Monotonic now.
    pub now_ms: u64,
    /// Device id, needed by `factory_reset`'s confirmation.
    pub device_id: &'a str,
    /// The current snapshot: what the target check reads.
    pub observed: &'a Observed,
    /// The plane client every verb goes through.
    pub plane: &'a mut PlaneClient,
    /// The pending-job log, so the ack can find the command id again.
    pub jobs: &'a mut JobLog,
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
pub fn dispatch(cmd: &Command, ctx: &mut DispatchCtx<'_>) -> Dispatched {
    let mut effects = Vec::new();

    if cmd.v != PROTOCOL_VERSION {
        return done(
            Reply::err(
                &cmd.id,
                alloc::format!("unsupported payload version {}", cmd.v),
            ),
            effects,
        );
    }
    if let Some(verify) = ctx.verify_sig
        && !verify(cmd)
    {
        return done(Reply::err(&cmd.id, "signature rejected"), effects);
    }

    if cmd.action.is_plane_verb() {
        return submit(cmd, ctx);
    }

    let reply = match cmd.action {
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
        // Every plane verb is handled above.
        _ => Reply::err(&cmd.id, "unhandled action"),
    };

    done(reply, effects)
}

fn done(reply: Reply, effects: Vec<SideEffect>) -> Dispatched {
    Dispatched {
        reply,
        job: None,
        effects,
    }
}

/// Check the target, submit the plane job, answer `accepted`.
fn submit(cmd: &Command, ctx: &mut DispatchCtx<'_>) -> Dispatched {
    let job = match plane_job(cmd, ctx) {
        Ok(job) => job,
        Err(e) => return done(Reply::err(&cmd.id, e), Vec::new()),
    };
    let nodes = job.nodes();
    let id = ctx.plane.submit(ctx.now_ms, job);
    ctx.jobs
        .note(id, &cmd.id, cmd.action, cmd.target, &nodes, ctx.now_ms);
    Dispatched {
        reply: Reply::accepted(&cmd.id).with_job(id),
        job: Some(id),
        effects: Vec::new(),
    }
}

/// The plane command a verb and a target become, or why they cannot.
///
/// The presence check is the snapshot's: a node whose plane does not
/// answer, or whose slot holds no module, is refused here and nothing is
/// written (ADR 0004 decision 6, "commands to an absent node or plane
/// fail at once"). A node the controller has not read yet is let
/// through: the plane has the last word on presence.
fn plane_job(cmd: &Command, ctx: &DispatchCtx<'_>) -> Result<PlaneJob, String> {
    if !cmd.target.is_valid() {
        return Err(alloc::format!("{} is not a target", cmd.target));
    }
    match cmd.action.wants() {
        TargetKind::Plane => {
            let plane = cmd
                .target
                .plane()
                .ok_or_else(|| String::from("this action needs a plane target"))?;
            let address = plane_address(plane, ctx)?;
            Ok(PlaneJob::Plane(PlaneCommand::Reboot { address }))
        }
        TargetKind::Unit => {
            let (plane, unit) = cmd
                .target
                .unit()
                .ok_or_else(|| String::from("this action needs a unit target"))?;
            let address = plane_address(plane, ctx)?;
            if !ctx.observed.unit(plane, unit).is_some_and(|u| u.present) {
                return Err(alloc::format!(
                    "unit {unit} of plane {plane} is not present"
                ));
            }
            Ok(PlaneJob::Plane(PlaneCommand::UnitReset { address, unit }))
        }
        TargetKind::Node => node_job(cmd, ctx),
    }
}

/// The address of a plane that answers.
fn plane_address(plane: u8, ctx: &DispatchCtx<'_>) -> Result<u8, String> {
    ctx.observed
        .plane(plane)
        .map(|p| p.address)
        .ok_or_else(|| alloc::format!("plane {plane} is not present"))
}

/// A node verb over one node or over every present node.
fn node_job(cmd: &Command, ctx: &DispatchCtx<'_>) -> Result<PlaneJob, String> {
    // `on_all` is a group by definition and ignores a node target, the
    // way rev C did.
    if cmd.action == CommandKind::OnAll {
        let nodes = ctx.observed.present_nodes();
        if nodes.is_empty() {
            return Err(String::from("no node is present"));
        }
        let opts = group_options(&nodes, cmd, ctx);
        return Ok(PlaneJob::Node(NodeCommand::on_all(nodes).with_opts(opts)));
    }
    let Target::Node(node) = cmd.target else {
        // Every other verb acts on one node. `all` over a verb that is
        // not `on_all` would be several jobs with one ack, which the
        // job model has no room for.
        return Err(alloc::format!(
            "{} needs a single node target",
            cmd.action.as_str()
        ));
    };
    check_present(node, ctx)?;
    let opts = cmd.args.apply_to(node_options(node, ctx));
    let verb = match cmd.action {
        CommandKind::On => NodeCommand::on(node),
        CommandKind::Off => NodeCommand::off(node),
        CommandKind::ForceOff => NodeCommand::force_off(node),
        CommandKind::Cycle => NodeCommand::cycle(node),
        CommandKind::Reset => NodeCommand::Reset { node },
        CommandKind::Press => NodeCommand::Press {
            node,
            switch: cmd.args.press_switch(),
            ms: cmd.args.press_ms(),
        },
        CommandKind::PowerOn => NodeCommand::PowerOn { node },
        CommandKind::PowerOff => NodeCommand::PowerOff { node },
        CommandKind::ClearLatch => NodeCommand::ClearLatch { node },
        other => return Err(alloc::format!("{} is not a node action", other.as_str())),
    };
    Ok(PlaneJob::Node(verb.with_opts(opts)))
}

/// A node the controller knows is not there is refused at once.
fn check_present(node: NodeId, ctx: &DispatchCtx<'_>) -> Result<(), String> {
    let (address, _) = crate::plane::node_location(node)
        .ok_or_else(|| alloc::format!("{node} is not a node id"))?;
    if ctx.observed.plane_at(address).is_none() {
        return Err(alloc::format!(
            "node {node} is on plane {} which is not present",
            address - granite_proto::ADDR_BASE
        ));
    }
    match ctx.observed.node(node) {
        Some(n) if n.present => Ok(()),
        Some(_) => Err(alloc::format!("node {node} is not present")),
        // The plane answers but this slot has not been read yet.
        None if ctx
            .observed
            .plane_at(address)
            .is_some_and(|p| p.info_ms != 0 && p.ts_ms == 0) =>
        {
            Ok(())
        }
        None => Err(alloc::format!("node {node} is not present")),
    }
}

/// The timings of one node.
fn node_options(node: NodeId, ctx: &DispatchCtx<'_>) -> NodeOptions {
    NodeOptions::of_settings(&ctx.config.nodes.settings(node))
}

/// The timings of a group: the shortest LED timeout and soft-off window
/// of the nodes in it, so no node in a staggered `on_all` waits longer
/// than its own setting allows.
fn group_options(nodes: &[NodeId], cmd: &Command, ctx: &DispatchCtx<'_>) -> NodeOptions {
    let mut opts = NodeOptions::default();
    for (i, node) in nodes.iter().enumerate() {
        let one = node_options(*node, ctx);
        if i == 0 {
            opts = one;
            continue;
        }
        opts.led_timeout_s = opts.led_timeout_s.min(one.led_timeout_s);
        opts.soft_off_s = opts.soft_off_s.min(one.soft_off_s);
    }
    cmd.args.apply_to(opts)
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
