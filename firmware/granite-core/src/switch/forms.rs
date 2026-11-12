//! Every write to the switch, as the form it posts (ADR 0006 component
//! 15).
//!
//! The rules the switch's CGI imposes are encoded here, once, so the
//! firmware's client never builds a form itself:
//!
//! - ports are numbered from 0 in form fields (`port0` is Port 1),
//! - several forms need **every** field on every submit, and an empty
//!   rate or storm field means "unlimited", so the caller has to pass
//!   the current value of what it does not want to change
//!   (read-modify-write, `docs/sodola.md`),
//! - the isolation form repeats the same key for every port, so a form
//!   is a `Vec` of pairs and not a map,
//! - a trunk always carries its native VLAN, and the UI forces VLAN 1
//!   as well.
//!
//! [`encode`] is the only place that percent-encodes, and it is tested
//! against the handful of characters a VLAN name or a password can hold.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::model::{Duplex, LoopMode, Speed, VlanMode, jumbo_form_value, normalise_mac};

/// One form: ordered key/value pairs, because keys repeat.
pub type Form = Vec<(String, String)>;

fn pair(key: &str, value: impl Into<String>) -> (String, String) {
    (String::from(key), value.into())
}

/// The `portN` checkbox fields for a set of 1-based ports.
fn port_fields(ports: &[u8]) -> Form {
    ports
        .iter()
        .map(|p| pair(&format!("port{}", p.saturating_sub(1)), "on"))
        .collect()
}

/// `application/x-www-form-urlencoded` body of a form.
pub fn encode(form: &Form) -> String {
    let mut out = String::new();
    for (key, value) in form {
        if !out.is_empty() {
            out.push('&');
        }
        percent(&mut out, key);
        out.push('=');
        percent(&mut out, value);
    }
    out
}

/// Percent-encode one field, the way a browser does.
fn percent(out: &mut String, s: &str) {
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push('+'),
            other => {
                out.push('%');
                out.push(hex(other >> 4));
                out.push(hex(other & 0x0f));
            }
        }
    }
}

const fn hex(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        _ => (b'A' + n - 10) as char,
    }
}

// ---------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------

/// `login.cgi`: the POST that makes the switch accept the cookie.
///
/// `token` is [`super::md5::session_token`], which is also the cookie
/// value, and the switch wants it in the `Response` field.
pub fn login(user: &str, password: &str, token: &str) -> Form {
    alloc::vec![
        pair("username", user),
        pair("password", password),
        pair("Response", token),
        pair("language", "EN"),
    ]
}

/// `save.cgi`: write the running configuration to flash.
pub fn save() -> Form {
    alloc::vec![pair("cmd", "save")]
}

/// `reboot.cgi`: restart, dropping everything unsaved.
pub fn reboot() -> Form {
    alloc::vec![pair("cmd", "reboot")]
}

/// `reset.cgi`: factory default.
pub fn factory_reset() -> Form {
    alloc::vec![pair("cmd", "factory_default")]
}

// ---------------------------------------------------------------------
// VLANs
// ---------------------------------------------------------------------

/// `vlan.cgi?page=static`: add a VLAN.
pub fn vlan_add(vid: u16, name: &str) -> Form {
    alloc::vec![pair("vid", vid.to_string()), pair("name", name)]
}

/// `vlan.cgi?page=getRmvVlanEntry`: delete VLANs. The checkbox is
/// `remove_<vid>`.
pub fn vlan_remove(vids: &[u16]) -> Form {
    vids.iter()
        .map(|vid| pair(&format!("remove_{vid}"), "on"))
        .collect()
}

/// `vlan.cgi?page=port_based`: make ports untagged members of one VLAN.
pub fn port_access(ports: &[u8], vid: u16) -> Form {
    let mut form = port_fields(ports);
    form.push(pair("vlan_type", VlanMode::Access.form_value()));
    form.push(pair("vlan_access", vid.to_string()));
    form
}

/// `vlan.cgi?page=port_based`: make ports trunks.
///
/// The native VLAN and VLAN 1 are always in the list: a trunk carries
/// its native VLAN by definition, and the web UI checks VLAN 1 itself,
/// so leaving it out would mean "the form said something else than the
/// UI would have".
pub fn port_trunk(ports: &[u8], native: u16, allowed: &[u16]) -> Form {
    let mut vids: Vec<u16> = allowed.to_vec();
    vids.push(native);
    vids.push(1);
    vids.sort_unstable();
    vids.dedup();
    let mut form = port_fields(ports);
    form.push(pair("vlan_type", VlanMode::Trunk.form_value()));
    form.push(pair("vlan_trunk", native.to_string()));
    for vid in vids {
        form.push(pair(&format!("VLAN{vid}"), "on"));
    }
    form
}

// ---------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------

/// `port.cgi`: one port's admin state, speed, duplex and flow control.
///
/// Every field goes on every submit, and the write applies about a
/// second later: a second port write inside that window is silently
/// dropped, which is why the planner marks these ops with a settle
/// time.
pub fn port_config(port: u8, enabled: bool, speed: Speed, duplex: Duplex, flow: bool) -> Form {
    let mut form = port_fields(&[port]);
    form.push(pair("state", if enabled { "1" } else { "0" }));
    form.push(pair("duplex", duplex.form_value()));
    form.push(pair("speed", speed.form_value()));
    form.push(pair("flowctrl", if flow { "1" } else { "0" }));
    form.push(pair("cmd", "port"));
    form
}

/// The duplex a speed implies: the switch refuses a fixed speed with
/// auto duplex and auto speed with a fixed duplex
/// (`docs/sodola.md`, "Quirks"). `current` is used when it fits.
pub fn duplex_for(speed: Speed, current: Duplex) -> Duplex {
    match speed {
        Speed::Auto => Duplex::Auto,
        _ if current == Duplex::Auto => Duplex::Full,
        _ => current,
    }
}

// ---------------------------------------------------------------------
// Forwarding: jumbo frames, storm control, rate limits
// ---------------------------------------------------------------------

/// `fwd.cgi?page=jumboframe`: the enable toggle. An empty form is what
/// the UI posts to turn it off, and that also resets the size to 1522.
pub fn jumbo_toggle(on: bool) -> Form {
    if on {
        alloc::vec![pair("enable_jumbo", "on")]
    } else {
        Vec::new()
    }
}

/// `fwd.cgi?page=jumboframe`: the frame size. `None` when the switch
/// does not offer that size.
pub fn jumbo_size(size: u32) -> Option<Form> {
    let value = jumbo_form_value(size)?;
    Some(alloc::vec![
        pair("jumboframe", value),
        pair("cmd", "jumboframe"),
    ])
}

/// A rate field: `None` is the empty string, which the switch reads as
/// unlimited.
fn rate_value(rate: Option<u32>) -> String {
    rate.map(|r| r.to_string()).unwrap_or_default()
}

/// `fwd.cgi?page=storm_ctrl`: one port's four storm limits \[Kbit/s\].
///
/// All four fields are sent every time, so the caller passes the
/// current value of the ones it is not changing.
pub fn storm(
    port: u8,
    bcast: Option<u32>,
    mcast: Option<u32>,
    unknown_ucast: Option<u32>,
    unknown_mcast: Option<u32>,
) -> Form {
    let mut form = port_fields(&[port]);
    form.push(pair("bc_rate", rate_value(bcast)));
    form.push(pair("mc_rate", rate_value(mcast)));
    form.push(pair("uuc_rate", rate_value(unknown_ucast)));
    form.push(pair("umc_rate", rate_value(unknown_mcast)));
    form.push(pair("cmd", "storm"));
    form
}

/// `port.cgi?page=bwctrl`: one port's rate limits \[Kbit/s\].
pub fn rate_limit(port: u8, ingress: Option<u32>, egress: Option<u32>) -> Form {
    let mut form = port_fields(&[port]);
    form.push(pair("in_rate", rate_value(ingress)));
    form.push(pair("out_rate", rate_value(egress)));
    form.push(pair("cmd", "bandwidthcontrol"));
    form
}

/// The highest storm or rate limit a port accepts \[Kbit/s\].
pub const fn rate_max(port: u8) -> u32 {
    if port == super::model::SFP_PORT {
        10_000_000
    } else {
        2_500_000
    }
}

/// The lowest storm or rate limit a port accepts \[Kbit/s\].
pub const RATE_MIN: u32 = 16;

// ---------------------------------------------------------------------
// Loop prevention and the management address
// ---------------------------------------------------------------------

/// `loop.cgi`: the loop prevention mode. The two timers are sent along
/// because the form needs every field; a mode change rewrites p2p on
/// every port, which the ADR accepts.
pub fn loop_mode(mode: LoopMode, interval_s: u32, recover_s: u32) -> Form {
    alloc::vec![
        pair("func_type", mode.form_value()),
        pair("interval_time", interval_s.to_string()),
        pair("recover_time", recover_s.to_string()),
        pair("cmd", "loop"),
    ]
}

/// `ip.cgi`: a static management address.
pub fn ip_static(ip: &str, netmask: &str, gateway: &str) -> Form {
    alloc::vec![
        pair("dhcp_state", "0"),
        pair("ip", ip),
        pair("netmask", netmask),
        pair("gateway", gateway),
        pair("cmd", "ip"),
    ]
}

/// `ip.cgi`: DHCP.
pub fn ip_dhcp() -> Form {
    alloc::vec![pair("dhcp_state", "1"), pair("cmd", "ip")]
}

// ---------------------------------------------------------------------
// MAC table
// ---------------------------------------------------------------------

/// Page sizes `mac.cgi?page=fwd_tbl` offers, as form values.
pub const MAC_PERPAGE_30: &str = "3";

/// `mac.cgi?page=fwd_tbl`: set the page size to 30 and show page 1.
///
/// The paging and the search filter are server-side state shared with
/// any browser on the web UI, so a walk does a plain GET of the page
/// first to drop a stuck filter and then posts this.
pub fn mac_perpage() -> Form {
    alloc::vec![pair("cmd", "perpage"), pair("perpage", MAC_PERPAGE_30)]
}

/// `mac.cgi?page=fwd_tbl`: go to one page, 30 rows at a time.
pub fn mac_goto(page: u32) -> Form {
    alloc::vec![
        pair("cmd", "goto"),
        pair("pageidx", page.to_string()),
        pair("perpage", MAC_PERPAGE_30),
    ]
}

/// `mac.cgi?page=static`: add a static entry.
pub fn mac_static_add(mac: &str, vlan: u16, port: u8) -> Form {
    alloc::vec![
        pair("mac", normalise_mac(mac)),
        pair("vlan", vlan.to_string()),
        pair("src", port.saturating_sub(1).to_string()),
        pair("cmd", "macstatic"),
    ]
}

/// `port.cgi?page=stats`: clear the counters.
pub fn stats_clear() -> Form {
    alloc::vec![pair("submit", "Clear"), pair("cmd", "stats")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_encode_the_way_a_browser_would() {
        let form = alloc::vec![
            pair("vid", "300"),
            pair("name", "node net/1 & 2"),
            pair("empty", ""),
        ];
        assert_eq!(
            encode(&form),
            "vid=300&name=node+net%2F1+%26+2&empty=",
            "space becomes +, the rest percent-encoded, an empty value stays empty"
        );
    }

    #[test]
    fn port_fields_are_zero_based() {
        assert_eq!(
            encode(&port_access(&[1, 4, 9], 100)),
            "port0=on&port3=on&port8=on&vlan_type=0&vlan_access=100"
        );
    }

    #[test]
    fn a_trunk_always_carries_its_native_vlan_and_vlan_1() {
        assert_eq!(
            encode(&port_trunk(&[9], 100, &[200])),
            "port8=on&vlan_type=1&vlan_trunk=100&VLAN1=on&VLAN100=on&VLAN200=on"
        );
    }

    #[test]
    fn a_port_write_sends_every_field() {
        assert_eq!(
            encode(&port_config(7, false, Speed::G1, Duplex::Full, true)),
            "port6=on&state=0&duplex=1&speed=2&flowctrl=1&cmd=port"
        );
        assert_eq!(duplex_for(Speed::Auto, Duplex::Full), Duplex::Auto);
        assert_eq!(duplex_for(Speed::G1, Duplex::Auto), Duplex::Full);
        assert_eq!(duplex_for(Speed::G1, Duplex::Half), Duplex::Half);
    }

    #[test]
    fn an_empty_rate_field_is_unlimited() {
        assert_eq!(
            encode(&storm(1, Some(10_000), None, Some(50_000), None)),
            "port0=on&bc_rate=10000&mc_rate=&uuc_rate=50000&umc_rate=&cmd=storm"
        );
        assert_eq!(
            encode(&rate_limit(9, None, None)),
            "port8=on&in_rate=&out_rate=&cmd=bandwidthcontrol"
        );
        assert_eq!(rate_max(9), 10_000_000);
        assert_eq!(rate_max(1), 2_500_000);
    }

    #[test]
    fn vlan_removal_names_the_checkbox_after_the_vlan() {
        assert_eq!(encode(&vlan_remove(&[100, 200])), "remove_100=on&remove_200=on");
    }

    #[test]
    fn jumbo_off_is_an_empty_form() {
        assert_eq!(encode(&jumbo_toggle(false)), "");
        assert_eq!(encode(&jumbo_toggle(true)), "enable_jumbo=on");
        assert_eq!(
            encode(&jumbo_size(9216).expect("9216 is offered")),
            "jumboframe=4&cmd=jumboframe"
        );
        assert!(jumbo_size(1234).is_none());
    }

    #[test]
    fn the_login_form_carries_the_cookie_token() {
        let token = super::super::md5::session_token("admin", "admin");
        assert_eq!(
            encode(&login("admin", "admin", &token)),
            format!("username=admin&password=admin&Response={token}&language=EN")
        );
    }
}
