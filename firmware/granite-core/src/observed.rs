//! The one sensor snapshot. Publishers, the rule engine and the actuator
//! read this; none of them ever touch the hardware directly (ADR
//! component 4, "Sensing").
//!
//! Every field carries the monotonic millisecond timestamp of the reading
//! that produced it, so a consumer can tell a fresh zero from a stale one.

use alloc::string::String;
use alloc::vec::Vec;
use core::array;

use serde::{Deserialize, Serialize};

use crate::hal::RomId;
use crate::node::{LastAction, NodeState, Nodes, SenseMode};
use crate::{NODE_COUNT, NodeId, node_index};

/// A value plus when it was read. `ts_ms == 0` means "never read".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Stamped<T> {
    /// The reading.
    pub value: T,
    /// Monotonic milliseconds at the time of the reading.
    pub ts_ms: u64,
}

impl<T> Stamped<T> {
    /// A reading taken at `ts_ms`.
    pub const fn new(value: T, ts_ms: u64) -> Self {
        Stamped { value, ts_ms }
    }

    /// A placeholder that was never read.
    pub const fn never(value: T) -> Self {
        Stamped { value, ts_ms: 0 }
    }

    /// True when the reading is missing or older than `max_age_ms`.
    pub fn is_stale(&self, now_ms: u64, max_age_ms: u64) -> bool {
        self.ts_ms == 0 || now_ms.saturating_sub(self.ts_ms) > max_age_ms
    }
}

/// What is known about one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeObs {
    /// Reported state (`Busy` while an action runs).
    pub state: NodeState,
    /// Debounced power LED, `None` when unknown or ignored. The actuator
    /// reads this rather than `state`, because `Busy` hides the LED.
    pub led: Option<bool>,
    /// Whether this node's LED is trusted.
    pub sense: SenseMode,
    /// Last action the controller ran on this node.
    pub last_action: Option<LastAction>,
    /// When the controller last reset this node.
    pub last_reset_ms: Option<u64>,
    /// When the LED state last changed.
    pub ts_ms: u64,
}

impl Default for NodeObs {
    fn default() -> Self {
        NodeObs {
            state: NodeState::Unknown,
            led: None,
            sense: SenseMode::Enabled,
            last_action: None,
            last_reset_ms: None,
            ts_ms: 0,
        }
    }
}

/// One DS18B20, present or missing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbeObs {
    /// ROM id.
    pub rom: RomId,
    /// User name from config, empty if unnamed.
    pub name: String,
    /// Centi-degrees Celsius, `None` when the probe did not answer.
    pub centi_c: Option<i16>,
    /// When it was last read.
    pub ts_ms: u64,
}

/// The snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observed {
    /// Index 0 is node 1.
    pub nodes: [NodeObs; NODE_COUNT],
    /// Probes in config order; slot index is the Modbus probe register.
    pub probes: Vec<ProbeObs>,
    /// TMP1075, centi-degrees Celsius.
    pub board_temp: Stamped<Option<i16>>,
    /// VIN in millivolts.
    pub vin_mv: Stamped<Option<u32>>,
    /// Dry contacts, bit 0 = input 1, true = closed.
    pub dry_in: Stamped<Option<u8>>,
    /// Ethernet link.
    pub link_up: Stamped<bool>,
    /// MQTT session.
    pub mqtt_connected: Stamped<bool>,
    /// Seconds since boot.
    pub uptime_s: Stamped<u32>,
}

impl Default for Observed {
    fn default() -> Self {
        Observed {
            nodes: array::from_fn(|_| NodeObs::default()),
            probes: Vec::new(),
            board_temp: Stamped::never(None),
            vin_mv: Stamped::never(None),
            dry_in: Stamped::never(None),
            link_up: Stamped::never(false),
            mqtt_connected: Stamped::never(false),
            uptime_s: Stamped::never(0),
        }
    }
}

impl Observed {
    /// Empty snapshot: everything unknown.
    pub fn new() -> Self {
        Self::default()
    }

    /// Copy the node view out of a [`Nodes`] tracker.
    pub fn refresh_nodes(&mut self, nodes: &Nodes) {
        for i in 0..NODE_COUNT {
            let node = (i + 1) as NodeId;
            self.nodes[i] = NodeObs {
                state: nodes.state(node),
                led: nodes.led(node),
                sense: self.nodes[i].sense,
                last_action: nodes.last_action(node).cloned(),
                last_reset_ms: nodes.last_reset_ms(node),
                ts_ms: nodes.led_updated_ms(node),
            };
        }
    }

    /// Debounced LED of a node.
    pub fn node_led(&self, node: NodeId) -> Option<bool> {
        node_index(node).and_then(|i| self.nodes[i].led)
    }

    /// Reported state of a node.
    pub fn node_state(&self, node: NodeId) -> NodeState {
        node_index(node).map_or(NodeState::Unknown, |i| self.nodes[i].state.clone())
    }

    /// Sense mode of a node.
    pub fn node_sense(&self, node: NodeId) -> SenseMode {
        node_index(node).map_or(SenseMode::Enabled, |i| self.nodes[i].sense)
    }

    /// Probe in slot `slot` (0-based), if configured.
    pub fn probe(&self, slot: usize) -> Option<&ProbeObs> {
        self.probes.get(slot)
    }

    /// Highest probe reading in centi-degrees, `None` when no probe answered.
    pub fn probe_max(&self) -> Option<i16> {
        self.probes.iter().filter_map(|p| p.centi_c).max()
    }

    /// Dry input `n` (1-based), `None` when U15 was never read.
    pub fn dry(&self, n: u8) -> Option<bool> {
        if n == 0 || n as usize > crate::DRY_COUNT {
            return None;
        }
        self.dry_in.value.map(|bits| bits & (1 << (n - 1)) != 0)
    }
}
