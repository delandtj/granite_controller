//! One parser per CGI page, `&str -> Result<T, ParseError>` (ADR 0006
//! component 15).
//!
//! Every parser recognises its page before it reads it: a page whose
//! marker table is not there, or whose row shape is not the one these
//! parsers were written against, is rejected rather than guessed at. The
//! firmware turns that into "unsupported firmware", which is the whole
//! point of the fingerprint check in component 16.
//!
//! The fixtures these are tested against live in
//! `granite-core/tests/fixtures/sodola-v200.1.8/`, captured from a live
//! unit with `tools/sodola.py get`.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use super::model::{
    Duplex, IpCfg, Jumbo, JUMBO_SIZES, LoopCfg, LoopMode, MacEntry, MacPage, PortCfg, PortStats,
    PortVlan, RatePort, Speed, StormPort, SwitchInfo, Vlan, VlanMode, VlanTable, normalise_mac,
};
use super::scan;

/// Why a page could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The page is the login redirect: the switch forgot the session.
    LoginRedirect,
    /// The page is not the one that was asked for, or not a shape these
    /// parsers know.
    Unrecognised {
        /// Which page was expected.
        page: &'static str,
        /// What is missing.
        detail: String,
    },
}

impl ParseError {
    fn unrecognised(page: &'static str, detail: impl Into<String>) -> Self {
        ParseError::Unrecognised {
            page,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::LoginRedirect => f.write_str("the switch redirected to login.cgi"),
            ParseError::Unrecognised { page, detail } => {
                write!(f, "{page} is not a page this firmware knows: {detail}")
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

/// What every parser does first: reject a login redirect and an empty
/// body (which is what the CGI answers without a `Referer`).
fn usable(html: &str, page: &'static str) -> Result<(), ParseError> {
    if scan::is_login_redirect(html) {
        return Err(ParseError::LoginRedirect);
    }
    if html.trim().is_empty() {
        return Err(ParseError::unrecognised(page, "empty body"));
    }
    Ok(())
}

/// `Port 7` to `7`, for the first cell of every per-port table.
fn port_cell(cell: &str) -> Option<u8> {
    let rest = cell.trim().strip_prefix("Port")?;
    rest.trim().parse().ok()
}

/// `Enable` / `Disable`.
fn enable_cell(cell: &str) -> Option<bool> {
    match cell.trim() {
        "Enable" | "Enabled" => Some(true),
        "Disable" | "Disabled" => Some(false),
        _ => None,
    }
}

/// `Link Up` / `Link Down`.
fn link_cell(cell: &str) -> Option<bool> {
    match cell.trim() {
        "Link Up" | "Up" => Some(true),
        "Link Down" | "Down" => Some(false),
        _ => None,
    }
}

/// `On` / `Off`.
fn on_cell(cell: &str) -> Option<bool> {
    match cell.trim() {
        "On" => Some(true),
        "Off" => Some(false),
        _ => None,
    }
}

/// A rate cell: a number in Kbit/s, or `Unlimited` / `Off` for none.
fn rate_cell(cell: &str) -> Option<Option<u32>> {
    let text = cell.trim();
    let first = text.split_whitespace().next().unwrap_or("");
    match first {
        "Unlimited" | "Off" | "" | "-" => Some(None),
        n => n.parse().ok().map(Some),
    }
}

/// `0Day0Hour33Minute6Second` to seconds.
fn uptime_seconds(text: &str) -> Option<u32> {
    let mut total: u32 = 0;
    let mut number: Option<u32> = None;
    let mut unit = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_digit() {
            if !unit.is_empty() {
                total += scale(&unit)? * number.take().unwrap_or(0);
                unit.clear();
            }
            number = Some(number.unwrap_or(0) * 10 + u32::from(ch as u8 - b'0'));
        } else if ch.is_ascii_alphabetic() {
            unit.push(ch);
        }
    }
    if !unit.is_empty() {
        total += scale(&unit)? * number.take().unwrap_or(0);
    }
    // A trailing number with no unit is not an uptime string.
    if number.is_some() { None } else { Some(total) }
}

fn scale(unit: &str) -> Option<u32> {
    match unit {
        "Day" | "Days" => Some(86_400),
        "Hour" | "Hours" => Some(3_600),
        "Minute" | "Minutes" | "Min" => Some(60),
        "Second" | "Seconds" | "Sec" => Some(1),
        _ => None,
    }
}

/// The per-port rows of a table, keyed by port: every row whose first
/// cell is `Port N` and which has `columns` cells in total.
fn port_rows(html: &str, columns: usize) -> Vec<(u8, Vec<String>)> {
    scan::rows(html)
        .into_iter()
        .filter(|r| r.len() == columns)
        .filter_map(|r| port_cell(&r[0]).map(|port| (port, r)))
        .filter(|(port, _)| super::model::is_port(*port))
        .collect()
}

// ---------------------------------------------------------------------
// info.cgi
// ---------------------------------------------------------------------

/// `info.cgi`: the device facts and the link state of every port.
///
/// The link state comes back as [`PortStats`] with only `port`,
/// `link` and the negotiated speed filled in, so the worker has one
/// shape for "what the link is doing" whichever page it came from.
pub fn info(html: &str) -> Result<(SwitchInfo, Vec<PortLink>), ParseError> {
    const PAGE: &str = "info.cgi";
    usable(html, PAGE)?;
    if !html.contains("Firmware Version") {
        return Err(ParseError::unrecognised(PAGE, "no \"Firmware Version\" row"));
    }
    let mut out = SwitchInfo {
        name: scan::attr(html, "devName").unwrap_or_default(),
        ..SwitchInfo::default()
    };
    // The device table is `<th>label</th><td>value</td>` pairs, two to
    // a row, so it is read as pairs rather than as rows.
    for (label, value) in label_value_pairs(html) {
        match label.trim_end_matches(':').trim() {
            "Device Model" => out.model = value,
            "Firmware Version" => out.firmware = value,
            "MAC Address" => out.mac = normalise_mac(&value),
            "IP Address" => out.ip = value,
            "Netmask" => out.netmask = value,
            "Sys Uptime" => {
                out.uptime_s = uptime_seconds(&value).ok_or_else(|| {
                    ParseError::unrecognised(PAGE, format!("uptime \"{value}\""))
                })?;
            }
            _ => {}
        }
    }
    if out.firmware.is_empty() || out.mac.is_empty() {
        return Err(ParseError::unrecognised(
            PAGE,
            "no firmware version or MAC address",
        ));
    }

    let mut links: Vec<PortLink> = Vec::new();
    for (port, row) in port_rows(html, 5) {
        let link = link_cell(&row[1])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("link cell \"{}\"", row[1])))?;
        links.push(PortLink {
            port,
            link,
            duplex: Duplex::parse(&row[2]).unwrap_or_default(),
            speed: Speed::parse(&row[3]).unwrap_or_default(),
            flow: on_cell(&row[4]).unwrap_or(false),
        });
    }
    if links.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no port status rows"));
    }
    Ok((out, links))
}

/// What `info.cgi` says one port's link is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortLink {
    /// Port number, 1-based.
    pub port: u8,
    /// Link state.
    pub link: bool,
    /// Negotiated duplex.
    pub duplex: Duplex,
    /// Negotiated speed.
    pub speed: Speed,
    /// Flow control in force.
    pub flow: bool,
}

/// Every `<th>label</th>` immediately followed by a `<td>value</td>`.
fn label_value_pairs(html: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find("<th") {
        rest = &rest[at..];
        let Some(open_end) = rest.find('>') else { break };
        let body = &rest[open_end + 1..];
        // The firmware closes these cells with `</td>`, not `</th>`.
        let end = match (body.find("</td"), body.find("</th")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };
        let label = scan::text(&body[..end]);
        let after = &body[end..];
        let value = match after.find("<td") {
            Some(td_at) => {
                let td = &after[td_at..];
                match (td.find('>'), td.find("</td")) {
                    (Some(s), Some(e)) if e > s => scan::text(&td[s + 1..e]),
                    _ => String::new(),
                }
            }
            None => String::new(),
        };
        if !label.is_empty() {
            out.push((label, value));
        }
        rest = after;
    }
    out
}

// ---------------------------------------------------------------------
// port.cgi
// ---------------------------------------------------------------------

/// `port.cgi`: the per-port admin configuration.
pub fn port_config(html: &str) -> Result<Vec<PortCfg>, ParseError> {
    const PAGE: &str = "port.cgi";
    usable(html, PAGE)?;
    if !scan::has_field(html, "port0") || !html.contains("flowctrl") {
        return Err(ParseError::unrecognised(PAGE, "no port configuration form"));
    }
    let mut out: Vec<PortCfg> = Vec::new();
    for (port, row) in port_rows(html, 8) {
        let enabled = enable_cell(&row[1])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("state \"{}\"", row[1])))?;
        let duplex = Duplex::parse(&row[2])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("duplex \"{}\"", row[2])))?;
        let speed = Speed::parse(&row[4])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("speed \"{}\"", row[4])))?;
        out.push(PortCfg {
            port,
            enabled,
            duplex,
            duplex_actual: Duplex::parse(&row[3]).unwrap_or_default(),
            speed,
            speed_actual: Speed::parse(&row[5]).unwrap_or_default(),
            flow: on_cell(&row[6])
                .ok_or_else(|| ParseError::unrecognised(PAGE, format!("flow \"{}\"", row[6])))?,
            flow_actual: on_cell(&row[7]).unwrap_or(false),
        });
    }
    if out.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no port rows"));
    }
    Ok(out)
}

/// `port.cgi?page=stats`: link state and the good packet/byte counters.
///
/// Each counter is printed as two u32 halves joined by `-`
/// (`hi-lo`), which is how the switch reports a 64-bit number.
pub fn stats(html: &str) -> Result<Vec<PortStats>, ParseError> {
    const PAGE: &str = "port.cgi?page=stats";
    usable(html, PAGE)?;
    if !html.contains("txgood") {
        return Err(ParseError::unrecognised(PAGE, "no counter columns"));
    }
    let mut out: Vec<PortStats> = Vec::new();
    for (port, row) in port_rows(html, 7) {
        let enabled = enable_cell(&row[1])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("state \"{}\"", row[1])))?;
        let link = link_cell(&row[2])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("link \"{}\"", row[2])))?;
        let mut counters = [0u64; 4];
        for (slot, cell) in counters.iter_mut().zip(row[3..7].iter()) {
            *slot = counter(cell)
                .ok_or_else(|| ParseError::unrecognised(PAGE, format!("counter \"{cell}\"")))?;
        }
        out.push(PortStats {
            port,
            enabled,
            link,
            tx_packets: counters[0],
            rx_packets: counters[1],
            tx_bytes: counters[2],
            rx_bytes: counters[3],
        });
    }
    if out.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no counter rows"));
    }
    Ok(out)
}

/// `hi-lo` to the 64-bit number it stands for.
fn counter(cell: &str) -> Option<u64> {
    let (hi, lo) = cell.trim().split_once('-')?;
    let hi: u64 = hi.trim().parse().ok()?;
    let lo: u64 = lo.trim().parse().ok()?;
    Some((hi << 32) + lo)
}

// ---------------------------------------------------------------------
// vlan.cgi
// ---------------------------------------------------------------------

/// `vlan.cgi?page=static`: the VLAN table.
pub fn vlans(html: &str) -> Result<VlanTable, ParseError> {
    const PAGE: &str = "vlan.cgi?page=static";
    usable(html, PAGE)?;
    if !html.contains("getRmvVlanEntry") {
        return Err(ParseError::unrecognised(PAGE, "no VLAN table form"));
    }
    let table = scan::after(html, "getRmvVlanEntry");
    let mut vlans: Vec<Vlan> = Vec::new();
    for row in scan::rows(table) {
        // Select, No., VLAN ID, VLAN Name. VLAN 1 has no checkbox, so
        // its first cell is empty, but the shape is the same.
        if row.len() != 4 {
            continue;
        }
        let Ok(id) = row[2].trim().parse::<u16>() else {
            continue;
        };
        vlans.push(Vlan {
            id,
            name: row[3].clone(),
        });
    }
    if vlans.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no VLAN rows"));
    }
    Ok(VlanTable {
        enabled: scan::checked(html, "enable_vlan"),
        vlans,
    })
}

/// `vlan.cgi?page=port_based`: per-port VLAN membership.
pub fn port_vlans(html: &str) -> Result<Vec<PortVlan>, ParseError> {
    const PAGE: &str = "vlan.cgi?page=port_based";
    usable(html, PAGE)?;
    if !html.contains("vlan_type") || !html.contains("showdiv") {
        return Err(ParseError::unrecognised(PAGE, "no port VLAN table"));
    }
    let table = scan::after(html, "showdiv");
    let mut out: Vec<PortVlan> = Vec::new();
    for (port, row) in port_rows(table, 5) {
        let mode = VlanMode::parse(&row[1])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("vlan type \"{}\"", row[1])))?;
        let access = row[2].trim().parse::<u16>().ok();
        let native = row[3].trim().parse::<u16>().ok();
        let mut allowed: Vec<u16> = Vec::new();
        for item in row[4].split([',', ' ']) {
            if let Ok(vid) = item.trim().parse::<u16>() {
                allowed.push(vid);
            }
        }
        allowed.sort_unstable();
        allowed.dedup();
        out.push(PortVlan {
            port,
            mode,
            access,
            native,
            allowed,
        });
    }
    if out.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no port VLAN rows"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// fwd.cgi
// ---------------------------------------------------------------------

/// `fwd.cgi?page=jumboframe`.
pub fn jumbo(html: &str) -> Result<Jumbo, ParseError> {
    const PAGE: &str = "fwd.cgi?page=jumboframe";
    usable(html, PAGE)?;
    if !scan::has_field(html, "enable_jumbo") {
        return Err(ParseError::unrecognised(PAGE, "no jumbo frame toggle"));
    }
    let enabled = scan::checked(html, "enable_jumbo");
    let selected = scan::selected(html, "jumboframe")
        .ok_or_else(|| ParseError::unrecognised(PAGE, "no frame size selected"))?;
    let size = JUMBO_SIZES
        .iter()
        .find(|(_, value)| *value == selected.value)
        .map(|(bytes, _)| *bytes)
        .ok_or_else(|| {
            ParseError::unrecognised(PAGE, format!("frame size value \"{}\"", selected.value))
        })?;
    Ok(Jumbo { enabled, size })
}

/// `fwd.cgi?page=storm_ctrl`.
pub fn storm(html: &str) -> Result<Vec<StormPort>, ParseError> {
    const PAGE: &str = "fwd.cgi?page=storm_ctrl";
    usable(html, PAGE)?;
    if !scan::has_field(html, "bc_rate") {
        return Err(ParseError::unrecognised(PAGE, "no storm control form"));
    }
    let mut out: Vec<StormPort> = Vec::new();
    for (port, row) in port_rows(html, 5) {
        let mut rates = [None; 4];
        for (slot, cell) in rates.iter_mut().zip(row[1..5].iter()) {
            *slot = rate_cell(cell)
                .ok_or_else(|| ParseError::unrecognised(PAGE, format!("rate \"{cell}\"")))?;
        }
        out.push(StormPort {
            port,
            bcast: rates[0],
            mcast: rates[1],
            unknown_ucast: rates[2],
            unknown_mcast: rates[3],
        });
    }
    if out.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no storm control rows"));
    }
    Ok(out)
}

/// `port.cgi?page=bw_ctrl`: the ingress and egress rate limits.
pub fn rate_limits(html: &str) -> Result<Vec<RatePort>, ParseError> {
    const PAGE: &str = "port.cgi?page=bw_ctrl";
    usable(html, PAGE)?;
    if !scan::has_field(html, "in_rate") {
        return Err(ParseError::unrecognised(PAGE, "no rate limit form"));
    }
    let mut out: Vec<RatePort> = Vec::new();
    for (port, row) in port_rows(html, 3) {
        let ingress = rate_cell(&row[1])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("ingress \"{}\"", row[1])))?;
        let egress = rate_cell(&row[2])
            .ok_or_else(|| ParseError::unrecognised(PAGE, format!("egress \"{}\"", row[2])))?;
        out.push(RatePort {
            port,
            ingress,
            egress,
        });
    }
    if out.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no rate limit rows"));
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// loop.cgi and ip.cgi
// ---------------------------------------------------------------------

/// `loop.cgi`: the loop prevention mode and its two timers.
pub fn loop_cfg(html: &str) -> Result<LoopCfg, ParseError> {
    const PAGE: &str = "loop.cgi";
    usable(html, PAGE)?;
    let selected = scan::selected(html, "func_type")
        .ok_or_else(|| ParseError::unrecognised(PAGE, "no loop function selected"))?;
    let mode = LoopMode::from_form_value(&selected.value).ok_or_else(|| {
        ParseError::unrecognised(PAGE, format!("func_type \"{}\"", selected.value))
    })?;
    let number = |field: &str| -> u32 {
        scan::attr(html, field)
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0)
    };
    Ok(LoopCfg {
        mode,
        interval_s: number("interval_time"),
        recover_s: number("recover_time"),
    })
}

/// `ip.cgi`: the management address.
///
/// The page only puts the three values in the DOM from JavaScript, in
/// single-quoted attributes, and the labels it writes for DHCP mode
/// carry the same numbers; the inputs are read either way.
pub fn ip(html: &str) -> Result<IpCfg, ParseError> {
    const PAGE: &str = "ip.cgi";
    usable(html, PAGE)?;
    let selected = scan::selected(html, "dhcp_state")
        .ok_or_else(|| ParseError::unrecognised(PAGE, "no IP address mode selected"))?;
    let dhcp = match selected.value.trim() {
        "0" => false,
        "1" => true,
        other => {
            return Err(ParseError::unrecognised(
                PAGE,
                format!("dhcp_state \"{other}\""),
            ));
        }
    };
    let ip = scan::attr(html, "ip").unwrap_or_default();
    let netmask = scan::attr(html, "netmask").unwrap_or_default();
    let gateway = scan::attr(html, "gateway").unwrap_or_default();
    if ip.is_empty() || netmask.is_empty() {
        return Err(ParseError::unrecognised(PAGE, "no address or netmask"));
    }
    Ok(IpCfg {
        dhcp,
        ip,
        netmask,
        gateway,
    })
}

// ---------------------------------------------------------------------
// mac.cgi
// ---------------------------------------------------------------------

/// `mac.cgi?page=fwd_tbl`: one page of the forwarding table.
pub fn mac_page(html: &str) -> Result<MacPage, ParseError> {
    const PAGE: &str = "mac.cgi?page=fwd_tbl";
    usable(html, PAGE)?;
    if !html.contains("totalpage") {
        return Err(ParseError::unrecognised(PAGE, "no MAC table paging"));
    }
    let mut entries: Vec<MacEntry> = Vec::new();
    for row in scan::rows(html) {
        if row.len() != 4 || !looks_like_mac(&row[0]) {
            continue;
        }
        let Ok(vlan) = row[3].trim().parse::<u16>() else {
            continue;
        };
        let port = match row[2].trim() {
            "CPU" => None,
            n => match n.parse::<u8>() {
                Ok(p) if super::model::is_port(p) => Some(p),
                _ => continue,
            },
        };
        entries.push(MacEntry {
            mac: normalise_mac(&row[0]),
            is_static: row[1].trim().eq_ignore_ascii_case("static"),
            port,
            vlan,
        });
    }
    Ok(MacPage {
        entries,
        total: between(html, "Total ", " Items").and_then(|t| t.trim().parse().ok()).unwrap_or(0),
        pages: between(html, "id='totalpage'>", "<")
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(1),
    })
}

/// The text between two markers.
fn between<'a>(html: &'a str, from: &str, to: &str) -> Option<&'a str> {
    let at = html.find(from)? + from.len();
    let end = html[at..].find(to)? + at;
    Some(&html[at..end])
}

/// `AA:BB:CC:DD:EE:FF` in either case, which is the only thing the MAC
/// table's first column ever holds.
fn looks_like_mac(cell: &str) -> bool {
    let cell = cell.trim();
    if cell.len() != 17 {
        return false;
    }
    cell.split(':').count() == 6
        && cell
            .split(':')
            .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_strings_become_seconds() {
        assert_eq!(uptime_seconds("0Day0Hour33Minute6Second"), Some(33 * 60 + 6));
        assert_eq!(uptime_seconds("2Day3Hour4Minute5Second"), Some(
            2 * 86_400 + 3 * 3_600 + 4 * 60 + 5
        ));
        assert_eq!(uptime_seconds("nonsense"), None, "an unknown unit");
        assert_eq!(uptime_seconds("12"), None, "a number with no unit");
    }

    #[test]
    fn counters_are_two_u32_halves() {
        assert_eq!(counter("0-2616"), Some(2_616));
        assert_eq!(counter("1-0"), Some(1 << 32));
        assert_eq!(counter("nope"), None);
    }

    #[test]
    fn a_login_redirect_is_rejected_by_every_parser() {
        let page = "<script>window.top.location.replace(\"/login.cgi\");</script>";
        assert_eq!(info(page).unwrap_err(), ParseError::LoginRedirect);
        assert_eq!(stats(page).unwrap_err(), ParseError::LoginRedirect);
        assert_eq!(vlans(page).unwrap_err(), ParseError::LoginRedirect);
        assert_eq!(ip(page).unwrap_err(), ParseError::LoginRedirect);
    }

    #[test]
    fn a_page_that_is_not_the_expected_one_is_refused() {
        let other = "<html><body><table><tr><td>Port 1</td></tr></table></body></html>";
        assert!(matches!(
            info(other),
            Err(ParseError::Unrecognised { .. })
        ));
        assert!(matches!(
            port_config(other),
            Err(ParseError::Unrecognised { .. })
        ));
        assert!(matches!(
            jumbo(""),
            Err(ParseError::Unrecognised { .. })
        ));
    }
}
