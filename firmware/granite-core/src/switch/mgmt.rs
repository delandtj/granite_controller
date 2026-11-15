//! The management VLAN of a block, and who is allowed in it (ADR 0006
//! component 25).
//!
//! The switch has no management VLAN setting: its CPU, and with it the
//! web UI the controller talks to, answers in VLAN 1. So VLAN 1 is the
//! management VLAN of a block, and the rule is that only the uplink and
//! the controller's own port carry it. A node port never does: a node
//! that can reach the switch's web UI can reconfigure the block it
//! lives in.
//!
//! What the ToR does with the uplink's untagged traffic - mapping it
//! into its own management VLAN with the port's PVID - is the ToR's
//! business and nothing here knows about it.
//!
//! Two halves, both pure:
//!
//!   - [`desired_carries_mgmt`], used by
//!     [`super::model::SwitchDesired::validate`]: a `desired.ports`
//!     entry that names a `node` and puts that port in VLAN 1 is a
//!     configuration error. The `node` label is the operator's own
//!     statement that the port is a node port.
//!   - [`exposed`], used by the worker: once the controller has found
//!     its own port, the ports of the unit that carry VLAN 1 and are
//!     neither the uplink nor the own port. Report only, like drift -
//!     nothing in here plans a write, and the planner does not know
//!     this module exists (a revert must be able to put VLAN 1 back on
//!     every port, which is what a factory-reset unit looks like).

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;

use super::model::{DesiredPort, PortVlan, SwitchState, VlanMode};
use super::plan::Protected;
use super::profile::Layout;

/// The management VLAN of a block: what the switch's CPU answers in.
pub const MGMT_VLAN: u16 = 1;

/// True when a port, as the switch reports it, carries the management
/// VLAN: an access port in VLAN 1, or a trunk with VLAN 1 as its native
/// VLAN or in its allowed list.
///
/// The mode decides which columns count. `vlan.cgi?page=port_based`
/// prints `-` in the access column of a trunk and leaves the allowed
/// list of an access port empty, so reading all three unconditionally
/// would only invent members.
pub fn port_carries_mgmt(actual: &PortVlan) -> bool {
    match actual.mode {
        VlanMode::Access => actual.access == Some(MGMT_VLAN),
        VlanMode::Trunk => {
            actual.native == Some(MGMT_VLAN) || actual.allowed.contains(&MGMT_VLAN)
        }
    }
}

/// True when a `desired.ports` entry puts its port in the management
/// VLAN.
///
/// Only what the entry names counts: a desired document is allowed to
/// be partial ("absent means do not manage"), so an entry that says
/// nothing about VLANs says nothing about VLAN 1 either. An entry
/// without a mode is judged on every VLAN field it does name, since
/// there is no mode to pick a column by.
pub fn desired_carries_mgmt(want: &DesiredPort) -> bool {
    match want.mode {
        Some(VlanMode::Access) => want.vlan == Some(MGMT_VLAN),
        Some(VlanMode::Trunk) => {
            want.native == Some(MGMT_VLAN) || want.allowed.contains(&MGMT_VLAN)
        }
        None => {
            want.vlan == Some(MGMT_VLAN)
                || want.native == Some(MGMT_VLAN)
                || want.allowed.contains(&MGMT_VLAN)
        }
    }
}

/// The ports of this unit that carry the management VLAN and should
/// not, ascending.
///
/// `None` means "no telling", and nothing is reported then: either the
/// controller has not found its own MAC in the switch's table (`own` is
/// `None`), in which case there is no saying which port to exempt, or
/// `vlan.cgi?page=port_based` has not been read yet.
///
/// The exempt pair is [`Protected`]'s, so "the uplink and the
/// controller's own port" has one definition in the image. An uplink
/// the unit has not got is dropped there, which is what makes it count
/// as no uplink declared (ADR 0006 component 24).
pub fn exposed(
    layout: &Layout,
    state: &SwitchState,
    uplink: Option<u8>,
    own: Option<u8>,
) -> Option<Vec<u8>> {
    own?;
    if state.port_vlans.is_empty() {
        return None;
    }
    let exempt = Protected::new(layout, uplink, own);
    let mut out: Vec<u8> = Vec::new();
    for port in 1..=layout.ports {
        if exempt.ports.contains(&port) {
            continue;
        }
        if state.port_vlan(port).is_some_and(port_carries_mgmt) {
            out.push(port);
        }
    }
    Some(out)
}

/// A port list as an event detail: `"1,2,3"`, plain and ascending.
///
/// Not the range form `desired.storm.ports` is written in: an event
/// detail is read by a human and parsed by nothing.
pub fn port_list(ports: &[u8]) -> String {
    let names: Vec<String> = ports.iter().map(|p| p.to_string()).collect();
    names.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::switch::model::MacEntry;

    fn trunk(port: u8, native: u16, allowed: &[u16]) -> PortVlan {
        PortVlan {
            port,
            mode: VlanMode::Trunk,
            access: None,
            native: Some(native),
            allowed: allowed.to_vec(),
        }
    }

    fn access(port: u8, vid: u16) -> PortVlan {
        PortVlan {
            port,
            mode: VlanMode::Access,
            access: Some(vid),
            native: None,
            allowed: Vec::new(),
        }
    }

    #[test]
    fn carrying_the_management_vlan_is_a_matter_of_the_mode() {
        assert!(port_carries_mgmt(&access(1, 1)));
        assert!(!port_carries_mgmt(&access(1, 100)));
        assert!(port_carries_mgmt(&trunk(9, 1, &[1, 100])));
        assert!(port_carries_mgmt(&trunk(9, 100, &[1, 100])));
        assert!(!port_carries_mgmt(&trunk(9, 100, &[100, 200])));

        // The allowed list of an access port is not read: the page
        // leaves it empty, and a stray entry is not a membership.
        let mut odd = access(1, 100);
        odd.allowed = alloc::vec![1];
        assert!(!port_carries_mgmt(&odd));
    }

    #[test]
    fn a_desired_port_is_judged_on_what_it_names() {
        assert!(desired_carries_mgmt(&DesiredPort {
            port: 1,
            mode: Some(VlanMode::Access),
            vlan: Some(1),
            ..DesiredPort::default()
        }));
        assert!(!desired_carries_mgmt(&DesiredPort {
            port: 1,
            mode: Some(VlanMode::Access),
            vlan: Some(100),
            ..DesiredPort::default()
        }));
        assert!(desired_carries_mgmt(&DesiredPort {
            port: 9,
            mode: Some(VlanMode::Trunk),
            native: Some(1),
            ..DesiredPort::default()
        }));
        assert!(desired_carries_mgmt(&DesiredPort {
            port: 9,
            mode: Some(VlanMode::Trunk),
            native: Some(100),
            allowed: alloc::vec![1, 100],
            ..DesiredPort::default()
        }));
        assert!(!desired_carries_mgmt(&DesiredPort {
            port: 9,
            mode: Some(VlanMode::Trunk),
            native: Some(100),
            allowed: alloc::vec![100, 200],
            ..DesiredPort::default()
        }));

        // An entry that only sets the admin state says nothing about
        // VLAN 1, whatever the port is in today.
        assert!(!desired_carries_mgmt(&DesiredPort {
            port: 1,
            enabled: Some(true),
            ..DesiredPort::default()
        }));
        // No mode: every VLAN field it does name counts.
        assert!(desired_carries_mgmt(&DesiredPort {
            port: 1,
            vlan: Some(1),
            ..DesiredPort::default()
        }));
        assert!(desired_carries_mgmt(&DesiredPort {
            port: 9,
            allowed: alloc::vec![1],
            ..DesiredPort::default()
        }));
    }

    #[test]
    fn nothing_is_reported_before_the_own_port_and_the_page_are_known() {
        let layout = Layout::widest();
        let read = SwitchState {
            port_vlans: alloc::vec![trunk(1, 1, &[1]), trunk(9, 1, &[1])],
            ..SwitchState::default()
        };
        assert_eq!(
            exposed(layout, &read, Some(9), None),
            None,
            "the own port is not known, so there is no port to exempt"
        );
        assert_eq!(
            exposed(layout, &SwitchState::default(), Some(9), Some(1)),
            None,
            "the port VLAN page was not read"
        );
    }

    #[test]
    fn the_uplink_and_the_own_port_are_the_only_ones_allowed_in() {
        let layout = Layout::widest();
        let mut state = SwitchState {
            port_vlans: (1..=9).map(|p| trunk(p, 1, &[1])).collect(),
            macs: alloc::vec![MacEntry {
                mac: String::from("52:54:00:44:C5:3E"),
                is_static: false,
                port: Some(1),
                vlan: 1,
            }],
            ..SwitchState::default()
        };
        // A fresh unit: every port a trunk with native 1.
        assert_eq!(
            exposed(layout, &state, Some(9), Some(1)),
            Some(alloc::vec![2, 3, 4, 5, 6, 7, 8]),
            "ascending, without the uplink and the own port"
        );
        assert_eq!(port_list(&[2, 3, 4]), "2,3,4");

        // The fix: the node ports become access ports in a data VLAN.
        for port in 2..=8 {
            state.port_vlans[usize::from(port) - 1] = access(port, 100);
        }
        assert_eq!(
            exposed(layout, &state, Some(9), Some(1)),
            Some(Vec::new()),
            "closed"
        );
        assert_eq!(port_list(&[]), "");

        // An uplink the unit has not got is no uplink at all, so it
        // exempts nothing and the port that does carry VLAN 1 shows up.
        state.port_vlans[7] = trunk(8, 1, &[1]);
        assert_eq!(
            exposed(layout, &state, Some(99), Some(1)),
            Some(alloc::vec![8, 9])
        );
    }
}
