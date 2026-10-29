//! The plane client: the controller's side of the plane link.
//!
//! See `firmware/docs/adr/0004-controller-rev-d-node-model.md`
//! (decision 5 names this module and its duties) and
//! `firmware/docs/adr/0003-plane-link-protocol.md` (the protocol it
//! speaks). The wire format itself is `granite_proto` and is never
//! duplicated here: this module is the timing, the state machines and
//! the models.
//!
//! - [`client::PlaneClient`] is the whole thing, driven by
//!   [`client::PlaneClient::poll`] with a monotonic millisecond clock
//!   and a [`crate::hal::PlaneLink`]. The firmware calls it every
//!   50-100 ms and the client decides what is due.
//! - [`model`]: [`Plane`], [`Unit`] and [`Node`], the two state axes of
//!   ADR 0004 decision 3, and the node id mapping.
//! - [`command`]: the ADR 0004 verbs as jobs with a sequence number per
//!   plane, at most [`MAX_IN_FLIGHT`] in flight, results matched by
//!   sequence number and drained as [`command::FinishedJob`].
//! - [`event`]: plane events and the client's own, drained as
//!   [`event::ClientEvent`].
//! - [`config`]: the desired plane config, pushed with CONFIG_W when
//!   the plane's hash does not match.
//! - [`update`]: one plane firmware update at a time (ADR 0004
//!   decision 8).
//! - [`intent`]: the nodes the controller last commanded rail-off, so
//!   it can put them back after a plane reports "intended state lost".
//!
//! Everything is synchronous and allocation-light: one transaction per
//! link call, a bounded number of transactions per [`poll`], no
//! blocking and no `async`, so the host tests drive it with a fake
//! clock exactly as the firmware's task does with a real one.
//!
//! [`poll`]: client::PlaneClient::poll
//!
//! # Node ids
//!
//! `id = 8 * (address - 0x30) + slot + 1`, so 1-64 (ADR 0004
//! decision 2). [`node_id`] and [`node_location`] are the only two
//! places that arithmetic lives, and [`crate::is_node`] is the same
//! range.

pub mod client;
pub mod command;
pub mod config;
pub mod event;
pub mod intent;
pub mod link;
pub mod model;
pub mod update;

use granite_proto::{ADDR_BASE, MAX_PLANES, SLOT_COUNT};

pub use client::{PlaneClient, UpdatePolicy};
pub use command::{
    Command, FinishedJob, JobId, JobOutcome, NodeCommand, NodeOptions, OFF_TIMEOUT_S, ON_TIMEOUT_S,
    PRESS_MS, PlaneCommand, STAGGER_MS,
};
pub use config::DesiredConfig;
pub use event::ClientEvent;
pub use intent::IntendedOff;
pub use link::{PlaneErrors, PlaneLinkError, Transactions};
pub use model::{HostState, Node, NodeTelemetry, Plane, PowerState, Unit};
pub use update::{BufferedImage, ImageSource, UpdateFailure, UpdateRefused, UpdateStage};

use crate::NodeId;

/// Nodes a controller can serve: 8 planes of 8 slots (ADR 0004
/// decision 2).
pub const PLANE_NODE_COUNT: usize = MAX_PLANES * SLOT_COUNT;

/// Commands the client keeps in flight on one plane (ADR 0003
/// decision 4: the plane remembers 8 sequence numbers, the controller
/// stays well under it).
pub const MAX_IN_FLIGHT: usize = 4;

/// The node id of a slot on a plane, if both are in range.
///
/// `8 * (address - 0x30) + slot + 1`: the id follows the address straps
/// and the slot position, so it survives a reboot, a re-discovery and a
/// plane swap with the same straps (ADR 0004 decision 2).
pub const fn node_id(address: u8, slot: u8) -> Option<NodeId> {
    match granite_proto::plane_of_address(address) {
        Some(n) if (slot as usize) < SLOT_COUNT => Some(8 * n + slot + 1),
        _ => None,
    }
}

/// The plane address and slot a node id names, if it is a node id.
pub const fn node_location(id: NodeId) -> Option<(u8, u8)> {
    if id == 0 || id as usize > PLANE_NODE_COUNT {
        return None;
    }
    let zero = id - 1;
    Some((ADDR_BASE + zero / 8, zero % 8))
}

/// True when `id` names a node of some plane (1-64).
pub const fn is_plane_node(id: NodeId) -> bool {
    id != 0 && id as usize <= PLANE_NODE_COUNT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_follow_the_straps_and_the_slot() {
        assert_eq!(node_id(0x30, 0), Some(1));
        assert_eq!(node_id(0x30, 7), Some(8));
        assert_eq!(node_id(0x31, 0), Some(9));
        assert_eq!(node_id(0x37, 7), Some(64));
        assert_eq!(node_id(0x38, 0), None);
        assert_eq!(node_id(0x2F, 0), None);
        assert_eq!(node_id(0x30, 8), None);

        for id in 1..=PLANE_NODE_COUNT as NodeId {
            let (address, slot) = node_location(id).expect("every id has a location");
            assert_eq!(node_id(address, slot), Some(id));
        }
        assert_eq!(node_location(0), None);
        assert_eq!(node_location(65), None);
        assert!(is_plane_node(64) && !is_plane_node(65) && !is_plane_node(0));
    }
}
