//! The models the client owns: [`Plane`], [`Unit`] and [`Node`].
//!
//! A node is a slot on a plane (ADR 0004 decision 2) and its state has
//! two axes (decision 3): the rail, as the plane's slot status word
//! reports it, and the host, as the plane reads its power LED. The
//! telemetry travels with them.
//!
//! Nothing here talks to the link. The client hands these structs the
//! payload of a block it read and they decode it with `granite_proto`;
//! a record the plane did not send leaves the field as it was, which is
//! what a TLV block asks for (a reader never assumes order and never
//! assumes presence).

use granite_proto::{
    CrossCheck, Energy, ImageState, IndexBitmap, Info, LedMode, LinkErrors, NodeLed, PlaneType,
    PowerMinMax, SLOT_COUNT, SegmentErrors, SequenceState, Slot, SlotStatus, SlotSummary, Status,
    StatusFlags, UNIT_COUNT, Unit as UnitRecord, UnitStatus, UpdateProgress, UpdateState, Version,
};

use crate::NodeId;

use super::node_id;

/// The rail of a node, the power axis of ADR 0004 decision 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerState {
    /// OFF is asserted: the rail is down because the controller, the
    /// plane or a fault put it down.
    Off,
    /// PGD: the hot-swap carries the slot.
    On,
    /// Enabled and waiting for PGD.
    Starting,
    /// OV latched, power limit tripped, retry limit reached, or the
    /// unit's own hot-swap latched off.
    Fault,
    /// No module in the slot, or no unit board on that half of the
    /// chain.
    Absent,
    /// The plane is not answering (or has not answered yet).
    #[default]
    Unknown,
}

impl PowerState {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            PowerState::Off => "off",
            PowerState::On => "on",
            PowerState::Starting => "starting",
            PowerState::Fault => "fault",
            PowerState::Absent => "absent",
            PowerState::Unknown => "unknown",
        }
    }

    /// True when the node exists as far as the plane can tell.
    pub const fn is_present(self) -> bool {
        !matches!(self, PowerState::Absent | PowerState::Unknown)
    }
}

impl core::fmt::Display for PowerState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The motherboard, as the plane reads its power LED: the host axis of
/// ADR 0004 decision 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostState {
    /// The LED is dark.
    Off,
    /// The LED is steady.
    On,
    /// The LED blinks: the host sleeps.
    Sleep,
    /// The rail is off, the LED is not trusted, or there is no data.
    #[default]
    Unknown,
}

impl HostState {
    /// Lowercase wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            HostState::Off => "off",
            HostState::On => "on",
            HostState::Sleep => "sleep",
            HostState::Unknown => "unknown",
        }
    }
}

impl core::fmt::Display for HostState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a slot reports beyond its two states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeTelemetry {
    /// Voltage \[10 mV\].
    pub volts: u16,
    /// Current \[mA\].
    pub amps: u16,
    /// Power \[0.1 W\].
    pub watts: u16,
    /// Power minimum since the plane's last min / max read \[0.1 W\].
    pub watts_min: u16,
    /// Power maximum since the plane's last min / max read \[0.1 W\].
    pub watts_max: u16,
    /// INA238 die temperature \[0.1 C\].
    pub temperature: i16,
    /// Energy since the plane's first boot \[J\].
    pub energy_joules: u64,
    /// The plane boot count the energy figure belongs to: a reader that
    /// sees it change knows the counter restarted.
    pub energy_boot_count: u32,
    /// The plane's module class code.
    pub module_class: u8,
    /// The raw module ID reading.
    pub module_id_raw: u16,
    /// Hot-swap retries inside the policy window.
    pub retry_count: u16,
    /// The power limit in force \[0.1 W\], 0 for none.
    pub power_limit: u16,
    /// Blink period of the host LED \[ms\], 0 unless blinking.
    pub led_period_ms: u16,
    /// LED-against-rail cross-checks.
    pub cross: CrossCheck,
    /// The slot's sequence state.
    pub sequence: SequenceState,
}

impl Default for NodeTelemetry {
    /// Nothing measured yet. The wire enums have no `Default` of their
    /// own (a wire value never means "unset"), so this spells them out.
    fn default() -> Self {
        NodeTelemetry {
            volts: 0,
            amps: 0,
            watts: 0,
            watts_min: 0,
            watts_max: 0,
            temperature: 0,
            energy_joules: 0,
            energy_boot_count: 0,
            module_class: 0,
            module_id_raw: 0,
            retry_count: 0,
            power_limit: 0,
            led_period_ms: 0,
            cross: CrossCheck::NONE,
            sequence: SequenceState::Idle,
        }
    }
}

impl NodeTelemetry {
    /// Power in watts, rounded down: what a reader that does not want
    /// deciwatts wants.
    pub const fn watts_whole(&self) -> u16 {
        self.watts / 10
    }
}

/// One node: a slot on a plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Node {
    /// Node id 1-64 (ADR 0004 decision 2).
    pub id: NodeId,
    /// The plane's I2C address.
    pub address: u8,
    /// Slot index on the wire, `2 * unit + (A = 0, B = 1)`.
    pub slot: u8,
    /// The rail.
    pub power: PowerState,
    /// The host.
    pub host: HostState,
    /// The slot status word as it came off the wire.
    pub status: SlotStatus,
    /// How the plane reads the host's LED.
    pub led: LedMode,
    /// Everything measured.
    pub telemetry: NodeTelemetry,
    /// When the 1 Hz summary for this slot last arrived \[ms\].
    pub summary_ms: u64,
    /// When the full SLOT block for this slot last arrived \[ms\].
    pub detail_ms: u64,
    /// The plane reported the retry limit for this slot and set it OFF
    /// (EVENTS kind 6). Sticky until the slot is enabled again, because
    /// no status bit carries it.
    pub retry_limit: bool,
}

impl Node {
    /// An unknown node at a place on the link.
    pub fn new(address: u8, slot: u8) -> Self {
        Node {
            id: node_id(address, slot).unwrap_or(0),
            address,
            slot,
            power: PowerState::Unknown,
            host: HostState::Unknown,
            status: SlotStatus::NONE,
            led: LedMode::Off,
            telemetry: NodeTelemetry::default(),
            summary_ms: 0,
            detail_ms: 0,
            retry_limit: false,
        }
    }

    /// The unit board this slot sits on (0-3).
    pub const fn unit(&self) -> u8 {
        self.slot / 2
    }

    /// True for a node that exists and should appear in a state
    /// payload: its unit is on the chain and a module answered.
    pub const fn is_present(&self) -> bool {
        self.power.is_present()
    }

    /// True while the plane runs a sequence on this slot.
    pub const fn is_busy(&self) -> bool {
        self.status.contains(SlotStatus::SEQUENCE_RUNNING)
    }

    /// Everything the client cannot know any more once its plane stops
    /// answering.
    fn forget(&mut self) {
        self.power = PowerState::Unknown;
        self.host = HostState::Unknown;
        self.status = SlotStatus::NONE;
    }

    /// Recompute both axes from the status word, the LED and whether
    /// the unit is there.
    fn derive(&mut self, unit_present: bool, unit_latched_off: bool) {
        self.power = if !unit_present || !self.status.contains(SlotStatus::PRESENT) {
            PowerState::Absent
        } else if unit_latched_off
            || self.retry_limit
            || self
                .status
                .intersects(SlotStatus::OV_LATCHED | SlotStatus::POWER_LIMIT_TRIPPED)
        {
            PowerState::Fault
        } else if !self.status.contains(SlotStatus::ENABLED) {
            PowerState::Off
        } else if self.status.contains(SlotStatus::PGD) {
            PowerState::On
        } else {
            PowerState::Starting
        };
        // The ADR is explicit: with the rail down the LED says nothing,
        // so the host axis is `unknown` rather than `off`.
        self.host = match (self.power, self.led) {
            (PowerState::On, LedMode::Off) => HostState::Off,
            (PowerState::On, LedMode::On) => HostState::On,
            (PowerState::On, LedMode::Blinking) => HostState::Sleep,
            _ => HostState::Unknown,
        };
    }

    /// Take a SUMMARY record.
    fn apply_summary(&mut self, now_ms: u64, record: &SlotSummary) {
        self.status = record.status;
        self.led = record.led;
        self.telemetry.volts = record.volts;
        self.telemetry.amps = record.amps;
        self.telemetry.watts = record.watts;
        self.summary_ms = now_ms;
        if record.status.contains(SlotStatus::ENABLED) {
            self.retry_limit = false;
        }
    }

    /// Take the records of a SLOT block.
    fn apply_slot(&mut self, now_ms: u64, payload: &[u8]) {
        for record in Slot::records(payload).flatten() {
            match record {
                Slot::Status(s) => {
                    self.status = s;
                    if s.contains(SlotStatus::ENABLED) {
                        self.retry_limit = false;
                    }
                }
                Slot::RetryCount(n) => self.telemetry.retry_count = n,
                Slot::ModuleClass(c) => self.telemetry.module_class = c,
                Slot::ModuleIdRaw(v) => self.telemetry.module_id_raw = v,
                Slot::Volts(v) => self.telemetry.volts = v,
                Slot::Amps(v) => self.telemetry.amps = v,
                Slot::Watts(v) => self.telemetry.watts = v,
                Slot::PowerMinMax(PowerMinMax { min, max }) => {
                    self.telemetry.watts_min = min;
                    self.telemetry.watts_max = max;
                }
                Slot::Temperature(t) => self.telemetry.temperature = t,
                Slot::Energy(Energy { joules, boot_count }) => {
                    self.telemetry.energy_joules = joules;
                    self.telemetry.energy_boot_count = boot_count;
                }
                Slot::NodeLed(NodeLed { mode, period_ms }) => {
                    self.led = mode;
                    self.telemetry.led_period_ms = period_ms;
                }
                Slot::CrossCheck(c) => self.telemetry.cross = c,
                Slot::SequenceState(s) => self.telemetry.sequence = s,
                Slot::PowerLimit(p) => self.telemetry.power_limit = p,
            }
        }
        self.detail_ms = now_ms;
    }
}

/// One unit board of a chained plane (ADR 0005), index 0-3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unit {
    /// Unit index 0-3.
    pub index: u8,
    /// The unit status word as it came off the wire.
    pub status: UnitStatus,
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
    /// When the UNIT block for it last arrived \[ms\].
    pub updated_ms: u64,
}

impl Unit {
    /// An unknown unit at an index.
    pub const fn new(index: u8) -> Self {
        Unit {
            index,
            status: UnitStatus::NONE,
            volts: 0,
            amps: 0,
            watts: 0,
            temperature: 0,
            segment_errors: 0,
            times_lost: 0,
            updated_ms: 0,
        }
    }

    /// True when the unit answers on the chain.
    pub const fn is_present(&self) -> bool {
        self.status.contains(UnitStatus::PRESENT)
    }

    /// True when the unit's own hot-swap latched off: every node on it
    /// is in fault until [`super::PlaneCommand::UnitReset`].
    pub const fn is_latched_off(&self) -> bool {
        self.status.contains(UnitStatus::HOT_SWAP_LATCHED_OFF)
    }

    /// Take the records of a UNIT block.
    fn apply(&mut self, now_ms: u64, payload: &[u8]) {
        for record in UnitRecord::records(payload).flatten() {
            match record {
                UnitRecord::Status(s) => self.status = s,
                UnitRecord::InputVolts(v) => self.volts = v,
                UnitRecord::InputAmps(v) => self.amps = v,
                UnitRecord::InputWatts(v) => self.watts = v,
                UnitRecord::InputTemperature(t) => self.temperature = t,
                UnitRecord::SegmentErrors(n) => self.segment_errors = n,
                UnitRecord::TimesLost(n) => self.times_lost = n,
            }
        }
        self.updated_ms = now_ms;
    }
}

/// What an INFO read told the client that it did not know before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InfoChange {
    /// The boot count changed: the plane rebooted (or this is the first
    /// INFO of a plane the client has just found).
    pub rebooted: bool,
    /// The config hash changed.
    pub config_hash: bool,
    /// The image state changed.
    pub image_state: bool,
    /// The units-present bitmap changed.
    pub units: bool,
}

impl InfoChange {
    /// True when anything at all changed.
    pub const fn any(&self) -> bool {
        self.rebooted || self.config_hash || self.image_state || self.units
    }
}

/// One plane and everything on it.
#[derive(Debug, Clone)]
pub struct Plane {
    /// I2C address, 0x30-0x37.
    pub address: u8,
    /// Plane number, 0-7 (the address minus 0x30).
    pub number: u8,
    /// Protocol format version the plane speaks (INFO 0x01).
    pub format_version: u8,
    /// What kind of plane it is, once INFO has been read.
    pub plane_type: Option<PlaneType>,
    /// Slots it has (8 on the chained plane).
    pub slot_capacity: u8,
    /// Firmware version.
    pub firmware: Version,
    /// Bootloader version.
    pub bootloader: Version,
    /// Serial: a hash of the STM32 UID.
    pub serial: u32,
    /// What the plane can do.
    pub capabilities: granite_proto::Capabilities,
    /// Units present, bit n = unit n.
    pub units_present: IndexBitmap,
    /// Boot count.
    pub boot_count: u32,
    /// The hash of the config set the plane has stored.
    pub config_hash: u32,
    /// The state of the running image.
    pub image_state: ImageState,
    /// The plane flags of the last STATUS read.
    pub flags: StatusFlags,
    /// Core input voltage \[10 mV\].
    pub core_volts: u16,
    /// Core (MCU) temperature \[0.1 C\].
    pub core_temperature: i16,
    /// Uptime \[s\].
    pub uptime_s: u32,
    /// The next event sequence number the plane will use.
    pub event_head: u16,
    /// What the plane is doing with a firmware image.
    pub update: UpdateProgress,
    /// Frames the plane dropped on the link (its own counters).
    pub link_errors: LinkErrors,
    /// I2C errors on the plane's segment bus, per channel.
    pub segment_errors: SegmentErrors,
    /// The plane's own reset cause code.
    pub last_reset_cause: u8,
    /// The four unit boards.
    pub units: [Unit; UNIT_COUNT],
    /// The eight slots, as nodes.
    pub nodes: [Node; SLOT_COUNT],
    /// When INFO last arrived \[ms\].
    pub info_ms: u64,
    /// When STATUS last arrived \[ms\].
    pub status_ms: u64,
}

impl Plane {
    /// A plane the client has just found, with nothing read yet.
    pub fn new(address: u8) -> Self {
        Plane {
            address,
            number: address.saturating_sub(granite_proto::ADDR_BASE),
            format_version: 0,
            plane_type: None,
            slot_capacity: 0,
            firmware: Version::new(0, 0, 0),
            bootloader: Version::new(0, 0, 0),
            serial: 0,
            capabilities: granite_proto::Capabilities::NONE,
            units_present: IndexBitmap::NONE,
            boot_count: 0,
            config_hash: 0,
            image_state: ImageState::Confirmed,
            flags: StatusFlags::NONE,
            core_volts: 0,
            core_temperature: 0,
            uptime_s: 0,
            event_head: 0,
            update: UpdateProgress::new(UpdateState::Idle, 0),
            link_errors: LinkErrors::default(),
            segment_errors: SegmentErrors::default(),
            last_reset_cause: 0,
            units: core::array::from_fn(|i| Unit::new(i as u8)),
            nodes: core::array::from_fn(|i| Node::new(address, i as u8)),
            info_ms: 0,
            status_ms: 0,
        }
    }

    /// The node in `slot`, if that is a slot.
    pub fn node(&self, slot: u8) -> Option<&Node> {
        self.nodes.get(slot as usize)
    }

    /// Every node whose unit is on the chain and whose slot holds a
    /// module.
    pub fn present_nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter(|n| n.is_present())
    }

    /// Every unit on the chain.
    pub fn present_units(&self) -> impl Iterator<Item = &Unit> {
        self.units.iter().filter(|u| u.is_present())
    }

    /// True while the plane runs a sequence on any slot.
    pub const fn sequence_running(&self) -> bool {
        self.flags.contains(StatusFlags::SEQUENCE_RUNNING)
    }

    /// True while the plane holds unacknowledged events.
    pub const fn events_pending(&self) -> bool {
        self.flags.contains(StatusFlags::EVENTS_PENDING)
    }

    /// True while the plane says its intended state is gone: the core
    /// lost power, OFF slots came back ON, and the controller has to
    /// put them back (ADR 0004, "What an Expert Would Ask").
    pub const fn intended_state_lost(&self) -> bool {
        self.flags.contains(StatusFlags::INTENDED_STATE_LOST)
    }

    /// Take the records of an INFO block and say what changed.
    pub fn apply_info(&mut self, now_ms: u64, payload: &[u8]) -> InfoChange {
        let before = (
            self.boot_count,
            self.config_hash,
            self.image_state,
            self.units_present.bits(),
        );
        let first = self.info_ms == 0;
        for record in Info::records(payload).flatten() {
            match record {
                Info::FormatVersion(v) => self.format_version = v,
                Info::PlaneType(t) => self.plane_type = Some(t),
                Info::SlotCapacity(n) => self.slot_capacity = n,
                Info::FirmwareVersion(v) => self.firmware = v,
                Info::Serial(s) => self.serial = s,
                Info::Capabilities(c) => self.capabilities = c,
                Info::UnitsPresent(m) => self.units_present = m,
                Info::BootCount(n) => self.boot_count = n,
                Info::ConfigHash(h) => self.config_hash = h,
                Info::ImageState(s) => self.image_state = s,
                Info::BootloaderVersion(v) => self.bootloader = v,
            }
        }
        self.info_ms = now_ms;
        InfoChange {
            rebooted: !first && self.boot_count != before.0,
            config_hash: self.config_hash != before.1,
            image_state: self.image_state != before.2,
            units: self.units_present.bits() != before.3,
        }
    }

    /// Take the records of a STATUS block.
    ///
    /// The command results travel in the same block; the client reads
    /// them straight off the payload ([`granite_proto::Status`]) rather
    /// than through the model, so nothing is copied twice.
    pub fn apply_status(&mut self, now_ms: u64, payload: &[u8]) {
        for record in Status::records(payload).flatten() {
            match record {
                Status::CoreInputVoltage(v) => self.core_volts = v,
                Status::CoreTemperature(t) => self.core_temperature = t,
                Status::Uptime(s) => self.uptime_s = s,
                Status::Flags(f) => self.flags = f,
                Status::CommandResult(_) => {}
                Status::EventHead(seq) => self.event_head = seq,
                Status::UpdateProgress(p) => self.update = p,
                Status::LinkErrors(e) => self.link_errors = e,
                Status::SegmentErrors(e) => self.segment_errors = e,
                Status::LastResetCause(c) => self.last_reset_cause = c,
            }
        }
        self.status_ms = now_ms;
    }

    /// Take a SUMMARY block: one record per slot of every present unit.
    ///
    /// Returns the slots whose status word changed, as a bitmap: those
    /// are the ones ADR 0003 decision 5 wants a full SLOT read of at
    /// once rather than at the next 10 s sweep.
    pub fn apply_summary(&mut self, now_ms: u64, payload: &[u8]) -> IndexBitmap {
        let mut changed = IndexBitmap::NONE;
        for record in SlotSummary::records(payload).flatten() {
            let Some(node) = self.nodes.get_mut(record.slot as usize) else {
                continue;
            };
            let before = (node.status, node.led);
            node.apply_summary(now_ms, &record);
            if before != (node.status, node.led) {
                changed = changed.plus(record.slot);
            }
        }
        self.derive_states();
        changed
    }

    /// Take a SLOT block for `slot`.
    pub fn apply_slot(&mut self, now_ms: u64, slot: u8, payload: &[u8]) {
        if let Some(node) = self.nodes.get_mut(slot as usize) {
            node.apply_slot(now_ms, payload);
        }
        self.derive_states();
    }

    /// Take a UNIT block for `unit`.
    pub fn apply_unit(&mut self, now_ms: u64, unit: u8, payload: &[u8]) {
        if let Some(model) = self.units.get_mut(unit as usize) {
            model.apply(now_ms, payload);
        }
        self.derive_states();
    }

    /// Mark a slot as having reached its retry limit (EVENTS kind 6):
    /// the plane set it OFF and no status bit says why.
    pub fn note_retry_limit(&mut self, slot: u8) {
        if let Some(node) = self.nodes.get_mut(slot as usize) {
            node.retry_limit = true;
        }
        self.derive_states();
    }

    /// Recompute every node's two axes.
    pub fn derive_states(&mut self) {
        for node in &mut self.nodes {
            let unit = &self.units[(node.slot / 2) as usize];
            // A unit the plane has never read is "present" as far as
            // INFO's bitmap goes: the UNIT block may not have arrived
            // yet, and an absent-looking node would flap.
            let present = if unit.updated_ms == 0 {
                self.units_present.has(node.slot / 2)
            } else {
                unit.is_present()
            };
            let latched = unit.is_latched_off();
            node.derive(present, latched);
        }
    }

    /// Everything the client cannot know any more once the plane stops
    /// answering: both axes of every node go `unknown`, the identity
    /// and the last telemetry stay for a reader to look at.
    pub fn forget_states(&mut self) {
        self.flags = StatusFlags::NONE;
        for node in &mut self.nodes {
            node.forget();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use granite_proto::{MAX_PAYLOAD, RecordWriter};

    /// A payload of records, as a block read would give it.
    fn payload<F: FnOnce(&mut RecordWriter)>(f: F) -> alloc::vec::Vec<u8> {
        let mut buf = [0u8; MAX_PAYLOAD];
        let mut w = RecordWriter::new(&mut buf);
        f(&mut w);
        w.payload().to_vec()
    }

    #[test]
    fn info_reports_what_changed() {
        let mut plane = Plane::new(0x31);
        let first = plane.apply_info(
            1_000,
            &payload(|w| {
                let _ = Info::BootCount(1).encode(w);
                let _ = Info::ConfigHash(0xAABB).encode(w);
                let _ = Info::FirmwareVersion(Version::new(0, 1, 0)).encode(w);
                let _ = Info::UnitsPresent(IndexBitmap::I0).encode(w);
            }),
        );
        assert!(!first.rebooted, "the first INFO is not a reboot");
        assert_eq!(plane.firmware, Version::new(0, 1, 0));

        let again = plane.apply_info(
            2_000,
            &payload(|w| {
                let _ = Info::BootCount(2).encode(w);
                let _ = Info::ConfigHash(0xAABB).encode(w);
            }),
        );
        assert!(again.rebooted && !again.config_hash);
        assert_eq!(plane.boot_count, 2);
    }

    #[test]
    fn the_two_axes_follow_the_status_word() {
        let mut plane = Plane::new(0x30);
        plane.units_present = IndexBitmap::I0;
        plane.apply_unit(
            0,
            0,
            &payload(|w| {
                let _ =
                    UnitRecord::Status(UnitStatus::PRESENT | UnitStatus::HOT_SWAP_PGD).encode(w);
            }),
        );

        let on = SlotStatus::PRESENT | SlotStatus::ENABLED | SlotStatus::PGD;
        let changed = plane.apply_summary(
            100,
            &payload(|w| {
                let _ = SlotSummary {
                    slot: 0,
                    status: on,
                    volts: 4_800,
                    amps: 1_000,
                    watts: 480,
                    led: LedMode::On,
                }
                .encode(w);
                let _ = SlotSummary {
                    slot: 1,
                    status: SlotStatus::PRESENT,
                    volts: 0,
                    amps: 0,
                    watts: 0,
                    led: LedMode::Off,
                }
                .encode(w);
            }),
        );
        assert_eq!(changed.bits(), 0b11, "both slots are new");
        assert_eq!(plane.nodes[0].power, PowerState::On);
        assert_eq!(plane.nodes[0].host, HostState::On);
        assert_eq!(plane.nodes[0].id, 1);
        // Rail off: the LED cannot speak for the host (ADR 0004
        // decision 3).
        assert_eq!(plane.nodes[1].power, PowerState::Off);
        assert_eq!(plane.nodes[1].host, HostState::Unknown);
        // No module, and no unit on the other half of the chain.
        assert_eq!(plane.nodes[2].power, PowerState::Absent);
        assert_eq!(plane.present_nodes().count(), 2);

        // A tripped power limit is a fault whatever the rail says.
        plane.apply_slot(
            200,
            0,
            &payload(|w| {
                let _ =
                    Slot::Status(SlotStatus::PRESENT | SlotStatus::POWER_LIMIT_TRIPPED).encode(w);
                let _ = Slot::Energy(Energy {
                    joules: 4_200,
                    boot_count: 3,
                })
                .encode(w);
            }),
        );
        assert_eq!(plane.nodes[0].power, PowerState::Fault);
        assert_eq!(plane.nodes[0].telemetry.energy_joules, 4_200);
        assert_eq!(plane.nodes[0].telemetry.energy_boot_count, 3);

        // And a plane that stops answering knows nothing about either
        // axis, while the identity stays.
        plane.forget_states();
        assert_eq!(plane.nodes[0].power, PowerState::Unknown);
        assert_eq!(plane.nodes[0].id, 1);
    }

    #[test]
    fn a_latched_unit_faults_both_its_nodes() {
        let mut plane = Plane::new(0x30);
        plane.units_present = IndexBitmap::I0;
        plane.apply_unit(
            0,
            0,
            &payload(|w| {
                let _ = UnitRecord::Status(UnitStatus::PRESENT | UnitStatus::HOT_SWAP_LATCHED_OFF)
                    .encode(w);
            }),
        );
        plane.apply_summary(
            10,
            &payload(|w| {
                for slot in 0..2u8 {
                    let _ = SlotSummary {
                        slot,
                        status: SlotStatus::PRESENT | SlotStatus::ENABLED,
                        volts: 0,
                        amps: 0,
                        watts: 0,
                        led: LedMode::Off,
                    }
                    .encode(w);
                }
            }),
        );
        assert_eq!(plane.nodes[0].power, PowerState::Fault);
        assert_eq!(plane.nodes[1].power, PowerState::Fault);
    }
}
