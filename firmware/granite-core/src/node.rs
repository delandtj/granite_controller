//! Per-node settings and the last job a node was given.
//!
//! A node is a slot on a plane (ADR 0004 decision 2): its *state* comes
//! from the plane and lives in [`crate::plane::model::Node`], and what
//! the operator decides about it lives here - the name, what to do with
//! it at a plane boot, whether its power LED can be trusted, how long a
//! soft off waits and how far it escalates (decisions 2 and 4).
//!
//! Settings are kept for nodes that are not present: a plane that is
//! unplugged for a day comes back to the names and policies it had
//! (decision 2). That is why the `nodes` config section is a list keyed
//! by node id and not an array of the nodes that happen to answer.

use alloc::format;
use alloc::string::String;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::NodeId;

/// What to do with a node once per plane boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootPolicy {
    /// Touch nothing. The default: neither a controller nor a plane
    /// restart moves a node.
    #[default]
    Leave,
    /// Power the node on if its rail is off.
    On,
    /// Power the node off if its rail is on.
    Off,
}

impl BootPolicy {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            BootPolicy::Leave => "leave",
            BootPolicy::On => "on",
            BootPolicy::Off => "off",
        }
    }
}

impl fmt::Display for BootPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How far `off` and `force_off` go when the host ignores its button
/// (ADR 0004 decision 4, user 2026-10-29).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Escalation {
    /// Short press only; the verb fails if the host stays up.
    None,
    /// Hold PWR for five seconds, and leave the rail alone.
    Hold,
    /// Hold PWR for five seconds, then cut the rail if the host is
    /// still lit. The default: `force_off` always ends with the node
    /// off.
    #[default]
    HoldThenCut,
}

impl Escalation {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Escalation::None => "none",
            Escalation::Hold => "hold",
            Escalation::HoldThenCut => "hold_then_cut",
        }
    }

    /// The plane's escalation code (ADR 0003 CTRL 0x51).
    pub const fn as_wire(self) -> granite_proto::Escalate {
        match self {
            Escalation::None => granite_proto::Escalate::None,
            Escalation::Hold => granite_proto::Escalate::HoldPwr,
            Escalation::HoldThenCut => granite_proto::Escalate::HoldPwrThenCutRail,
        }
    }
}

impl fmt::Display for Escalation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Default soft-off window \[s\]: how long `off` waits for the host to
/// go dark before it escalates. The rev C value.
pub const DEFAULT_SOFT_OFF_S: u16 = crate::plane::OFF_TIMEOUT_S;

/// Longest soft-off window a config may ask for \[s\]. The plane carries
/// the timeout as a `u16` of seconds and runs the wait itself; an hour
/// is already far past any board's shutdown.
pub const MAX_SOFT_OFF_S: u16 = 3_600;

/// One node's settings. The `nodes` config section owns a list of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeSettings {
    /// Node id 1..=64. The key of the entry.
    pub node: NodeId,
    /// Display name. Empty means [`default_name`].
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Boot policy, applied once per plane boot.
    pub boot_policy: BootPolicy,
    /// Whether the plane's reading of the host's power LED is trusted.
    /// With it off, `on` and `off` never wait for the light.
    pub led_sense: bool,
    /// How long `off` waits for the host \[s\].
    pub t_soft_off_s: u16,
    /// How far `off` and `force_off` escalate.
    pub escalation: Escalation,
}

impl Default for NodeSettings {
    fn default() -> Self {
        NodeSettings {
            node: 0,
            name: String::new(),
            boot_policy: BootPolicy::Leave,
            led_sense: true,
            t_soft_off_s: DEFAULT_SOFT_OFF_S,
            escalation: Escalation::HoldThenCut,
        }
    }
}

impl NodeSettings {
    /// Defaults for one node.
    pub fn for_node(node: NodeId) -> Self {
        NodeSettings {
            node,
            ..Default::default()
        }
    }

    /// The display name: the configured one, or the default.
    pub fn display_name(&self) -> String {
        if self.name.trim().is_empty() {
            default_name(self.node)
        } else {
            self.name.clone()
        }
    }

    /// Clamp what can be clamped silently.
    pub fn clamped(mut self) -> Self {
        if self.t_soft_off_s == 0 {
            self.t_soft_off_s = DEFAULT_SOFT_OFF_S;
        }
        if self.t_soft_off_s > MAX_SOFT_OFF_S {
            self.t_soft_off_s = MAX_SOFT_OFF_S;
        }
        self
    }
}

/// Default display name for a node id.
pub fn default_name(node: NodeId) -> String {
    format!("node{node}")
}

/// What the state publisher reports as a node's last job.
///
/// A command is accepted and finishes later (ADR 0004 decision 6), so
/// what a reader wants is the verb, when it was accepted, and what the
/// plane said in the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastJob {
    /// Command id that asked for it.
    pub id: String,
    /// The plane job id, so an ack can be matched to it.
    pub job: u32,
    /// The verb ("on", "force_off", "unit_reset", ...).
    pub verb: String,
    /// When it was accepted \[ms\].
    pub started_ms: u64,
    /// When it finished, if it has \[ms\].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    /// Outcome, once known ("done", "failed", "absent", "link_lost",
    /// "timed_out").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_the_adr_values() {
        let s = NodeSettings::for_node(9);
        assert_eq!(s.display_name(), "node9");
        assert_eq!(s.boot_policy, BootPolicy::Leave);
        assert!(s.led_sense);
        assert_eq!(s.escalation, Escalation::HoldThenCut);
        assert_eq!(
            s.escalation.as_wire(),
            granite_proto::Escalate::HoldPwrThenCutRail
        );
        assert_eq!(s.t_soft_off_s, DEFAULT_SOFT_OFF_S);

        let clamped = NodeSettings {
            t_soft_off_s: 0,
            ..NodeSettings::for_node(1)
        }
        .clamped();
        assert_eq!(clamped.t_soft_off_s, DEFAULT_SOFT_OFF_S);
    }
}
