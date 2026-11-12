//! The switch as the rest of the firmware sees it: the summary that
//! goes into the [`crate::observed::Observed`] snapshot and, from
//! there, into the retained `state` payload (ADR 0006 component 21).
//!
//! It is deliberately small and comparable: `state` is republished on
//! change, so nothing in here may move on its own. The counters do move
//! and go to their own topic instead ([`super::counters`]).

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

use super::model::{Speed, SwitchDesired, SwitchState, VlanMode};

fn is_false(b: &bool) -> bool {
    !*b
}

/// One port inside `state.switch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchPortObs {
    /// Port number, 1-based.
    pub port: u8,
    /// Admin state.
    pub enabled: bool,
    /// Link state.
    pub link: bool,
    /// Negotiated speed while the link is up, the configured one
    /// otherwise.
    pub speed: String,
    /// `access` or `trunk`.
    pub vlan_mode: String,
    /// The access VLAN, or the native VLAN of a trunk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vlan: Option<u16>,
    /// The node the configuration says is cabled here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<u8>,
    /// MACs seen on this port, as far as the table was walked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub macs: Vec<String>,
}

/// `state.switch`: the switch summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwitchObs {
    /// False after three failed polls in a row.
    pub reachable: bool,
    /// Firmware version string, empty until `info.cgi` was read.
    pub fw: String,
    /// Seconds since the switch booted.
    pub uptime_s: u32,
    /// Management MAC.
    pub mac: String,
    /// The ports, in order.
    pub ports: Vec<SwitchPortObs>,
    /// What differs from the desired state, empty when nothing does.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drift: Vec<String>,
    /// Monotonic milliseconds of the last reconcile that applied
    /// something; 0 when none has.
    #[serde(skip_serializing_if = "is_zero")]
    pub last_reconcile: u64,
    /// True while the switch's MAC does not match `switch.mac`: the
    /// controller then talks to nothing but `info.cgi`.
    #[serde(skip_serializing_if = "is_false")]
    pub mismatch: bool,
    /// True while the firmware version is not one the writes were
    /// tested against.
    #[serde(skip_serializing_if = "is_false")]
    pub fw_unknown: bool,
    /// True while the controller writes to the switch at all
    /// (`switch.manage`).
    #[serde(skip_serializing_if = "is_false")]
    pub managed: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl SwitchObs {
    /// The summary of what was read.
    ///
    /// `desired` only supplies the `node` labels, which are the
    /// operator's statement about cabling; nothing here is written.
    pub fn of_state(state: &SwitchState, desired: &SwitchDesired) -> Self {
        let info = state.info.clone().unwrap_or_default();
        let mut ports: Vec<SwitchPortObs> = Vec::new();
        for port in 1..=super::model::PORT_COUNT {
            let cfg = state.port(port);
            let stats = state.stats_port(port);
            let vlan = state.port_vlan(port);
            if cfg.is_none() && stats.is_none() && vlan.is_none() {
                continue;
            }
            let link = stats.is_some_and(|s| s.link);
            let speed = match cfg {
                Some(c) if link => c.speed_actual,
                Some(c) => c.speed,
                None => Speed::Auto,
            };
            ports.push(SwitchPortObs {
                port,
                enabled: cfg.map(|c| c.enabled).or(stats.map(|s| s.enabled)).unwrap_or(false),
                link,
                speed: alloc::string::String::from(speed.as_str()),
                vlan_mode: alloc::string::String::from(
                    vlan.map_or(VlanMode::Access, |v| v.mode).as_str(),
                ),
                vlan: vlan.and_then(|v| match v.mode {
                    VlanMode::Access => v.access,
                    VlanMode::Trunk => v.native,
                }),
                node: desired.node_of_port(port),
                macs: state.macs_on(port),
            });
        }
        SwitchObs {
            reachable: true,
            fw: info.firmware,
            uptime_s: info.uptime_s,
            mac: info.mac,
            ports,
            drift: Vec::new(),
            last_reconcile: 0,
            mismatch: false,
            fw_unknown: false,
            managed: false,
        }
    }

    /// The summary of a switch that is configured but not answering.
    /// Everything read earlier is kept, so a consumer still sees which
    /// node sat on which port while the switch is away.
    pub fn unreachable(previous: Option<&SwitchObs>) -> Self {
        let mut out = previous.cloned().unwrap_or_default();
        out.reachable = false;
        out
    }

    /// One port, if the switch reported it.
    pub fn port(&self, port: u8) -> Option<&SwitchPortObs> {
        self.ports.iter().find(|p| p.port == port)
    }

    /// The port a node is cabled to, as the configuration says.
    pub fn port_of_node(&self, node: u8) -> Option<u8> {
        self.ports
            .iter()
            .find(|p| p.node == Some(node))
            .map(|p| p.port)
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{DesiredPort, PortCfg, PortStats, PortVlan};
    use super::*;

    #[test]
    fn the_summary_reports_the_negotiated_speed_of_a_live_port() {
        let state = SwitchState {
            ports: alloc::vec![PortCfg {
                port: 9,
                enabled: true,
                duplex: super::super::model::Duplex::Auto,
                duplex_actual: super::super::model::Duplex::Full,
                speed: Speed::Auto,
                speed_actual: Speed::G10,
                flow: false,
                flow_actual: false,
            }],
            stats: alloc::vec![PortStats {
                port: 9,
                enabled: true,
                link: true,
                ..PortStats::default()
            }],
            port_vlans: alloc::vec![PortVlan {
                port: 9,
                mode: VlanMode::Trunk,
                access: None,
                native: Some(1),
                allowed: alloc::vec![1, 100],
            }],
            macs: alloc::vec![super::super::model::MacEntry {
                mac: String::from("52:54:00:44:C5:3E"),
                is_static: false,
                port: Some(9),
                vlan: 1,
            }],
            ..SwitchState::default()
        };
        let desired = SwitchDesired {
            ports: alloc::vec![DesiredPort {
                port: 9,
                node: Some(4),
                ..DesiredPort::default()
            }],
            ..SwitchDesired::default()
        };
        let obs = SwitchObs::of_state(&state, &desired);
        assert_eq!(obs.ports.len(), 1, "only the port that was read");
        let port = obs.port(9).expect("port 9");
        assert!(port.link && port.enabled);
        assert_eq!(port.speed, "10G", "the negotiated speed, not \"auto\"");
        assert_eq!(port.vlan_mode, "trunk");
        assert_eq!(port.vlan, Some(1), "a trunk reports its native VLAN");
        assert_eq!(port.node, Some(4));
        assert_eq!(port.macs.len(), 1);
        assert_eq!(obs.port_of_node(4), Some(9));

        let gone = SwitchObs::unreachable(Some(&obs));
        assert!(!gone.reachable);
        assert_eq!(gone.ports.len(), 1, "what was read is kept");
    }
}
