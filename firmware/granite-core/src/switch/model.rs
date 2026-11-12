//! What the controller knows about the switch, and what it wants it to
//! be (ADR 0006 component 15).
//!
//! [`SwitchState`] is the read side: one field per CGI page, each
//! `Option` or an empty `Vec` while that page has not been read, so a
//! partly polled switch is a valid state and the planner can tell "not
//! managed" from "not read yet". [`SwitchDesired`] is the write side and
//! lives in the `switch` config section; an absent key there means "do
//! not manage" (ADR 0006, "Config section `switch`").
//!
//! no_std + alloc and serde, like every other model in this crate: the
//! payloads are built and tested on the host.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use serde::{Deserialize, Serialize};

/// Ports on an SL902-SWTGW218AS: 8x 2.5G copper plus the SFP+.
pub const PORT_COUNT: u8 = 9;

/// The SFP+ port, the only one that does 10G and the usual uplink.
pub const SFP_PORT: u8 = 9;

/// Firmware versions whose pages these parsers were written against.
/// Anything else is monitored if it parses, never written (ADR 0006
/// component 16, `switch_fw_unknown`).
pub const KNOWN_FIRMWARE: [&str; 1] = ["V200.1.8"];

/// True when `port` is a port of this switch.
pub const fn is_port(port: u8) -> bool {
    port >= 1 && port <= PORT_COUNT
}

/// True when the firmware string is one the writes were tested against.
pub fn firmware_known(firmware: &str) -> bool {
    KNOWN_FIRMWARE.contains(&firmware.trim())
}

/// A MAC address in the switch's own notation, normalised to uppercase
/// colon form (`1C:2A:A3:24:26:7E`).
pub fn normalise_mac(mac: &str) -> String {
    mac.trim().to_ascii_uppercase().replace(['-', '.'], ":")
}

/// True when two MAC strings name the same address whatever the
/// separator and case: the pairing check of ADR 0006 component 22.
pub fn same_mac(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && normalise_mac(a) == normalise_mac(b)
}

/// `info.cgi`: who the switch says it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitchInfo {
    /// Device name, as the operator set it.
    pub name: String,
    /// Model string.
    pub model: String,
    /// Firmware version, e.g. `V200.1.8`.
    pub firmware: String,
    /// Management MAC, uppercase colon form.
    pub mac: String,
    /// Management address.
    pub ip: String,
    /// Management netmask.
    pub netmask: String,
    /// Seconds since the switch booted.
    pub uptime_s: u32,
}

/// Negotiated or configured line speed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Speed {
    /// Auto-negotiation.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// 10 Mbit/s.
    #[serde(rename = "10m")]
    M10,
    /// 100 Mbit/s.
    #[serde(rename = "100m")]
    M100,
    /// 1 Gbit/s.
    #[serde(rename = "1g")]
    G1,
    /// 2.5 Gbit/s.
    #[serde(rename = "2.5g")]
    G2P5,
    /// 10 Gbit/s, port 9 only.
    #[serde(rename = "10g")]
    G10,
}

impl Speed {
    /// The name used in config and on the wire.
    pub const fn as_str(self) -> &'static str {
        match self {
            Speed::Auto => "auto",
            Speed::M10 => "10M",
            Speed::M100 => "100M",
            Speed::G1 => "1G",
            Speed::G2P5 => "2.5G",
            Speed::G10 => "10G",
        }
    }

    /// The `speed` form value of `port.cgi` (`tools/sodola.py` SPEEDS).
    pub const fn form_value(self) -> &'static str {
        match self {
            Speed::Auto => "10",
            Speed::M10 => "0",
            Speed::M100 => "1",
            Speed::G1 => "2",
            Speed::G2P5 => "5",
            Speed::G10 => "4",
        }
    }

    /// Parse a name, accepting both the config spelling and the labels
    /// the switch's own tables print (`1000M`, `2500M`).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        for speed in [
            Speed::Auto,
            Speed::M10,
            Speed::M100,
            Speed::G1,
            Speed::G2P5,
            Speed::G10,
        ] {
            if s.eq_ignore_ascii_case(speed.as_str()) {
                return Some(speed);
            }
        }
        match s {
            "1000M" => Some(Speed::G1),
            "2500M" => Some(Speed::G2P5),
            "10000M" => Some(Speed::G10),
            _ => None,
        }
    }

    /// True when this speed cannot be set on `port`: no 10G on the
    /// copper ports, no 10M on the SFP+ (`docs/sodola.md`, "Quirks").
    pub const fn impossible_on(self, port: u8) -> bool {
        match self {
            Speed::G10 => port != SFP_PORT,
            Speed::M10 => port == SFP_PORT,
            _ => false,
        }
    }
}

impl fmt::Display for Speed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Duplex, configured or negotiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Duplex {
    /// Auto-negotiation.
    #[default]
    Auto,
    /// Full duplex.
    Full,
    /// Half duplex.
    Half,
}

impl Duplex {
    /// Name used in payloads.
    pub const fn as_str(self) -> &'static str {
        match self {
            Duplex::Auto => "auto",
            Duplex::Full => "full",
            Duplex::Half => "half",
        }
    }

    /// The `duplex` form value of `port.cgi`.
    pub const fn form_value(self) -> &'static str {
        match self {
            Duplex::Auto => "2",
            Duplex::Full => "1",
            Duplex::Half => "0",
        }
    }

    /// Parse a table label (`Auto`, `Full Duplex`, `Half Duplex`).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("auto") {
            return Some(Duplex::Auto);
        }
        if s.starts_with("Full") || s.eq_ignore_ascii_case("full") {
            return Some(Duplex::Full);
        }
        if s.starts_with("Half") || s.eq_ignore_ascii_case("half") {
            return Some(Duplex::Half);
        }
        None
    }
}

/// `port.cgi`: one port's admin configuration and what it negotiated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortCfg {
    /// Port number, 1-based.
    pub port: u8,
    /// Admin state.
    pub enabled: bool,
    /// Configured duplex.
    pub duplex: Duplex,
    /// Negotiated duplex.
    pub duplex_actual: Duplex,
    /// Configured speed.
    pub speed: Speed,
    /// Negotiated speed.
    pub speed_actual: Speed,
    /// Configured flow control.
    pub flow: bool,
    /// Flow control in force.
    pub flow_actual: bool,
}

/// `port.cgi?page=stats`: link state and the four counters the switch
/// keeps. There are no error, CRC or drop counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PortStats {
    /// Port number, 1-based.
    pub port: u8,
    /// Admin state.
    pub enabled: bool,
    /// Link state.
    pub link: bool,
    /// Good packets sent.
    pub tx_packets: u64,
    /// Good packets received.
    pub rx_packets: u64,
    /// Good bytes sent.
    pub tx_bytes: u64,
    /// Good bytes received.
    pub rx_bytes: u64,
}

/// One 802.1Q VLAN.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vlan {
    /// VLAN id, 1-4094.
    pub id: u16,
    /// Name, up to 14 characters, often empty.
    #[serde(default)]
    pub name: String,
}

/// `vlan.cgi?page=static`: the VLAN table and whether 802.1Q is on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VlanTable {
    /// True when "VLAN Manage" is on.
    pub enabled: bool,
    /// The VLANs, in the order the page lists them.
    pub vlans: Vec<Vlan>,
}

impl VlanTable {
    /// True when the table holds `vid`.
    pub fn has(&self, vid: u16) -> bool {
        self.vlans.iter().any(|v| v.id == vid)
    }
}

/// How a port handles VLAN tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VlanMode {
    /// Untagged member of one VLAN.
    #[default]
    Access,
    /// Tagged member of a list, untagged on the native VLAN.
    Trunk,
}

impl VlanMode {
    /// Name used in config and payloads.
    pub const fn as_str(self) -> &'static str {
        match self {
            VlanMode::Access => "access",
            VlanMode::Trunk => "trunk",
        }
    }

    /// The `vlan_type` form value of `vlan.cgi?page=port_based`.
    pub const fn form_value(self) -> &'static str {
        match self {
            VlanMode::Access => "0",
            VlanMode::Trunk => "1",
        }
    }

    /// Parse a table label.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("access") {
            Some(VlanMode::Access)
        } else if s.eq_ignore_ascii_case("trunk") {
            Some(VlanMode::Trunk)
        } else {
            None
        }
    }
}

/// `vlan.cgi?page=port_based`: one port's VLAN membership.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortVlan {
    /// Port number, 1-based.
    pub port: u8,
    /// Access or trunk.
    pub mode: VlanMode,
    /// The access VLAN, when the port is an access port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access: Option<u16>,
    /// The native (untagged) VLAN of a trunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<u16>,
    /// The VLANs a trunk carries, ascending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<u16>,
}

/// `fwd.cgi?page=jumboframe`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Jumbo {
    /// Whether jumbo frames are on at all.
    pub enabled: bool,
    /// Maximum frame size in bytes.
    pub size: u32,
}

/// The frame sizes the switch offers, with their form values.
pub const JUMBO_SIZES: [(u32, &str); 6] = [
    (1522, "1"),
    (1536, "2"),
    (1552, "3"),
    (9216, "4"),
    (10_000, "5"),
    (12_000, "6"),
];

/// The form value for a frame size, when the switch offers it.
pub fn jumbo_form_value(size: u32) -> Option<&'static str> {
    JUMBO_SIZES
        .iter()
        .find(|(bytes, _)| *bytes == size)
        .map(|(_, value)| *value)
}

/// `loop.cgi`: loop prevention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopMode {
    /// Nothing.
    #[default]
    Off,
    /// Loop detection.
    Loop,
    /// Spanning tree.
    Stp,
}

impl LoopMode {
    /// Name used in config and payloads.
    pub const fn as_str(self) -> &'static str {
        match self {
            LoopMode::Off => "off",
            LoopMode::Loop => "loop",
            LoopMode::Stp => "stp",
        }
    }

    /// The `func_type` form value of `loop.cgi`.
    pub const fn form_value(self) -> &'static str {
        match self {
            LoopMode::Off => "0",
            LoopMode::Loop => "1",
            LoopMode::Stp => "3",
        }
    }

    /// Parse a `func_type` value.
    pub fn from_form_value(s: &str) -> Option<Self> {
        match s.trim() {
            "0" => Some(LoopMode::Off),
            "1" | "2" => Some(LoopMode::Loop),
            "3" => Some(LoopMode::Stp),
            _ => None,
        }
    }
}

/// `loop.cgi`: the mode and the two loop-detection timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopCfg {
    /// Off, loop detection or spanning tree.
    pub mode: LoopMode,
    /// Loop detection interval \[s\].
    pub interval_s: u32,
    /// Loop detection recover time \[s\]; 0 means "never".
    pub recover_s: u32,
}

impl Default for LoopCfg {
    fn default() -> Self {
        LoopCfg {
            mode: LoopMode::Off,
            interval_s: 2,
            recover_s: 10,
        }
    }
}

/// One port's storm control, in Kbit/s. `None` is unlimited, which is
/// what an empty form field means (`docs/sodola.md`, "Quirks").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StormPort {
    /// Port number, 1-based.
    pub port: u8,
    /// Broadcast limit.
    pub bcast: Option<u32>,
    /// Multicast limit.
    pub mcast: Option<u32>,
    /// Unknown unicast limit.
    pub unknown_ucast: Option<u32>,
    /// Unknown multicast limit.
    pub unknown_mcast: Option<u32>,
}

/// One port's ingress and egress rate limit, in Kbit/s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RatePort {
    /// Port number, 1-based.
    pub port: u8,
    /// Ingress limit, `None` for unlimited.
    pub ingress: Option<u32>,
    /// Egress limit, `None` for unlimited.
    pub egress: Option<u32>,
}

/// `ip.cgi`: the management address.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct IpCfg {
    /// True for DHCP.
    pub dhcp: bool,
    /// Address.
    pub ip: String,
    /// Netmask in dotted form.
    pub netmask: String,
    /// Default gateway.
    pub gateway: String,
}

/// One row of `mac.cgi?page=fwd_tbl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacEntry {
    /// The address, uppercase colon form.
    pub mac: String,
    /// True for a static entry.
    #[serde(default)]
    pub is_static: bool,
    /// The port it was learned on; `None` for the switch's own CPU port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u8>,
    /// The VLAN it was learned in.
    pub vlan: u16,
}

/// `mac.cgi?page=fwd_tbl`: one page of the forwarding table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MacPage {
    /// The rows on this page.
    pub entries: Vec<MacEntry>,
    /// Entries in the whole table.
    pub total: u32,
    /// Pages at the current page size.
    pub pages: u32,
}

/// Everything the controller has read from the switch.
///
/// A field that is `None` (or an empty `Vec`) was not read yet, which is
/// not the same as "not configured": the planner refuses to plan against
/// a page it has not seen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitchState {
    /// `info.cgi`.
    pub info: Option<SwitchInfo>,
    /// `port.cgi`.
    pub ports: Vec<PortCfg>,
    /// `vlan.cgi?page=static`.
    pub vlans: Option<VlanTable>,
    /// `vlan.cgi?page=port_based`.
    pub port_vlans: Vec<PortVlan>,
    /// `fwd.cgi?page=jumboframe`.
    pub jumbo: Option<Jumbo>,
    /// `fwd.cgi?page=storm_ctrl`.
    pub storm: Vec<StormPort>,
    /// `port.cgi?page=bw_ctrl`.
    pub rate: Vec<RatePort>,
    /// `loop.cgi`.
    pub loop_cfg: Option<LoopCfg>,
    /// `ip.cgi`.
    pub ip: Option<IpCfg>,
    /// `port.cgi?page=stats`.
    pub stats: Vec<PortStats>,
    /// The forwarding table, as far as it was walked.
    pub macs: Vec<MacEntry>,
}

impl SwitchState {
    /// The configuration of one port, if `port.cgi` was read.
    pub fn port(&self, port: u8) -> Option<&PortCfg> {
        self.ports.iter().find(|p| p.port == port)
    }

    /// The VLAN membership of one port, if the page was read.
    pub fn port_vlan(&self, port: u8) -> Option<&PortVlan> {
        self.port_vlans.iter().find(|p| p.port == port)
    }

    /// One port's storm control, if the page was read.
    pub fn storm_port(&self, port: u8) -> Option<&StormPort> {
        self.storm.iter().find(|p| p.port == port)
    }

    /// One port's rate limits, if the page was read.
    pub fn rate_port(&self, port: u8) -> Option<&RatePort> {
        self.rate.iter().find(|p| p.port == port)
    }

    /// One port's counters, if the stats page was read.
    pub fn stats_port(&self, port: u8) -> Option<&PortStats> {
        self.stats.iter().find(|p| p.port == port)
    }

    /// Which port a MAC was last seen on, if the table was walked.
    pub fn port_of_mac(&self, mac: &str) -> Option<u8> {
        self.macs
            .iter()
            .find(|e| same_mac(&e.mac, mac))
            .and_then(|e| e.port)
    }

    /// Every MAC seen on one port.
    pub fn macs_on(&self, port: u8) -> Vec<String> {
        self.macs
            .iter()
            .filter(|e| e.port == Some(port))
            .map(|e| e.mac.clone())
            .collect()
    }

    /// The firmware string, empty when `info.cgi` was not read.
    pub fn firmware(&self) -> &str {
        self.info.as_ref().map_or("", |i| i.firmware.as_str())
    }
}

// ---------------------------------------------------------------------
// The desired state: the `switch.desired` config
// ---------------------------------------------------------------------

/// One VLAN the operator wants to exist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesiredVlan {
    /// VLAN id.
    pub id: u16,
    /// Name, optional.
    pub name: String,
}

/// One port's desired configuration. Every field but `port` is
/// optional: what is not named is not managed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesiredPort {
    /// Port number, 1-based.
    pub port: u8,
    /// Access or trunk.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<VlanMode>,
    /// The access VLAN, for `mode = "access"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    /// The native VLAN, for `mode = "trunk"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native: Option<u16>,
    /// The VLANs a trunk carries. The native VLAN is always added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allowed: Vec<u16>,
    /// Admin state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Line speed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<Speed>,
    /// Which node the operator says is cabled here. A statement about
    /// cabling, checked against the MAC table, never written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node: Option<u8>,
}

/// Desired storm control over a set of ports.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesiredStorm {
    /// Ports as a range list, e.g. `1-8`.
    pub ports: String,
    /// Broadcast limit \[Kbit/s\]; absent leaves it alone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bcast_kbps: Option<u32>,
    /// Multicast limit \[Kbit/s\].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcast_kbps: Option<u32>,
    /// Unknown unicast limit \[Kbit/s\].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_ucast_kbps: Option<u32>,
    /// Unknown multicast limit \[Kbit/s\].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unknown_mcast_kbps: Option<u32>,
}

/// How the switch should get its management address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredIpMode {
    /// DHCP.
    #[default]
    Dhcp,
    /// Static.
    Static,
}

/// The desired management address.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesiredIp {
    /// DHCP or static.
    pub mode: DesiredIpMode,
    /// Address in CIDR form, e.g. `10.0.202.5/24`.
    pub addr: String,
    /// Default gateway.
    pub gw: String,
}

/// The `switch.desired` document: what the switch should look like.
///
/// Absent keys mean "do not manage", so a partly described switch is
/// valid and the rest is left alone (ADR 0006, "Config section
/// `switch`").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitchDesired {
    /// The VLANs that should exist. `None` leaves the VLAN table alone;
    /// a list is exhaustive, so a VLAN not named in it is deleted
    /// (VLAN 1 cannot be deleted and is never planned for deletion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlans: Option<Vec<DesiredVlan>>,
    /// Per-port settings, by port number.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<DesiredPort>,
    /// Maximum frame size in bytes; `0` turns jumbo frames off.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jumbo: Option<u32>,
    /// Storm control.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storm: Option<DesiredStorm>,
    /// Loop prevention mode.
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_mode: Option<LoopMode>,
    /// The management address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<DesiredIp>,
}

impl SwitchDesired {
    /// True when nothing is managed at all.
    pub fn is_empty(&self) -> bool {
        *self == SwitchDesired::default()
    }

    /// The desired settings of one port.
    pub fn port(&self, port: u8) -> Option<&DesiredPort> {
        self.ports.iter().find(|p| p.port == port)
    }

    /// The node the operator says is cabled to `port`.
    pub fn node_of_port(&self, port: u8) -> Option<u8> {
        self.port(port).and_then(|p| p.node)
    }

    /// Everything that is wrong with this document.
    pub fn validate(&self) -> Vec<String> {
        let mut errs: Vec<String> = Vec::new();
        if let Some(vlans) = &self.vlans {
            for v in vlans {
                if v.id == 0 || v.id > 4094 {
                    errs.push(format!("switch.desired.vlans: {} is not a VLAN id", v.id));
                }
                if v.name.chars().count() > 14 {
                    errs.push(format!(
                        "switch.desired.vlans: name \"{}\" is longer than 14 characters",
                        v.name
                    ));
                }
            }
        }
        for p in &self.ports {
            if !is_port(p.port) {
                errs.push(format!(
                    "switch.desired.ports: {} is not a port (1..={PORT_COUNT})",
                    p.port
                ));
                continue;
            }
            match p.mode {
                Some(VlanMode::Access) if p.vlan.is_none() => {
                    errs.push(format!(
                        "switch.desired.ports[{}]: an access port needs \"vlan\"",
                        p.port
                    ));
                }
                Some(VlanMode::Trunk) if p.native.is_none() => {
                    errs.push(format!(
                        "switch.desired.ports[{}]: a trunk port needs \"native\"",
                        p.port
                    ));
                }
                _ => {}
            }
            if let Some(speed) = p.speed
                && speed.impossible_on(p.port)
            {
                errs.push(format!(
                    "switch.desired.ports[{}]: port {} has no {speed}",
                    p.port, p.port
                ));
            }
            if let Some(node) = p.node
                && !crate::is_node(node)
            {
                errs.push(format!(
                    "switch.desired.ports[{}]: {node} is not a node id",
                    p.port
                ));
            }
        }
        if let Some(size) = self.jumbo
            && size != 0
            && jumbo_form_value(size).is_none()
        {
            let offered: Vec<String> = JUMBO_SIZES.iter().map(|(b, _)| b.to_string()).collect();
            errs.push(format!(
                "switch.desired.jumbo: {size} is not one of 0 (off), {}",
                offered.join(", ")
            ));
        }
        if let Some(storm) = &self.storm
            && parse_port_list(&storm.ports).is_none()
        {
            errs.push(format!(
                "switch.desired.storm.ports: \"{}\" is not a port list",
                storm.ports
            ));
        }
        if let Some(ip) = &self.ip
            && ip.mode == DesiredIpMode::Static
        {
            if parse_cidr(&ip.addr).is_none() {
                errs.push(format!(
                    "switch.desired.ip.addr: \"{}\" is not an address/prefix",
                    ip.addr
                ));
            }
            if parse_ipv4(&ip.gw).is_none() {
                errs.push(format!(
                    "switch.desired.ip.gw: \"{}\" is not an address",
                    ip.gw
                ));
            }
        }
        errs
    }
}

/// `"1-4,9"` to `[1, 2, 3, 4, 9]`. `None` when it is not a port list or
/// names a port the switch does not have.
pub fn parse_port_list(spec: &str) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        match part.split_once('-') {
            Some((a, b)) => {
                let from: u8 = a.trim().parse().ok()?;
                let to: u8 = b.trim().parse().ok()?;
                if from > to {
                    return None;
                }
                for port in from..=to {
                    out.push(port);
                }
            }
            None => out.push(part.parse().ok()?),
        }
    }
    if out.is_empty() || out.iter().any(|p| !is_port(*p)) {
        return None;
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// An IPv4 address in dotted form.
pub fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut parts = s.trim().split('.');
    for slot in out.iter_mut() {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some(out)
}

/// `"10.0.202.5/24"` to the address and the prefix length.
pub fn parse_cidr(s: &str) -> Option<([u8; 4], u8)> {
    let (addr, prefix) = s.trim().split_once('/')?;
    let bits: u8 = prefix.trim().parse().ok()?;
    if bits == 0 || bits > 32 {
        return None;
    }
    Some((parse_ipv4(addr)?, bits))
}

/// A prefix length as a dotted netmask, which is what `ip.cgi` wants.
pub fn netmask_of_prefix(bits: u8) -> String {
    let mask: u32 = if bits >= 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - bits)
    };
    let o = mask.to_be_bytes();
    format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3])
}

/// An address as the switch prints it.
pub fn ipv4_string(a: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", a[0], a[1], a[2], a[3])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_lists_and_addresses_parse() {
        assert_eq!(parse_port_list("1-4,9"), Some(alloc::vec![1, 2, 3, 4, 9]));
        assert_eq!(parse_port_list("9"), Some(alloc::vec![9]));
        assert_eq!(parse_port_list("1-8"), Some((1u8..=8).collect()));
        assert_eq!(parse_port_list("0-3"), None, "there is no port 0");
        assert_eq!(parse_port_list("1-10"), None, "there is no port 10");
        assert_eq!(parse_port_list("4-1"), None);
        assert_eq!(parse_port_list(""), None);
        assert_eq!(parse_cidr("10.0.202.5/24"), Some(([10, 0, 202, 5], 24)));
        assert_eq!(parse_cidr("10.0.202.5"), None);
        assert_eq!(parse_cidr("10.0.202.5/0"), None);
        assert_eq!(netmask_of_prefix(24), "255.255.255.0");
        assert_eq!(netmask_of_prefix(22), "255.255.252.0");
        assert_eq!(parse_ipv4("256.1.1.1"), None);
        assert_eq!(parse_ipv4("1.1.1"), None);
    }

    #[test]
    fn speeds_know_which_port_can_do_them() {
        assert!(Speed::G10.impossible_on(1));
        assert!(!Speed::G10.impossible_on(SFP_PORT));
        assert!(Speed::M10.impossible_on(SFP_PORT));
        assert_eq!(Speed::parse("2.5G"), Some(Speed::G2P5));
        assert_eq!(Speed::parse("2500M"), Some(Speed::G2P5));
        assert_eq!(Speed::parse("40G"), None);
    }

    #[test]
    fn macs_compare_whatever_the_separator() {
        assert!(same_mac("1c:2a:a3:24:26:7e", "1C:2A:A3:24:26:7E"));
        assert!(same_mac("1c-2a-a3-24-26-7e", "1C:2A:A3:24:26:7E"));
        assert!(!same_mac("", "1C:2A:A3:24:26:7E"));
        assert!(!same_mac("1c:2a:a3:24:26:7f", "1C:2A:A3:24:26:7E"));
    }

    #[test]
    fn a_desired_document_is_validated() {
        let mut d = SwitchDesired {
            vlans: Some(alloc::vec![DesiredVlan {
                id: 4095,
                name: String::from("much too long a name"),
            }]),
            ports: alloc::vec![DesiredPort {
                port: 1,
                mode: Some(VlanMode::Access),
                speed: Some(Speed::G10),
                ..DesiredPort::default()
            }],
            jumbo: Some(1234),
            ..SwitchDesired::default()
        };
        let errs = d.validate();
        assert_eq!(errs.len(), 5, "{errs:?}");

        d = SwitchDesired {
            vlans: Some(alloc::vec![DesiredVlan {
                id: 300,
                name: String::from("storage"),
            }]),
            ports: alloc::vec![DesiredPort {
                port: 7,
                mode: Some(VlanMode::Access),
                vlan: Some(300),
                ..DesiredPort::default()
            }],
            jumbo: Some(9216),
            loop_mode: Some(LoopMode::Loop),
            ip: Some(DesiredIp {
                mode: DesiredIpMode::Static,
                addr: String::from("10.0.202.5/24"),
                gw: String::from("10.0.202.1"),
            }),
            storm: Some(DesiredStorm {
                ports: String::from("1-8"),
                bcast_kbps: Some(10_000),
                ..DesiredStorm::default()
            }),
        };
        assert!(d.validate().is_empty(), "{:?}", d.validate());
        assert!(!d.is_empty());
        assert!(SwitchDesired::default().is_empty());
    }
}
