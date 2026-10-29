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
//! A `[[plane]]` section describes one simulated plane on the link
//! (controller ADR 0004 architecture item 3), its units, what is in each
//! slot and the faults to inject as time passes:
//!
//! ```toml
//! [[plane]]
//! address = 0x30
//! units = [0, 1, 2, 3]
//! blink_window_ms = [100, 500]
//!
//! [[plane.slot]]
//! slot = 0
//! module = true
//! module_id = 0x0400
//! load_ma = 1200
//! host_on = true
//! respond = "normal"       # normal | ignores_short_press | ignores_hold
//! blink_period_ms = 0      # non-zero is a sleeping host
//!
//! [[plane.fault]]
//! at_s = 30
//! kind = "unit_unplug"     # see [`FaultKind`]
//! unit = 1
//! ```
//!
//! Everything is optional; the defaults are a healthy 8-node frame with
//! every node off, one probe at 40 C, 19 V on the bus and one plane at
//! 0x30 with its four units populated.

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

/// How the host behind a plane slot answers its PWR button.
///
/// The plane runs the press itself (ADR 0002 decision 6), so what a
/// scenario gets to choose is how stubborn the motherboard is: that is
/// what the escalation ladder of CTRL 0x51 exists for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaneRespond {
    /// A cooperative motherboard: a short press powers on or starts an
    /// ACPI shutdown, a long hold cuts power.
    #[default]
    Normal,
    /// The host ignores a short press: an operating system that will not
    /// shut down. `off` has to escalate to the 5 s hold.
    IgnoresShortPress,
    /// The host survives even the hold: only cutting the rail ends it
    /// (`force_off` with escalate 3).
    IgnoresHold,
}

/// One slot of a simulated plane: what is screwed into it and what the
/// host behind it does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaneSlotScenario {
    /// Wire slot index 0-7 (2 x unit + A/B).
    pub slot: u8,
    /// A module is screwed down, so its ID resistor answers.
    pub module: bool,
    /// The module's 12-bit ID count; the plane maps it to a class
    /// itself. Full scale (0x0FFF) is an empty slot.
    pub module_id: u16,
    /// What the host draws while its rail is up \[mA\].
    pub load_ma: u32,
    /// The host is running and its power LED is lit.
    pub host_on: bool,
    /// How the host answers its PWR button.
    pub respond: PlaneRespond,
    /// LED blink period while the host is on \[ms\]; 0 is a steady LED,
    /// anything else is a sleeping host.
    pub blink_period_ms: u32,
}

impl Default for PlaneSlotScenario {
    fn default() -> Self {
        PlaneSlotScenario {
            slot: 0,
            module: true,
            module_id: 0x0400,
            load_ma: 1_000,
            host_on: false,
            respond: PlaneRespond::Normal,
            blink_period_ms: 0,
        }
    }
}

/// What a scheduled plane fault does.
///
/// Each one is a thing that happens to the hardware or to the link, not
/// a command: the plane has to notice it on its own, which is the point
/// of having them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    /// Nak the next `count` transactions to this plane.
    #[default]
    DropTransactions,
    /// Corrupt one byte of the next `count` answers, so the
    /// controller's CRC check rejects them.
    CorruptAnswer,
    /// The plane resets on its watchdog: VDD held, so the backup
    /// registers and the units' expanders survive.
    Watchdog,
    /// The plane loses its own power: the backup registers go with it,
    /// OFF slots come back ON and STATUS says "intended state lost".
    PowerLoss,
    /// The unit's chain cable comes out: no 3V3 on that unit.
    UnitUnplug,
    /// The unit's chain cable goes back in.
    UnitReplug,
    /// The unit's own hot-swap (LM5069-1) latches off.
    UnitHotSwapLatch,
    /// The slot's overvoltage latch is set.
    OvLatch,
    /// The slot's hot-swap drops PGD for one cycle.
    PgdLoss,
    /// The slot's power limit trips: PWR_LIMIT is set to `value`
    /// \[0.1 W\] under the load, which pulls OV_KILL.
    PowerLimitTrip,
    /// The host behind the slot stops (`on`) or resumes (`on = false`)
    /// answering its PWR button.
    IgnorePwr,
    /// The slot's load becomes `value` \[mA\].
    Load,
}

/// One fault and when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaneFault {
    /// Simulated time at which it fires \[s\].
    pub at_s: u64,
    /// What happens.
    pub kind: FaultKind,
    /// Slot 0-7, for a slot fault.
    pub slot: u8,
    /// Unit 0-3, for a unit fault.
    pub unit: u8,
    /// How many transactions to drop or corrupt.
    pub count: u32,
    /// The new load \[mA\] for `load`, or the power limit \[0.1 W\] for
    /// `power_limit_trip`.
    pub value: u32,
    /// Set (true) or clear (false) the behaviour; `ignore_pwr` only.
    pub on: bool,
}

impl Default for PlaneFault {
    fn default() -> Self {
        PlaneFault {
            at_s: 0,
            kind: FaultKind::default(),
            slot: 0,
            unit: 0,
            count: 1,
            value: 0,
            on: true,
        }
    }
}

/// One simulated plane on the link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaneScenario {
    /// Its I2C address, 0x30-0x37 (the straps).
    pub address: u8,
    /// Which chain channels carry a unit board, 0-3.
    pub units: Vec<u8>,
    /// Blink detection window \[ms\], shortest and longest period. Left
    /// out, the plane keeps its own default (200 / 4000), and every wait
    /// on a host LED is then at least five seconds long.
    pub blink_window_ms: Option<[u16; 2]>,
    /// Per-slot hardware; a slot left out gets the defaults.
    #[serde(rename = "slot")]
    pub slots: Vec<PlaneSlotScenario>,
    /// Faults to inject, with the time they fire.
    #[serde(rename = "fault")]
    pub faults: Vec<PlaneFault>,
}

impl Default for PlaneScenario {
    fn default() -> Self {
        PlaneScenario {
            address: granite_proto::ADDR_BASE,
            units: (0..granite_proto::UNIT_COUNT as u8).collect(),
            blink_window_ms: None,
            slots: Vec::new(),
            faults: Vec::new(),
        }
    }
}

impl PlaneScenario {
    /// The scenario for one slot, defaults filled in.
    pub fn slot(&self, slot: u8) -> PlaneSlotScenario {
        self.slots
            .iter()
            .find(|s| s.slot == slot)
            .cloned()
            .unwrap_or(PlaneSlotScenario {
                slot,
                ..Default::default()
            })
    }

    /// True while the unit on chain channel `unit` is populated.
    pub fn has_unit(&self, unit: u8) -> bool {
        self.units.contains(&unit)
    }

    /// Drop what cannot be on a plane and sort what is left.
    fn normalise(&mut self) {
        self.units
            .retain(|u| (*u as usize) < granite_proto::UNIT_COUNT);
        self.units.sort_unstable();
        self.units.dedup();
        self.slots
            .retain(|s| (s.slot as usize) < granite_proto::SLOT_COUNT);
        self.slots.sort_by_key(|s| s.slot);
        self.slots.dedup_by_key(|s| s.slot);
        self.faults.sort_by_key(|f| f.at_s);
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
    /// Planes on the link, `[[plane]]` in the file.
    #[serde(rename = "plane")]
    pub planes: Vec<PlaneScenario>,
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
            planes: vec![PlaneScenario::default()],
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
        // Planes are keyed by their address: anything off the strap
        // range cannot exist, and two planes on one address is the
        // wiring mistake ADR 0003 says software cannot resolve.
        self.planes
            .retain(|p| granite_proto::plane_of_address(p.address).is_some());
        self.planes.sort_by_key(|p| p.address);
        self.planes.dedup_by_key(|p| p.address);
        for plane in self.planes.iter_mut() {
            plane.normalise();
        }
    }

    /// The scenario for the plane at `address`, if the file has one.
    pub fn plane(&self, address: u8) -> Option<&PlaneScenario> {
        self.planes.iter().find(|p| p.address == address)
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
