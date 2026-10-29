//! What the client has to tell the rest of the firmware.
//!
//! The plane's own events (ADR 0003 EVENTS 0x60, kinds 1-36) arrive on
//! EXP_INT, are turned into [`ClientEvent`]s with the node id or the
//! unit they belong to, and are acknowledged with EV_ACK once they are
//! in the queue: the line stays asserted until they are, so the client
//! acknowledges only what it has really taken (ADR 0003 decision 3).
//!
//! The client's own findings travel in the same queue: a plane found or
//! lost, a plane that rebooted, a suspected address collision, a job
//! that ended, a config set pushed, an update that finished, and the
//! OFF commands it re-applied after a plane lost its intended state.

use alloc::vec::Vec;

use granite_proto::{EventKind, ImageState, ResultCode, Version};

use crate::NodeId;

use super::command::{JobId, JobOutcome};
use super::update::UpdateFailure;

/// One thing the client has learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientEvent {
    /// A plane answered at an address that had none.
    PlaneFound {
        /// Its address.
        address: u8,
        /// Its firmware version.
        firmware: Version,
        /// Its boot count.
        boot_count: u32,
    },
    /// A plane the client knew stopped answering
    /// ([`super::link::LOST_AFTER_FAILURES`] failed transactions).
    PlaneLost {
        /// Its address.
        address: u8,
    },
    /// A plane's boot count changed under the client's feet.
    PlaneRebooted {
        /// Its address.
        address: u8,
        /// The new boot count.
        boot_count: u32,
        /// The plane's own reset cause code.
        reset_cause: u8,
        /// The plane lost its intended state with its power: OFF slots
        /// came back ON and the client re-applies them.
        intended_state_lost: bool,
    },
    /// Every answer from an address fails its CRC with a valid length:
    /// two planes are probably strapped to it (ADR 0003). The straps
    /// have to be fixed; nothing in software can.
    AddressCollisionSuspected {
        /// The address.
        address: u8,
    },
    /// A plane event about one slot.
    Node {
        /// The node id 1-64.
        node: NodeId,
        /// Its plane's address.
        address: u8,
        /// Its slot.
        slot: u8,
        /// What happened.
        kind: EventKind,
        /// The plane's uptime when it happened \[s\].
        uptime_s: u32,
    },
    /// A plane event about one unit board.
    Unit {
        /// The plane's address.
        address: u8,
        /// Unit 0-3.
        unit: u8,
        /// What happened.
        kind: EventKind,
        /// The plane's uptime when it happened \[s\].
        uptime_s: u32,
    },
    /// A plane event about the plane itself.
    Plane {
        /// Its address.
        address: u8,
        /// What happened.
        kind: EventKind,
        /// The plane's uptime when it happened \[s\].
        uptime_s: u32,
    },
    /// A plane's event ring overflowed before the client read it.
    EventsLost {
        /// Its address.
        address: u8,
        /// How many events went missing.
        count: u16,
    },
    /// A job ended; [`super::PlaneClient::drain_jobs`] has the detail.
    JobFinished {
        /// The job.
        id: JobId,
        /// How it ended.
        outcome: JobOutcome,
        /// The worst result code a plane reported, if one did.
        code: Option<ResultCode>,
    },
    /// A config set was written and stored: the plane's hash changed.
    ConfigPushed {
        /// The plane's address.
        address: u8,
        /// The hash the plane reports now.
        hash: u32,
    },
    /// A config set was written and the plane's hash did not change,
    /// [`super::config::PUSH_TRIES`] times over: the plane refused it.
    ConfigRefused {
        /// The plane's address.
        address: u8,
    },
    /// A plane firmware update ended well: the new image is confirmed
    /// (ADR 0004 decision 8).
    UpdateConfirmed {
        /// The plane's address.
        address: u8,
        /// The version that now runs.
        version: Version,
    },
    /// A plane firmware update ended badly. Nothing was confirmed, so
    /// the plane's probation rolls the image back on its own.
    UpdateFailed {
        /// The plane's address.
        address: u8,
        /// The version that was being installed.
        version: Version,
        /// Where it broke.
        reason: UpdateFailure,
        /// What the plane says about its running image, if it is
        /// answering.
        image_state: Option<ImageState>,
    },
    /// A plane reported "intended state lost" and the client put the
    /// nodes it last commanded off back off.
    IntendedOffReapplied {
        /// The plane's address.
        address: u8,
        /// The nodes it re-applied OFF to.
        nodes: Vec<NodeId>,
    },
}

impl ClientEvent {
    /// The plane this event is about.
    pub const fn address(&self) -> Option<u8> {
        match self {
            ClientEvent::PlaneFound { address, .. }
            | ClientEvent::PlaneLost { address }
            | ClientEvent::PlaneRebooted { address, .. }
            | ClientEvent::AddressCollisionSuspected { address }
            | ClientEvent::Node { address, .. }
            | ClientEvent::Unit { address, .. }
            | ClientEvent::Plane { address, .. }
            | ClientEvent::EventsLost { address, .. }
            | ClientEvent::ConfigPushed { address, .. }
            | ClientEvent::ConfigRefused { address }
            | ClientEvent::UpdateConfirmed { address, .. }
            | ClientEvent::UpdateFailed { address, .. }
            | ClientEvent::IntendedOffReapplied { address, .. } => Some(*address),
            ClientEvent::JobFinished { .. } => None,
        }
    }

    /// The node this event is about, if it is about one.
    pub const fn node(&self) -> Option<NodeId> {
        match self {
            ClientEvent::Node { node, .. } => Some(*node),
            _ => None,
        }
    }

    /// A short lowercase name, for a log line or an MQTT event kind.
    pub const fn as_str(&self) -> &'static str {
        match self {
            ClientEvent::PlaneFound { .. } => "plane_found",
            ClientEvent::PlaneLost { .. } => "plane_lost",
            ClientEvent::PlaneRebooted { .. } => "plane_rebooted",
            ClientEvent::AddressCollisionSuspected { .. } => "address_collision",
            ClientEvent::Node { kind, .. }
            | ClientEvent::Unit { kind, .. }
            | ClientEvent::Plane { kind, .. } => kind_name(*kind),
            ClientEvent::EventsLost { .. } => "events_lost",
            ClientEvent::JobFinished { .. } => "job_finished",
            ClientEvent::ConfigPushed { .. } => "config_pushed",
            ClientEvent::ConfigRefused { .. } => "config_refused",
            ClientEvent::UpdateConfirmed { .. } => "update_confirmed",
            ClientEvent::UpdateFailed { .. } => "update_failed",
            ClientEvent::IntendedOffReapplied { .. } => "intended_off_reapplied",
        }
    }
}

/// The lowercase name of a plane event kind (ADR 0003 EVENTS kinds).
pub const fn kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::SlotOn => "slot_on",
        EventKind::SlotOff => "slot_off",
        EventKind::PgdLost => "pgd_lost",
        EventKind::OvLatched => "ov_latched",
        EventKind::PowerLimitTrip => "power_limit_trip",
        EventKind::RetryLimitReached => "retry_limit_reached",
        EventKind::LedChanged => "led_changed",
        EventKind::SequenceDone => "sequence_done",
        EventKind::SequenceFailed => "sequence_failed",
        EventKind::UnitPresent => "unit_present",
        EventKind::UnitLost => "unit_lost",
        EventKind::UnitResetSeen => "unit_reset_seen",
        EventKind::UnitHotSwapLatchedOff => "unit_hot_swap_latched_off",
        EventKind::Boot => "plane_boot",
        EventKind::ConfigStored => "config_stored",
        EventKind::UpdateStateChanged => "update_state_changed",
        EventKind::ImageConfirmed => "image_confirmed",
        EventKind::RolledBack => "rolled_back",
    }
}
