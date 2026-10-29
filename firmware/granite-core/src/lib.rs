//! Host-testable core of the Granite controller firmware.
//!
//! See `firmware/docs/adr/0001-firmware-architecture.md` and
//! `firmware/docs/adr/0004-controller-rev-d-node-model.md`. This crate
//! holds everything that can be reasoned about and tested without
//! hardware:
//!
//! - [`api`]: the HTTP route table, transport-agnostic, with the traits
//!   the platform fills in (network, identity, OTA, randomness).
//! - [`hal`]: the traits the ESP-IDF binary (and the host simulator)
//!   implement. No ESP-IDF, embedded-hal or vendor types leak in here.
//! - [`node`]: per-node settings: name, boot policy, LED sense, the
//!   soft-off window and the escalation ladder.
//! - [`plane`]: the controller's side of the plane link (ADR 0004
//!   decision 5): discovery, the poll schedule, the plane / unit / node
//!   models, the command jobs, the events, the config push and the
//!   plane firmware updates, over [`hal::PlaneLink`] and
//!   `granite_proto`. A node is a slot on a plane; nothing in the
//!   controller presses a button itself any more.
//! - [`observed`]: the single sensor snapshot every reader works from.
//! - [`jobs`]: the plane jobs a command produced, so the ack that
//!   follows the plane's result can find the command id again.
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

pub mod api;
pub mod config;
pub mod dispatch;
pub mod hal;
pub mod jobs;
pub mod modbus_map;
pub mod modbus_server;
pub mod msg;
pub mod node;
pub mod observed;
pub mod plane;
pub mod rules;

/// Nodes a controller can serve: 8 planes of 8 slots (ADR 0004
/// decision 2).
pub const MAX_NODES: usize = plane::PLANE_NODE_COUNT;

/// Planes a controller can serve, addresses 0x30-0x37.
pub const MAX_PLANES: usize = granite_proto::MAX_PLANES;

/// Unit boards per plane.
pub const UNIT_COUNT: usize = granite_proto::UNIT_COUNT;

/// Number of dry-contact inputs.
pub const DRY_COUNT: usize = 4;

/// Number of probe slots exposed in the data model (4 connectors on the
/// board, 8 slots so a splitter or a second bus segment fits without a
/// map bump).
pub const PROBE_SLOTS: usize = 8;

/// Node identifier as used on the wire: 1..=64 (ADR 0004 decision 2,
/// `8 * (address - 0x30) + slot + 1`).
pub type NodeId = u8;

/// True if `node` is a usable node id.
pub const fn is_node(node: NodeId) -> bool {
    node >= 1 && node as usize <= MAX_NODES
}

/// Zero-based index for a wire node id, if it is in range.
pub fn node_index(node: NodeId) -> Option<usize> {
    if is_node(node) {
        Some(node as usize - 1)
    } else {
        None
    }
}

/// True if `plane` is a plane number (0-7).
pub const fn is_plane(plane: u8) -> bool {
    (plane as usize) < MAX_PLANES
}

/// True if `unit` is a unit index (0-3).
pub const fn is_unit(unit: u8) -> bool {
    (unit as usize) < UNIT_COUNT
}

/// What a command or a rule acts on.
///
/// A node is one slot of one plane, and the plane and unit targets carry
/// the verbs that belong to a board rather than to a node
/// (`plane_reboot`, `unit_reset`). `all` is every node the client knows
/// is present, which with the plane means "every present slot of every
/// answering plane" rather than a fixed eight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Target {
    /// Every present node, in id order.
    #[default]
    All,
    /// One node, 1..=64.
    Node(NodeId),
    /// One plane, 0..=7.
    Plane(u8),
    /// One unit board of one plane.
    Unit {
        /// Plane number 0..=7.
        plane: u8,
        /// Unit index 0..=3.
        unit: u8,
    },
}

impl Target {
    /// The node id, if this is a single node.
    pub const fn node(self) -> Option<NodeId> {
        match self {
            Target::Node(n) => Some(n),
            _ => None,
        }
    }

    /// The plane number, if this names one.
    pub const fn plane(self) -> Option<u8> {
        match self {
            Target::Plane(p) | Target::Unit { plane: p, .. } => Some(p),
            _ => None,
        }
    }

    /// The unit, if this names one.
    pub const fn unit(self) -> Option<(u8, u8)> {
        match self {
            Target::Unit { plane, unit } => Some((plane, unit)),
            _ => None,
        }
    }

    /// The plane's I2C address, if this names a plane or a unit.
    pub const fn address(self) -> Option<u8> {
        match self.plane() {
            Some(p) if is_plane(p) => Some(granite_proto::ADDR_BASE + p),
            _ => None,
        }
    }

    /// True when the target names something that could exist.
    pub const fn is_valid(self) -> bool {
        match self {
            Target::All => true,
            Target::Node(n) => is_node(n),
            Target::Plane(p) => is_plane(p),
            Target::Unit { plane, unit } => is_plane(plane) && is_unit(unit),
        }
    }

    /// Parse the wire form: `all`, a node id, `planeN` or `planeN.unitU`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Some(Target::All);
        }
        if let Ok(n) = s.parse::<NodeId>() {
            return is_node(n).then_some(Target::Node(n));
        }
        let (plane, unit) = match s.split_once('.') {
            Some((p, u)) => (p, Some(u)),
            None => (s, None),
        };
        let number: u8 = strip_prefix_ci(plane, "plane")?.parse().ok()?;
        if !is_plane(number) {
            return None;
        }
        match unit {
            None => Some(Target::Plane(number)),
            Some(u) => {
                let index: u8 = strip_prefix_ci(u, "unit")?.parse().ok()?;
                is_unit(index).then_some(Target::Unit {
                    plane: number,
                    unit: index,
                })
            }
        }
    }
}

/// `s` without a case-insensitive prefix.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, rest) = s.split_at_checked(prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then_some(rest)
}

impl core::fmt::Display for Target {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Target::All => f.write_str("all"),
            Target::Node(n) => write!(f, "{n}"),
            Target::Plane(p) => write!(f, "plane{p}"),
            Target::Unit { plane, unit } => write!(f, "plane{plane}.unit{unit}"),
        }
    }
}

impl serde::Serialize for Target {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Target::Node(n) => s.serialize_u8(*n),
            other => s.serialize_str(&alloc::string::ToString::to_string(other)),
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
            Repr::Str(s) => Target::parse(&s).ok_or_else(|| {
                D::Error::custom("target must be \"all\", 1..=64, \"planeN\" or \"planeN.unitU\"")
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_parse_and_print_the_same_way() {
        for (text, target) in [
            ("all", Target::All),
            ("1", Target::Node(1)),
            ("64", Target::Node(64)),
            ("plane0", Target::Plane(0)),
            ("plane7", Target::Plane(7)),
            ("plane3.unit2", Target::Unit { plane: 3, unit: 2 }),
        ] {
            assert_eq!(Target::parse(text), Some(target), "parse {text}");
            assert_eq!(alloc::string::ToString::to_string(&target), text);
            assert!(target.is_valid());
        }
        assert_eq!(Target::parse("0"), None);
        assert_eq!(Target::parse("65"), None);
        assert_eq!(Target::parse("plane8"), None);
        assert_eq!(Target::parse("plane1.unit4"), None);
        assert_eq!(Target::parse("node1"), None);
        assert_eq!(Target::Plane(2).address(), Some(0x32));
    }
}
