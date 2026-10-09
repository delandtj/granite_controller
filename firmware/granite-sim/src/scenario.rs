//! The scenario file: what the fake hardware does.
//!
//! ```toml
//! [hardware]
//! vin_mv = 19000
//! board_temp_c = 32.5
//! dry_in = [false, false, false, false]
//!
//! [[probes]]
//! rom = "28ff0123456789ab"
//! name = "inlet"
//! temp_c = 41.0
//! ramp_c_per_min = 0.0
//!
//! [[nodes]]
//! node = 1
//! powered = false
//! respond = "normal"   # normal | never | slow
//! on_delay_ms = 1500
//! off_delay_ms = 4000
//! ```
//!
//! Everything is optional; the defaults are a healthy 8-node frame with
//! every node off, one probe at 40 C and 19 V on the bus.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// How a node's power LED answers a press.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Respond {
    /// A cooperative motherboard: short press powers on, short press
    /// starts an ACPI shutdown, a 4 s hold cuts power.
    #[default]
    Normal,
    /// The LED never changes: `on` times out, `off` escalates to
    /// `force_off`, and `press` still works. This is the hung-node case.
    Never,
    /// Like `normal` but slower than the default timings, so `off`
    /// escalates and `on` is late.
    Slow,
}

/// One node in the scenario.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeScenario {
    /// Node id, 1..=8.
    pub node: u8,
    /// Initial LED state.
    pub powered: bool,
    /// How it answers presses.
    pub respond: Respond,
    /// How long after a short press the LED comes on.
    pub on_delay_ms: u64,
    /// How long after a short press the LED goes off (ACPI shutdown).
    pub off_delay_ms: u64,
    /// The LED wire is broken: this node always reads dark, even while it
    /// is powered. That is the failure mode ADR 0001 calls out ("an unlit
    /// LED on a running node reads as Off"), and the reason a per-node
    /// `sense = ignore` exists. When every node is marked broken the
    /// whole U15 read fails instead and every node is `unknown`.
    pub sense_broken: bool,
}

impl Default for NodeScenario {
    fn default() -> Self {
        NodeScenario {
            node: 1,
            powered: false,
            respond: Respond::Normal,
            on_delay_ms: 1_500,
            off_delay_ms: 4_000,
            sense_broken: false,
        }
    }
}

/// One DS18B20 on the fake 1-wire bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProbeScenario {
    /// ROM id, 16 hex digits.
    pub rom: String,
    /// Name the controller should map it to.
    pub name: String,
    /// Starting temperature.
    pub temp_c: f32,
    /// Linear drift, so a rule with a threshold can be made to fire.
    pub ramp_c_per_min: f32,
}

impl Default for ProbeScenario {
    fn default() -> Self {
        ProbeScenario {
            rom: String::from("28ff000000000001"),
            name: String::from("probe 1"),
            temp_c: 40.0,
            ramp_c_per_min: 0.0,
        }
    }
}

/// The board-level sensors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HardwareScenario {
    /// Bus voltage.
    pub vin_mv: u32,
    /// TMP1075 reading.
    pub board_temp_c: f32,
    /// The four dry contacts, true = closed.
    pub dry_in: [bool; 4],
    /// Ethernet link.
    pub link_up: bool,
    /// Drift on the bus voltage, so the low-voltage rule can be tested.
    pub vin_ramp_mv_per_min: i32,
}

impl Default for HardwareScenario {
    fn default() -> Self {
        HardwareScenario {
            vin_mv: 19_000,
            board_temp_c: 32.0,
            dry_in: [false; 4],
            link_up: true,
            vin_ramp_mv_per_min: 0,
        }
    }
}

/// The whole file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scenario {
    /// Device id the simulator claims. Empty means `granite-510000`.
    pub device_id: String,
    /// MAC the simulator reports.
    pub mac: String,
    /// Firmware version the simulator reports.
    pub fw: String,
    /// Per-device recovery token. Empty means a generated one.
    pub recovery_token: String,
    /// Board sensors.
    pub hardware: HardwareScenario,
    /// Probes on the bus.
    pub probes: Vec<ProbeScenario>,
    /// Nodes.
    pub nodes: Vec<NodeScenario>,
}

impl Default for Scenario {
    fn default() -> Self {
        Scenario {
            device_id: String::from("granite-510000"),
            mac: String::from("02:00:00:51:00:00"),
            fw: String::from(concat!("sim-", env!("CARGO_PKG_VERSION"))),
            recovery_token: String::new(),
            hardware: HardwareScenario::default(),
            probes: vec![ProbeScenario::default()],
            nodes: (1..=8)
                .map(|node| NodeScenario {
                    node,
                    ..Default::default()
                })
                .collect(),
        }
    }
}

impl Scenario {
    /// Read a scenario file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut scenario: Scenario = toml::from_str(&text)?;
        scenario.normalise();
        Ok(scenario)
    }

    /// Fill in the nodes the file left out and fix the obvious mistakes.
    pub fn normalise(&mut self) {
        self.nodes.retain(|n| (1..=8).contains(&n.node));
        for node in 1..=8u8 {
            if !self.nodes.iter().any(|n| n.node == node) {
                self.nodes.push(NodeScenario {
                    node,
                    ..Default::default()
                });
            }
        }
        self.nodes.sort_by_key(|n| n.node);
        self.probes.truncate(granite_core::PROBE_SLOTS);
        if self.device_id.trim().is_empty() {
            self.device_id = String::from("granite-510000");
        }
    }

    /// The scenario for one node.
    pub fn node(&self, node: u8) -> NodeScenario {
        self.nodes
            .iter()
            .find(|n| n.node == node)
            .cloned()
            .unwrap_or_default()
    }
}
