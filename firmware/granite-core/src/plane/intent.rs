//! The nodes the controller means to keep rail-off.
//!
//! ADR 0004's expert answer: "After a plane power loss the plane flags
//! 'intended state lost'; the controller then re-applies OFF to every
//! node it last commanded OFF (kept in NVS, so this also works after
//! both rebooted)."
//!
//! **What goes in the set.** A node joins it when the controller asked
//! for something whose end state is "the rail is down":
//!
//! - `power_off` always: it is the rail and nothing else;
//! - `off` and `force_off` only when the rail really ended up down.
//!   Both escalate to a five-second PWR hold and cut the rail if the
//!   host stays lit, but a host that obeyed its button keeps its
//!   standby rail (ADR 0004 decision 4). That node needs nothing
//!   re-applied: after a plane power loss its rail was up before and
//!   is up again, and cutting it would be a state the controller never
//!   asked for. So the client waits for the next SUMMARY after the
//!   sequence ends and looks at the slot's ENABLED bit
//!   ([`INTENT_SETTLE_MS`]).
//!
//! A node leaves the set as soon as the controller asks for anything
//! that ends with the rail up (`on`, `on_all`, `power_on`, `cycle`,
//! `clear_latch`), and when a plane reports the slot enabled by some
//! other hand (a local button, a plane-side retry) the set is not
//! touched: the controller's intent is what it asked for, not what it
//! sees.
//!
//! **Persistence.** The set is a handful of node ids, so it is handed
//! to the firmware as a slice rather than through a storage trait the
//! client would have to call: [`IntendedOff::take_dirty`] says when it
//! changed, [`IntendedOff::nodes`] is what to write, and
//! [`IntendedOff::load`] puts back what was read from NVS at boot.

use alloc::vec::Vec;

use crate::NodeId;

/// How long the client waits for a fresh SUMMARY before it gives up on
/// deciding whether an `off` ended with the rail cut \[ms\].
pub const INTENT_SETTLE_MS: u64 = 3_000;

/// The set of nodes the controller last commanded rail-off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntendedOff {
    nodes: Vec<NodeId>,
    dirty: bool,
}

impl IntendedOff {
    /// An empty set.
    pub fn new() -> Self {
        IntendedOff::default()
    }

    /// The set, lowest node id first. This is what the firmware
    /// persists.
    pub fn nodes(&self) -> &[NodeId] {
        &self.nodes
    }

    /// True when `node` is in the set.
    pub fn contains(&self, node: NodeId) -> bool {
        self.nodes.contains(&node)
    }

    /// How many nodes are in it.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when nothing is in it.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The nodes of one plane that are in the set.
    pub fn of_plane(&self, address: u8) -> Vec<NodeId> {
        self.nodes
            .iter()
            .copied()
            .filter(|n| super::node_location(*n).is_some_and(|(a, _)| a == address))
            .collect()
    }

    /// Put back what was persisted. Not a change to persist again.
    pub fn load(&mut self, nodes: &[NodeId]) {
        self.nodes = nodes
            .iter()
            .copied()
            .filter(|n| super::is_plane_node(*n))
            .collect();
        self.nodes.sort_unstable();
        self.nodes.dedup();
        self.dirty = false;
    }

    /// Add a node. True when the set changed.
    pub fn insert(&mut self, node: NodeId) -> bool {
        if !super::is_plane_node(node) || self.nodes.contains(&node) {
            return false;
        }
        self.nodes.push(node);
        self.nodes.sort_unstable();
        self.dirty = true;
        true
    }

    /// Remove a node. True when the set changed.
    pub fn remove(&mut self, node: NodeId) -> bool {
        let before = self.nodes.len();
        self.nodes.retain(|n| *n != node);
        let changed = self.nodes.len() != before;
        self.dirty |= changed;
        changed
    }

    /// True while the set has changed since the firmware last stored
    /// it; clears the flag.
    pub fn take_dirty(&mut self) -> bool {
        core::mem::take(&mut self.dirty)
    }

    /// True while the set has changed since the firmware last stored
    /// it, without clearing the flag.
    pub const fn is_dirty(&self) -> bool {
        self.dirty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_is_sorted_deduplicated_and_says_when_it_changed() {
        let mut set = IntendedOff::new();
        assert!(set.is_empty() && !set.is_dirty());
        assert!(set.insert(9));
        assert!(set.insert(1));
        assert!(!set.insert(9), "already in");
        assert!(!set.insert(0), "not a node id");
        assert_eq!(set.nodes(), &[1, 9]);
        assert!(set.take_dirty() && !set.take_dirty());

        assert_eq!(set.of_plane(0x30), alloc::vec![1]);
        assert_eq!(set.of_plane(0x31), alloc::vec![9]);
        assert!(set.remove(1) && !set.remove(1));
        assert!(set.take_dirty());

        set.load(&[64, 64, 3, 200]);
        assert_eq!(set.nodes(), &[3, 64]);
        assert!(!set.is_dirty(), "a load is not a change to store back");
    }
}
