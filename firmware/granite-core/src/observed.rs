//! The one sensor snapshot. Publishers, the rule engine and the API read
//! this; none of them ever touch the hardware or the plane link directly
//! (ADR 0001 component 4, ADR 0004 decision 7).
//!
//! The node, unit and plane parts are copied out of the plane client's
//! models once per cycle ([`Observed::refresh_planes`]), so every reader
//! sees the same picture for a whole tick and nothing has to borrow the
//! client while it is polling. The probes, the board temperature, VIN and
//! the dry inputs are still read by the firmware's own sense loop.
//!
//! Every field carries the monotonic millisecond timestamp of the reading
//! that produced it, so a consumer can tell a fresh zero from a stale one.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use crate::hal::RomId;
use crate::node::LastJob;
use crate::plane::model::{HostState, PowerState};
use crate::plane::{Node, Plane, PlaneClient, Unit};
use crate::{NodeId, UNIT_COUNT};

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

/// Joules in a watt-hour: the step the published energy figure is
/// rounded to (see [`NodeTelemetryObs::energy_wh`]).
pub const JOULES_PER_WH: u64 = 3_600;

/// What a node measures, in the plane's own units.
///
/// The raw wire scales are kept (10 mV, mA, 0.1 W, 0.1 C) so a rule can
/// compare without a conversion; the rounded accessors are what the
/// publishers use, because a payload that carries every milliamp would
/// make "publish on change" fire on noise (ADR 0004 decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeTelemetryObs {
    /// Voltage \[10 mV\].
    pub volts: u16,
    /// Current \[mA\].
    pub amps: u16,
    /// Power \[0.1 W\].
    pub watts: u16,
    /// Power minimum since the last min / max clear \[0.1 W\].
    pub watts_min: u16,
    /// Power maximum since the last min / max clear \[0.1 W\].
    pub watts_max: u16,
    /// INA238 die temperature \[0.1 C\].
    pub temperature: i16,
    /// Energy since the plane's first boot \[J\].
    pub energy_joules: u64,
    /// The plane boot count the energy figure belongs to.
    pub energy_boot_count: u32,
    /// The plane's module class code.
    pub module_class: u8,
    /// The power limit in force \[0.1 W\], 0 for none.
    pub power_limit: u16,
    /// Hot-swap retries inside the policy window.
    pub retry_count: u16,
}

impl NodeTelemetryObs {
    /// Voltage rounded to 0.1 V.
    pub fn volts_tenths(&self) -> u16 {
        div_round(u32::from(self.volts), 10) as u16
    }

    /// Current rounded to 0.1 A.
    pub fn amps_tenths(&self) -> u16 {
        div_round(u32::from(self.amps), 100) as u16
    }

    /// Power rounded to whole watts.
    pub fn watts_whole(&self) -> u16 {
        div_round(u32::from(self.watts), 10) as u16
    }

    /// Temperature rounded to whole degrees Celsius.
    pub fn temp_whole_c(&self) -> i16 {
        div_round_signed(i32::from(self.temperature), 10) as i16
    }

    /// Energy rounded to whole watt-hours: at any sane load this moves
    /// slowly enough for an on-change publisher.
    pub const fn energy_wh(&self) -> u64 {
        self.energy_joules / JOULES_PER_WH
    }
}

/// Round a non-negative division.
const fn div_round(value: u32, by: u32) -> u32 {
    (value + by / 2) / by
}

/// Round a division that may be negative, away from zero at the half.
const fn div_round_signed(value: i32, by: i32) -> i32 {
    if value >= 0 {
        (value + by / 2) / by
    } else {
        -((-value + by / 2) / by)
    }
}

/// Why a node is in [`PowerState::Fault`], or otherwise not usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeFaultsObs {
    /// No module in the slot, or no unit board on that half of the chain.
    pub absent: bool,
    /// The slot's overvoltage latch is set.
    pub ov_latched: bool,
    /// The slot's power limit tripped.
    pub power_limit: bool,
    /// The plane gave up retrying the hot-swap.
    pub retry_limit: bool,
    /// The unit's own hot-swap latched off: `unit_reset` re-arms it.
    pub unit_latched_off: bool,
    /// The plane's LED-against-rail cross-check disagrees.
    pub cross_check: bool,
}

impl NodeFaultsObs {
    /// True when anything is wrong.
    pub const fn any(&self) -> bool {
        self.absent
            || self.ov_latched
            || self.power_limit
            || self.retry_limit
            || self.unit_latched_off
            || self.cross_check
    }

    /// The bitmap the Modbus map publishes.
    pub const fn bits(&self) -> u16 {
        (self.absent as u16)
            | (self.ov_latched as u16) << 1
            | (self.power_limit as u16) << 2
            | (self.retry_limit as u16) << 3
            | (self.unit_latched_off as u16) << 4
            | (self.cross_check as u16) << 5
    }
}

/// What is known about one node: a slot on a plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeObs {
    /// Node id 1..=64.
    pub node: NodeId,
    /// Plane number 0..=7.
    pub plane: u8,
    /// The plane's I2C address.
    pub address: u8,
    /// Slot index on the plane, 0..=7.
    pub slot: u8,
    /// Unit board the slot sits on, 0..=3.
    pub unit: u8,
    /// The rail (ADR 0004 decision 3).
    pub power: PowerState,
    /// The host, as the plane reads its LED.
    pub host: HostState,
    /// True when the slot holds a module on a unit that is on the chain.
    pub present: bool,
    /// True while the plane runs a sequence on this slot.
    pub busy: bool,
    /// Why it is in fault, when it is.
    pub faults: NodeFaultsObs,
    /// Everything measured.
    pub telemetry: NodeTelemetryObs,
    /// The last job the controller gave this node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_job: Option<LastJob>,
    /// When the plane last reported this slot \[ms\].
    pub ts_ms: u64,
}

impl NodeObs {
    /// An unknown node at a place on the link.
    pub fn unknown(node: NodeId, address: u8, slot: u8) -> Self {
        NodeObs {
            node,
            plane: address.saturating_sub(granite_proto::ADDR_BASE),
            address,
            slot,
            unit: slot / 2,
            power: PowerState::Unknown,
            host: HostState::Unknown,
            present: false,
            busy: false,
            faults: NodeFaultsObs::default(),
            telemetry: NodeTelemetryObs::default(),
            last_job: None,
            ts_ms: 0,
        }
    }

    /// Copy one of the plane client's node models.
    pub fn of_node(node: &Node, unit: &Unit) -> Self {
        use granite_proto::SlotStatus;
        let status = node.status;
        NodeObs {
            node: node.id,
            plane: node.address.saturating_sub(granite_proto::ADDR_BASE),
            address: node.address,
            slot: node.slot,
            unit: node.unit(),
            power: node.power,
            host: node.host,
            present: node.is_present(),
            busy: node.is_busy(),
            faults: NodeFaultsObs {
                absent: !node.is_present(),
                ov_latched: status.contains(SlotStatus::OV_LATCHED),
                power_limit: status.contains(SlotStatus::POWER_LIMIT_TRIPPED),
                retry_limit: node.retry_limit,
                unit_latched_off: unit.is_latched_off(),
                cross_check: !node.telemetry.cross.is_empty(),
            },
            telemetry: NodeTelemetryObs {
                volts: node.telemetry.volts,
                amps: node.telemetry.amps,
                watts: node.telemetry.watts,
                watts_min: node.telemetry.watts_min,
                watts_max: node.telemetry.watts_max,
                temperature: node.telemetry.temperature,
                energy_joules: node.telemetry.energy_joules,
                energy_boot_count: node.telemetry.energy_boot_count,
                module_class: node.telemetry.module_class,
                power_limit: node.telemetry.power_limit,
                retry_count: node.telemetry.retry_count,
            },
            last_job: None,
            ts_ms: node.summary_ms.max(node.detail_ms),
        }
    }

    /// True when the rail carries the slot.
    pub const fn is_on(&self) -> bool {
        matches!(self.power, PowerState::On)
    }
}

/// What is known about one unit board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UnitObs {
    /// Unit index 0..=3.
    pub unit: u8,
    /// True when the unit answers on the chain.
    pub present: bool,
    /// True when its own hot-swap latched off.
    pub latched_off: bool,
    /// Input voltage \[10 mV\].
    pub volts: u16,
    /// Input current \[mA\].
    pub amps: u16,
    /// Input power \[0.1 W\].
    pub watts: u16,
    /// Input INA238 die temperature \[0.1 C\].
    pub temperature: i16,
    /// I2C errors on this unit's segment.
    pub segment_errors: u16,
    /// How often the unit stopped answering.
    pub times_lost: u16,
    /// When the plane last reported it \[ms\].
    pub ts_ms: u64,
}

impl UnitObs {
    /// Copy one of the plane client's unit models.
    pub fn of_unit(unit: &Unit) -> Self {
        UnitObs {
            unit: unit.index,
            present: unit.is_present(),
            latched_off: unit.is_latched_off(),
            volts: unit.volts,
            amps: unit.amps,
            watts: unit.watts,
            temperature: unit.temperature,
            segment_errors: unit.segment_errors,
            times_lost: unit.times_lost,
            ts_ms: unit.updated_ms,
        }
    }

    /// Input current rounded to 0.1 A.
    pub fn amps_tenths(&self) -> u16 {
        div_round(u32::from(self.amps), 100) as u16
    }

    /// Input power rounded to whole watts.
    pub fn watts_whole(&self) -> u16 {
        div_round(u32::from(self.watts), 10) as u16
    }
}

/// What is known about one plane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaneObs {
    /// Plane number 0..=7.
    pub plane: u8,
    /// Its I2C address.
    pub address: u8,
    /// What kind of plane it says it is, empty until INFO was read.
    pub plane_type: String,
    /// Firmware version, `major.minor.patch`.
    pub firmware: String,
    /// Bootloader version.
    pub bootloader: String,
    /// Serial: a hash of the MCU UID.
    pub serial: u32,
    /// Core input voltage \[10 mV\].
    pub bus_volts: u16,
    /// Core (MCU) temperature \[0.1 C\].
    pub core_temperature: i16,
    /// Boot count.
    pub boot_count: u32,
    /// Seconds since the plane booted.
    pub uptime_s: u32,
    /// The hash of the config set the plane holds.
    pub config_hash: u32,
    /// What the plane says about its running image.
    pub image_state: String,
    /// True while a sequence runs on any of its slots.
    pub sequence_running: bool,
    /// True while it holds unacknowledged events.
    pub events_pending: bool,
    /// True while it says its intended state is lost.
    pub intended_state_lost: bool,
    /// Bitmap of the units INFO says are on the chain.
    pub units_present: u8,
    /// Bitmap of the slots that hold a module.
    pub nodes_present: u8,
    /// The unit boards.
    pub units: Vec<UnitObs>,
    /// When INFO last arrived \[ms\].
    pub info_ms: u64,
    /// When STATUS last arrived \[ms\].
    pub ts_ms: u64,
}

impl PlaneObs {
    /// Copy one of the plane client's plane models.
    pub fn of_plane(plane: &Plane) -> Self {
        let mut nodes_present = 0u8;
        for node in plane.nodes.iter() {
            if node.is_present() {
                nodes_present |= 1 << node.slot;
            }
        }
        PlaneObs {
            plane: plane.number,
            address: plane.address,
            plane_type: plane
                .plane_type
                .map(|t| String::from(plane_type_name(t)))
                .unwrap_or_default(),
            firmware: version_string(plane.firmware),
            bootloader: version_string(plane.bootloader),
            serial: plane.serial,
            bus_volts: plane.core_volts,
            core_temperature: plane.core_temperature,
            boot_count: plane.boot_count,
            uptime_s: plane.uptime_s,
            config_hash: plane.config_hash,
            image_state: String::from(image_state_name(plane.image_state)),
            sequence_running: plane.sequence_running(),
            events_pending: plane.events_pending(),
            intended_state_lost: plane.intended_state_lost(),
            units_present: plane.units_present.bits(),
            nodes_present,
            units: plane.units.iter().map(UnitObs::of_unit).collect(),
            info_ms: plane.info_ms,
            ts_ms: plane.status_ms,
        }
    }

    /// One unit, if it is an index.
    pub fn unit(&self, unit: u8) -> Option<&UnitObs> {
        self.units.iter().find(|u| u.unit == unit)
    }

    /// Core input voltage rounded to 0.1 V.
    pub fn bus_volts_tenths(&self) -> u16 {
        div_round(u32::from(self.bus_volts), 10) as u16
    }
}

/// `major.minor.patch` of a plane version record.
pub fn version_string(v: granite_proto::Version) -> String {
    alloc::format!("{}.{}.{}", v.major, v.minor, v.patch)
}

/// Lowercase name of a plane type (INFO 0x02). The wire enums carry no
/// names of their own, so the controller gives them one.
pub const fn plane_type_name(t: granite_proto::PlaneType) -> &'static str {
    match t {
        granite_proto::PlaneType::Pp48Rev1 => "pp48_rev1",
        granite_proto::PlaneType::Pp48Chained => "pp48_chained",
    }
}

/// Lowercase name of an image state (INFO 0x0A).
pub const fn image_state_name(s: granite_proto::ImageState) -> &'static str {
    match s {
        granite_proto::ImageState::Confirmed => "confirmed",
        granite_proto::ImageState::Probation => "probation",
        granite_proto::ImageState::RolledBack => "rolled_back",
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
    /// Every node of every answering plane whose slot holds a module, in
    /// id order. Nodes that are not present are not listed (ADR 0004
    /// decision 2); [`Observed::node`] still answers for them.
    pub nodes: Vec<NodeObs>,
    /// Every plane that answers, lowest address first.
    pub planes: Vec<PlaneObs>,
    /// The addresses of the planes the controller had found and that
    /// stopped answering ([`crate::plane::PlaneClient::lost_addresses`]):
    /// what Modbus fault bit 0 reports. Empty on a controller that has
    /// not lost one.
    #[serde(default)]
    pub planes_lost: Vec<u8>,
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
            nodes: Vec::new(),
            planes: Vec::new(),
            planes_lost: Vec::new(),
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

    /// Copy the plane client's models into the snapshot.
    ///
    /// The last job of each node survives the refresh: it is the
    /// controller's own bookkeeping ([`crate::jobs`]), not something the
    /// plane reports.
    pub fn refresh_planes(&mut self, client: &PlaneClient) {
        let mut nodes: Vec<NodeObs> = Vec::new();
        let mut planes: Vec<PlaneObs> = Vec::new();
        for plane in client.planes() {
            for node in plane.present_nodes() {
                let unit = &plane.units[(node.unit() as usize).min(UNIT_COUNT - 1)];
                let mut obs = NodeObs::of_node(node, unit);
                obs.last_job = self
                    .nodes
                    .iter()
                    .find(|n| n.node == obs.node)
                    .and_then(|n| n.last_job.clone());
                nodes.push(obs);
            }
            planes.push(PlaneObs::of_plane(plane));
        }
        nodes.sort_unstable_by_key(|n| n.node);
        self.nodes = nodes;
        self.planes = planes;
        self.planes_lost = client.lost_addresses();
    }

    /// True while a plane the controller had found is not answering.
    pub fn any_plane_lost(&self) -> bool {
        !self.planes_lost.is_empty()
    }

    /// One node, if its plane answers and its slot holds a module.
    pub fn node(&self, node: NodeId) -> Option<&NodeObs> {
        self.nodes.iter().find(|n| n.node == node)
    }

    /// One node, mutably: how [`crate::jobs`] records a last job.
    pub fn node_mut(&mut self, node: NodeId) -> Option<&mut NodeObs> {
        self.nodes.iter_mut().find(|n| n.node == node)
    }

    /// The rail state of a node; `unknown` when nothing is known.
    pub fn node_power(&self, node: NodeId) -> PowerState {
        self.node(node).map_or(PowerState::Unknown, |n| n.power)
    }

    /// The host state of a node; `unknown` when nothing is known.
    pub fn node_host(&self, node: NodeId) -> HostState {
        self.node(node).map_or(HostState::Unknown, |n| n.host)
    }

    /// Every present node's id, in order: the set `all` acts on.
    pub fn present_nodes(&self) -> Vec<NodeId> {
        self.nodes
            .iter()
            .filter(|n| n.present)
            .map(|n| n.node)
            .collect()
    }

    /// One plane by number, if it answers.
    pub fn plane(&self, plane: u8) -> Option<&PlaneObs> {
        self.planes.iter().find(|p| p.plane == plane)
    }

    /// One plane by address, if it answers.
    pub fn plane_at(&self, address: u8) -> Option<&PlaneObs> {
        self.planes.iter().find(|p| p.address == address)
    }

    /// One unit, if its plane answers.
    pub fn unit(&self, plane: u8, unit: u8) -> Option<&UnitObs> {
        self.plane(plane)?.unit(unit)
    }

    /// Probe in slot `slot` (0-based), if configured.
    pub fn probe(&self, slot: usize) -> Option<&ProbeObs> {
        self.probes.get(slot)
    }

    /// Highest probe reading in centi-degrees, `None` when no probe answered.
    pub fn probe_max(&self) -> Option<i16> {
        self.probes.iter().filter_map(|p| p.centi_c).max()
    }

    /// Dry input `n` (1-based), `None` when the inputs were never read.
    pub fn dry(&self, n: u8) -> Option<bool> {
        if n == 0 || n as usize > crate::DRY_COUNT {
            return None;
        }
        self.dry_in.value.map(|bits| bits & (1 << (n - 1)) != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telemetry_rounds_for_the_publishers() {
        let t = NodeTelemetryObs {
            // 48.04 V, 1.004 A, 48.2 W, 31.4 C, 7260 J.
            volts: 4_804,
            amps: 1_004,
            watts: 482,
            temperature: 314,
            energy_joules: 7_260,
            ..NodeTelemetryObs::default()
        };
        assert_eq!(t.volts_tenths(), 480, "48.0 V");
        assert_eq!(t.amps_tenths(), 10, "1.0 A");
        assert_eq!(t.watts_whole(), 48);
        assert_eq!(t.temp_whole_c(), 31);
        assert_eq!(t.energy_wh(), 2);

        let cold = NodeTelemetryObs {
            temperature: -156,
            ..NodeTelemetryObs::default()
        };
        assert_eq!(cold.temp_whole_c(), -16);
    }

    #[test]
    fn fault_bits_are_stable() {
        let faults = NodeFaultsObs {
            absent: true,
            unit_latched_off: true,
            ..NodeFaultsObs::default()
        };
        assert_eq!(faults.bits(), 0b1_0001);
        assert!(faults.any());
        assert!(!NodeFaultsObs::default().any());
    }
}
