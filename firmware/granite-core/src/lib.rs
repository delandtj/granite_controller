//! Host-testable core of the Granite controller firmware.
//!
//! See `firmware/docs/adr/0001-firmware-architecture.md`. This crate holds
//! everything that can be reasoned about and tested without hardware:
//!
//! - [`api`]: the HTTP route table, transport-agnostic, with the traits
//!   the platform fills in (network, identity, OTA, randomness).
//! - [`hal`]: the traits the ESP-IDF binary (and the host simulator)
//!   implement. No ESP-IDF, embedded-hal or vendor types leak in here.
//! - [`node`]: per-node state, LED debounce, names, boot policy.
//! - [`actuator`]: the action table from the ADR as a state machine over
//!   [`hal::NodeSwitches`] plus a clock, with a bounded job queue.
//! - [`observed`]: the single sensor snapshot every reader works from.
//! - [`rules`]: up to 16 standalone rules evaluated against the snapshot.
//! - [`config`]: one `Config`, serialised per section, secrets separate.
//! - [`msg`]: the command/reply/event vocabulary shared by MQTT, HTTP and
//!   Modbus.
//! - [`modbus_map`]: the register table as data, both mapping directions
//!   and a Markdown generator.
//! - [`modbus_server`]: the Modbus TCP frame handler over byte slices,
//!   plus the IPv4/CIDR allow-list. No sockets.
//! - [`dispatch`]: the one entry point every transport funnels through.
//!
//! The crate is `no_std` + `alloc`; the `std` feature (on by default, used
//! by the tests and the host simulator) only turns on `std` in serde.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod actuator;
pub mod api;
pub mod config;
pub mod dispatch;
pub mod hal;
pub mod modbus_map;
pub mod modbus_server;
pub mod msg;
pub mod node;
pub mod observed;
pub mod rules;

/// Number of nodes a controller drives.
pub const NODE_COUNT: usize = 8;

/// Number of dry-contact inputs.
pub const DRY_COUNT: usize = 4;

/// Number of probe slots exposed in the data model (4 connectors on rev C,
/// 8 slots so a splitter or a second bus segment fits without a map bump).
pub const PROBE_SLOTS: usize = 8;

/// Node identifier as used on the wire: 1..=8.
pub type NodeId = u8;

/// True if `node` is a usable node id.
pub const fn is_node(node: NodeId) -> bool {
    node >= 1 && node as usize <= NODE_COUNT
}

/// Zero-based index for a wire node id, if it is in range.
pub fn node_index(node: NodeId) -> Option<usize> {
    if is_node(node) {
        Some(node as usize - 1)
    } else {
        None
    }
}

/// What a command or a rule acts on: one node or every node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    /// All 8 nodes, in the configured order.
    #[default]
    All,
    /// One node, 1..=8.
    Node(NodeId),
}

impl Target {
    /// The node id, if this is a single node.
    pub const fn node(self) -> Option<NodeId> {
        match self {
            Target::Node(n) => Some(n),
            Target::All => None,
        }
    }

    /// True when the target names a node that does not exist.
    pub const fn is_valid(self) -> bool {
        match self {
            Target::All => true,
            Target::Node(n) => is_node(n),
        }
    }
}

impl core::fmt::Display for Target {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Target::All => f.write_str("all"),
            Target::Node(n) => write!(f, "{n}"),
        }
    }
}

impl serde::Serialize for Target {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Target::All => s.serialize_str("all"),
            Target::Node(n) => s.serialize_u8(*n),
        }
    }
}

impl<'de> serde::Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Num(i64),
            Str(alloc::string::String),
        }

        match Repr::deserialize(d)? {
            Repr::Num(n) => {
                let n = u8::try_from(n).map_err(|_| D::Error::custom("node id out of range"))?;
                if is_node(n) {
                    Ok(Target::Node(n))
                } else {
                    Err(D::Error::custom("node id out of range"))
                }
            }
            Repr::Str(s) => {
                let s = s.trim();
                if s.eq_ignore_ascii_case("all") {
                    return Ok(Target::All);
                }
                match s.parse::<u8>() {
                    Ok(n) if is_node(n) => Ok(Target::Node(n)),
                    _ => Err(D::Error::custom("target must be \"all\" or 1..=8")),
                }
            }
        }
    }
}
