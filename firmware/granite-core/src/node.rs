//! Per-node state: what the LED says, what the actuator is doing, what the
//! user named it, and what should happen to it at controller boot.

use alloc::format;
use alloc::string::String;
use core::array;
use core::fmt;

use serde::{Deserialize, Serialize};

use crate::NODE_COUNT;
use crate::actuator::ActionKind;
use crate::{NodeId, node_index};

/// How long a power LED must read the same before the core believes it
/// (ADR: 100 ms debounce).
pub const DEBOUNCE_MS: u64 = 100;

/// What the controller is willing to claim about a node.
///
/// `Hung` is deliberately absent: the controller cannot know it. A reader
/// that wants to guess has `On` plus [`Nodes::last_reset_ms`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NodeState {
    /// U15 is unreadable, or the node's sense is configured as `ignore`.
    Unknown,
    /// LED stably off.
    Off,
    /// LED stably on.
    On,
    /// An action is in flight on this node.
    Busy {
        /// What is running.
        action: ActionKind,
        /// When it was accepted, in monotonic milliseconds.
        started_ms: u64,
    },
}

impl NodeState {
    /// Short wire name, without the `Busy` payload.
    pub const fn as_str(&self) -> &'static str {
        match self {
            NodeState::Unknown => "unknown",
            NodeState::Off => "off",
            NodeState::On => "on",
            NodeState::Busy { .. } => "busy",
        }
    }

    /// Modbus encoding: 0 unknown, 1 off, 2 on, 3 busy.
    pub const fn as_modbus(&self) -> u16 {
        match self {
            NodeState::Unknown => 0,
            NodeState::Off => 1,
            NodeState::On => 2,
            NodeState::Busy { .. } => 3,
        }
    }

    /// True while an action is in flight.
    pub const fn is_busy(&self) -> bool {
        matches!(self, NodeState::Busy { .. })
    }
}

impl fmt::Display for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What to do with a node once per controller boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootPolicy {
    /// Touch nothing. The default: a controller restart never moves a node.
    #[default]
    Leave,
    /// Power the node on if it is off.
    On,
    /// Power the node off if it is on.
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

/// Whether the power LED of a node can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenseMode {
    /// Normal: actions check the LED before and after.
    #[default]
    Enabled,
    /// The LED cannot be read on this board. Every action behaves like a
    /// raw `press` with the standard durations, and nothing is refused for
    /// an unknown state.
    Ignore,
}

impl SenseMode {
    /// True when the LED must not be consulted.
    pub const fn is_ignored(self) -> bool {
        matches!(self, SenseMode::Ignore)
    }
}

/// Per-node user settings; the `nodes` config section owns an array of these.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeSettings {
    /// Display name.
    pub name: String,
    /// Boot policy, applied once per controller boot.
    pub boot_policy: BootPolicy,
    /// Whether the power LED is trusted.
    pub sense: SenseMode,
}

impl Default for NodeSettings {
    fn default() -> Self {
        NodeSettings {
            name: String::new(),
            boot_policy: BootPolicy::Leave,
            sense: SenseMode::Enabled,
        }
    }
}

impl NodeSettings {
    /// Defaults for node `node` (1..=8), with the default name.
    pub fn for_node(node: NodeId) -> Self {
        NodeSettings {
            name: default_name(node),
            ..Default::default()
        }
    }
}

/// Default display name for a node id.
pub fn default_name(node: NodeId) -> String {
    format!("node{node}")
}

/// What the state publisher reports as a node's last action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastAction {
    /// Command id that asked for it.
    pub id: String,
    /// Which action.
    pub action: ActionKind,
    /// When it was accepted.
    pub started_ms: u64,
    /// When it finished, if it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_ms: Option<u64>,
    /// Outcome, once known ("ok", "shutdown_pending", "refused: ...").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
}

/// A debounced boolean. `None` means "no trustworthy value".
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Debounce {
    stable: Option<bool>,
    candidate: Option<bool>,
    since_ms: u64,
    updated_ms: u64,
}

impl Debounce {
    /// Feed a raw sample. `raw == None` means the source is unreadable,
    /// which drops the debounced value immediately (the ADR wants
    /// `Unknown` the moment U15 cannot be read, not 100 ms later).
    pub fn update(&mut self, now_ms: u64, raw: Option<bool>, window_ms: u64) -> Option<bool> {
        match raw {
            None => {
                self.stable = None;
                self.candidate = None;
                self.since_ms = now_ms;
                self.updated_ms = now_ms;
            }
            Some(v) => {
                if self.candidate != Some(v) {
                    self.candidate = Some(v);
                    self.since_ms = now_ms;
                }
                if self.stable != Some(v) && now_ms.saturating_sub(self.since_ms) >= window_ms {
                    self.stable = Some(v);
                    self.updated_ms = now_ms;
                }
            }
        }
        self.stable
    }

    /// Current debounced value.
    pub const fn value(&self) -> Option<bool> {
        self.stable
    }

    /// When the debounced value last changed.
    pub const fn updated_ms(&self) -> u64 {
        self.updated_ms
    }
}

/// The controller's view of all 8 nodes.
#[derive(Debug, Clone, Default)]
pub struct Nodes {
    led: [Debounce; NODE_COUNT],
    busy: [Option<(ActionKind, u64)>; NODE_COUNT],
    last_action: [Option<LastAction>; NODE_COUNT],
    last_reset_ms: [Option<u64>; NODE_COUNT],
    sense: [SenseMode; NODE_COUNT],
}

impl Nodes {
    /// Fresh tracker: every node `Unknown`, nothing busy.
    pub fn new() -> Self {
        Nodes {
            led: [Debounce::default(); NODE_COUNT],
            busy: array::from_fn(|_| None),
            last_action: array::from_fn(|_| None),
            last_reset_ms: array::from_fn(|_| None),
            sense: [SenseMode::Enabled; NODE_COUNT],
        }
    }

    /// Apply the per-node sense flags from config.
    pub fn set_sense(&mut self, sense: [SenseMode; NODE_COUNT]) {
        self.sense = sense;
    }

    /// Feed a sense cycle. `leds == None` means U15 was unreadable, which
    /// makes every node `Unknown`.
    pub fn update_sense(&mut self, now_ms: u64, leds: Option<u8>) {
        for i in 0..NODE_COUNT {
            let raw = match (leds, self.sense[i]) {
                // A node whose LED is not trusted never reports on/off.
                (_, SenseMode::Ignore) => None,
                (None, _) => None,
                (Some(bits), _) => Some(bits & (1 << i) != 0),
            };
            self.led[i].update(now_ms, raw, DEBOUNCE_MS);
        }
    }

    /// Debounced LED of a node, `None` if unknown or ignored.
    pub fn led(&self, node: NodeId) -> Option<bool> {
        node_index(node).and_then(|i| self.led[i].value())
    }

    /// When the LED state of a node last changed.
    pub fn led_updated_ms(&self, node: NodeId) -> u64 {
        node_index(node).map_or(0, |i| self.led[i].updated_ms())
    }

    /// Reported state of a node. `Busy` wins over the LED.
    pub fn state(&self, node: NodeId) -> NodeState {
        let Some(i) = node_index(node) else {
            return NodeState::Unknown;
        };
        if let Some((action, started_ms)) = self.busy[i] {
            return NodeState::Busy { action, started_ms };
        }
        match self.led[i].value() {
            None => NodeState::Unknown,
            Some(true) => NodeState::On,
            Some(false) => NodeState::Off,
        }
    }

    /// Mark a node busy with `action`.
    pub fn mark_busy(&mut self, node: NodeId, action: ActionKind, now_ms: u64) {
        if let Some(i) = node_index(node) {
            self.busy[i] = Some((action, now_ms));
        }
    }

    /// Clear the busy marker.
    pub fn clear_busy(&mut self, node: NodeId) {
        if let Some(i) = node_index(node) {
            self.busy[i] = None;
        }
    }

    /// Record (or update) the last action of a node.
    pub fn note_action(&mut self, node: NodeId, action: LastAction) {
        if let Some(i) = node_index(node) {
            self.last_action[i] = Some(action);
        }
    }

    /// Last action of a node.
    pub fn last_action(&self, node: NodeId) -> Option<&LastAction> {
        node_index(node).and_then(|i| self.last_action[i].as_ref())
    }

    /// Remember that a node was reset, so a reader can reason about `Hung`.
    pub fn note_reset(&mut self, node: NodeId, now_ms: u64) {
        if let Some(i) = node_index(node) {
            self.last_reset_ms[i] = Some(now_ms);
        }
    }

    /// When this node was last reset by the controller.
    pub fn last_reset_ms(&self, node: NodeId) -> Option<u64> {
        node_index(node).and_then(|i| self.last_reset_ms[i])
    }

    /// Bitmap of the debounced LEDs, unknown nodes reading 0.
    pub fn led_bits(&self) -> u8 {
        let mut bits = 0u8;
        for i in 0..NODE_COUNT {
            if self.led[i].value() == Some(true) {
                bits |= 1 << i;
            }
        }
        bits
    }
}
