//! The switch parsers against the pages of a real unit (ADR 0006
//! component 15, "tested on the host against HTML captured from the
//! real switch").
//!
//! The fixtures in `tests/fixtures/sodola-v200.1.8/` were captured with
//! read-only GETs from an SL902-SWTGW218AS on firmware V200.1.8
//! (`tools/sodola.py --host ... get '<page>'`), 2026-11-12. Its state at
//! the time: VLANs 1, 100 "data" and 200 "backend"; all nine ports
//! trunk, native 1, allowed 1; jumbo on at 12000; loop prevention,
//! storm control and rate limits off; only port 9 (SFP+) has link, at
//! 10G; the management address static on 192.168.64.241/24.

use granite_core::switch::model::{
    DesiredIp, DesiredIpMode, DesiredPort, DesiredVlan, Duplex, LoopMode, Speed, SwitchDesired,
    SwitchState, VlanMode, firmware_known, same_mac,
};
use granite_core::switch::parse::{self, ParseError};
use granite_core::switch::plan::{self, OpKind, Protected};
use granite_core::switch::{counters, forms};

/// The unit the fixtures came from.
const SWITCH_MAC: &str = "1C:2A:A3:24:26:7E";

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/sodola-v200.1.8/{name}.html",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Everything the poll loop reads, parsed into one state.
fn read_everything() -> SwitchState {
    let (info, links) = parse::info(&fixture("info")).expect("info.cgi");
    let mut state = SwitchState {
        info: Some(info),
        ports: parse::port_config(&fixture("port")).expect("port.cgi"),
        vlans: Some(parse::vlans(&fixture("vlan-static")).expect("vlan static")),
        port_vlans: parse::port_vlans(&fixture("vlan-port-based")).expect("vlan port_based"),
        jumbo: Some(parse::jumbo(&fixture("jumbo")).expect("jumbo")),
        storm: parse::storm(&fixture("storm")).expect("storm"),
        rate: parse::rate_limits(&fixture("bw-ctrl")).expect("bw_ctrl"),
        loop_cfg: Some(parse::loop_cfg(&fixture("loop")).expect("loop")),
        ip: Some(parse::ip(&fixture("ip")).expect("ip")),
        stats: parse::stats(&fixture("stats")).expect("stats"),
        macs: parse::mac_page(&fixture("mac-fwd-tbl"))
            .expect("mac table")
            .entries,
    };
    // The link state from info.cgi agrees with the stats page, which is
    // what lets the worker use either.
    for link in links {
        let stats = state
            .stats
            .iter()
            .find(|s| s.port == link.port)
            .expect("every port is on the stats page");
        assert_eq!(stats.link, link.link, "port {} link", link.port);
    }
    state.macs.sort_by(|a, b| a.mac.cmp(&b.mac));
    state
}

#[test]
fn info_cgi_gives_the_device_facts_and_the_link_state() {
    let (info, links) = parse::info(&fixture("info")).expect("info.cgi parses");
    assert_eq!(info.name, "SL902-SWTGW218AS");
    assert_eq!(info.model, "SL902-SWTGW218AS");
    assert_eq!(info.firmware, "V200.1.8");
    assert!(firmware_known(&info.firmware), "the tested firmware");
    assert_eq!(info.mac, SWITCH_MAC);
    assert!(same_mac(&info.mac, "1c:2a:a3:24:26:7e"));
    assert_eq!(info.ip, "192.168.64.241");
    assert_eq!(info.netmask, "255.255.255.0");
    assert_eq!(info.uptime_s, 33 * 60 + 6, "0Day0Hour33Minute6Second");

    assert_eq!(links.len(), 9);
    assert!(links[..8].iter().all(|l| !l.link), "ports 1-8 are down");
    let sfp = links[8];
    assert_eq!(sfp.port, 9);
    assert!(sfp.link);
    assert_eq!(sfp.speed, Speed::G10);
    assert_eq!(sfp.duplex, Duplex::Full);
    assert!(!sfp.flow);
}

#[test]
fn port_cgi_gives_the_admin_configuration_of_every_port() {
    let ports = parse::port_config(&fixture("port")).expect("port.cgi parses");
    assert_eq!(ports.len(), 9);
    assert!(ports.iter().all(|p| p.enabled), "nothing is disabled");
    assert!(
        ports.iter().all(|p| p.speed == Speed::Auto && p.duplex == Duplex::Auto),
        "every port is left on auto-negotiation"
    );
    assert!(ports.iter().all(|p| !p.flow));
    let sfp = ports.iter().find(|p| p.port == 9).expect("port 9");
    assert_eq!(sfp.speed_actual, Speed::G10, "the SFP+ negotiated 10G");
    assert_eq!(sfp.duplex_actual, Duplex::Full);
}

#[test]
fn the_stats_page_gives_link_and_the_64_bit_counters() {
    let stats = parse::stats(&fixture("stats")).expect("stats parses");
    assert_eq!(stats.len(), 9);
    let quiet = stats.iter().find(|s| s.port == 1).expect("port 1");
    assert!(!quiet.link);
    assert_eq!(quiet.tx_packets, 0);
    let sfp = stats.iter().find(|s| s.port == 9).expect("port 9");
    assert!(sfp.link && sfp.enabled);
    assert_eq!(sfp.tx_packets, 2_616);
    assert_eq!(sfp.rx_packets, 163_836);
    assert_eq!(sfp.tx_bytes, 1_778_810);
    assert_eq!(sfp.rx_bytes, 28_295_531);
}

#[test]
fn counters_become_rates_between_two_reads() {
    let first = parse::stats(&fixture("stats")).expect("stats");
    let mut second = first.clone();
    for port in second.iter_mut().filter(|p| p.port == 9) {
        port.tx_packets += 600;
        port.tx_bytes += 60_000;
    }
    let rates = counters::rates(Some((&first, 1_000)), &second, 61_000);
    assert_eq!(rates.interval_ms, 60_000);
    let sfp = rates.ports.iter().find(|p| p.port == 9).expect("port 9");
    assert_eq!(sfp.tx_pps, Some(10), "600 packets in 60 s");
    assert_eq!(sfp.tx_bps, Some(8_000), "60000 bytes in 60 s");
    assert_eq!(sfp.rx_pps, Some(0));
    assert!(!counters::counters_went_backwards(&first, &second));

    // A switch reboot resets them, and that is not a negative rate.
    let reset: Vec<_> = first
        .iter()
        .map(|p| granite_core::switch::model::PortStats {
            tx_packets: 0,
            rx_packets: 0,
            tx_bytes: 0,
            rx_bytes: 0,
            ..*p
        })
        .collect();
    assert!(counters::counters_went_backwards(&first, &reset));
    let after = counters::rates(Some((&first, 1_000)), &reset, 61_000);
    assert!(
        after
            .ports
            .iter()
            .find(|p| p.port == 9)
            .expect("port 9")
            .tx_pps
            .is_none(),
        "the only port that had counted anything reports no rate"
    );
    assert_eq!(
        after
            .ports
            .iter()
            .find(|p| p.port == 1)
            .expect("port 1")
            .tx_pps,
        Some(0),
        "a port that was at zero before and after did not go backwards"
    );
}

#[test]
fn the_vlan_pages_give_the_table_and_the_per_port_membership() {
    let table = parse::vlans(&fixture("vlan-static")).expect("vlan static parses");
    assert!(table.enabled, "802.1Q is on");
    let ids: Vec<u16> = table.vlans.iter().map(|v| v.id).collect();
    assert_eq!(ids, [1, 100, 200]);
    assert_eq!(table.vlans[1].name, "data");
    assert_eq!(table.vlans[2].name, "backend");
    assert!(table.vlans[0].name.is_empty(), "VLAN 1 has no name");
    assert!(table.has(200) && !table.has(300));

    let ports = parse::port_vlans(&fixture("vlan-port-based")).expect("port_based parses");
    assert_eq!(ports.len(), 9);
    for port in &ports {
        assert_eq!(port.mode, VlanMode::Trunk, "port {}", port.port);
        assert_eq!(port.native, Some(1));
        assert_eq!(port.allowed, [1]);
        assert_eq!(port.access, None, "the access column is \"-\"");
    }
}

#[test]
fn the_remaining_pages_parse() {
    let jumbo = parse::jumbo(&fixture("jumbo")).expect("jumbo parses");
    assert!(jumbo.enabled);
    assert_eq!(jumbo.size, 12_000);

    let loops = parse::loop_cfg(&fixture("loop")).expect("loop parses");
    assert_eq!(loops.mode, LoopMode::Off);
    assert_eq!(loops.interval_s, 2);
    assert_eq!(loops.recover_s, 10);

    let ip = parse::ip(&fixture("ip")).expect("ip parses");
    assert!(!ip.dhcp);
    assert_eq!(ip.ip, "192.168.64.241");
    assert_eq!(ip.netmask, "255.255.255.0");
    assert_eq!(ip.gateway, "192.168.64.254");

    let storm = parse::storm(&fixture("storm")).expect("storm parses");
    assert_eq!(storm.len(), 9);
    assert!(
        storm
            .iter()
            .all(|s| s.bcast.is_none() && s.mcast.is_none() && s.unknown_ucast.is_none()),
        "storm control is off everywhere"
    );

    let rate = parse::rate_limits(&fixture("bw-ctrl")).expect("bw_ctrl parses");
    assert_eq!(rate.len(), 9);
    assert!(
        rate.iter().all(|r| r.ingress.is_none() && r.egress.is_none()),
        "\"Unlimited\" is no limit"
    );
}

#[test]
fn the_mac_table_page_gives_the_rows_and_the_paging() {
    let page = parse::mac_page(&fixture("mac-fwd-tbl")).expect("mac table parses");
    assert_eq!(page.total, 36, "\"Total 36 Items\"");
    assert_eq!(page.pages, 4, "ten rows to a page by default");
    assert_eq!(page.entries.len(), 10);
    // The switch's own MAC sits on the CPU port, not on a real one.
    let own = page
        .entries
        .iter()
        .find(|e| same_mac(&e.mac, SWITCH_MAC))
        .expect("the switch itself is in its table");
    assert_eq!(own.port, None, "CPU");
    assert_eq!(own.vlan, 1);
    assert!(
        page.entries.iter().filter(|e| e.port == Some(9)).count() == 9,
        "everything else is learned on the uplink"
    );
    assert!(page.entries.iter().all(|e| !e.is_static));
}

#[test]
fn a_page_from_the_wrong_url_is_refused_instead_of_guessed_at() {
    // Every parser gets every other page; none may accept one.
    let pages = [
        "info",
        "port",
        "stats",
        "vlan-static",
        "vlan-port-based",
        "jumbo",
        "loop",
        "ip",
        "storm",
        "bw-ctrl",
        "mac-fwd-tbl",
        "isolation",
        "lag",
        "stp-global",
        "stp-port",
    ];
    type Parser = (&'static str, fn(&str) -> bool);
    let parsers: [Parser; 10] = [
        ("info", |h| parse::info(h).is_ok()),
        ("port", |h| parse::port_config(h).is_ok()),
        ("stats", |h| parse::stats(h).is_ok()),
        ("vlan-static", |h| parse::vlans(h).is_ok()),
        ("vlan-port-based", |h| parse::port_vlans(h).is_ok()),
        ("jumbo", |h| parse::jumbo(h).is_ok()),
        ("loop", |h| parse::loop_cfg(h).is_ok()),
        ("ip", |h| parse::ip(h).is_ok()),
        ("storm", |h| parse::storm(h).is_ok()),
        ("bw-ctrl", |h| parse::rate_limits(h).is_ok()),
    ];
    for (name, parser) in parsers {
        for page in pages {
            let html = fixture(page);
            let accepted = parser(&html);
            assert_eq!(
                accepted,
                page == name,
                "the {name} parser on the {page} page"
            );
        }
    }
}

#[test]
fn the_login_redirect_is_told_apart_from_a_page() {
    let html = fixture("login-redirect");
    assert_eq!(parse::info(&html).unwrap_err(), ParseError::LoginRedirect);
    assert_eq!(parse::stats(&html).unwrap_err(), ParseError::LoginRedirect);
    assert_eq!(
        parse::mac_page(&html).unwrap_err(),
        ParseError::LoginRedirect
    );
    // An empty body is what the CGI answers without a Referer header.
    assert!(matches!(
        parse::info(""),
        Err(ParseError::Unrecognised { .. })
    ));
}

#[test]
fn the_state_read_from_the_fixtures_is_the_switch_the_notes_describe() {
    let state = read_everything();
    assert_eq!(state.firmware(), "V200.1.8");
    assert_eq!(state.port_of_mac(SWITCH_MAC), None, "the CPU port");
    assert_eq!(state.macs_on(9).len(), 9);
    assert_eq!(state.port(9).map(|p| p.enabled), Some(true));
    assert_eq!(state.stats_port(9).map(|s| s.link), Some(true));
    assert_eq!(state.port_vlan(3).map(|p| p.mode), Some(VlanMode::Trunk));
}

#[test]
fn the_adr_example_plans_in_the_right_order_against_the_real_state() {
    let state = read_everything();
    // ADR 0006: add VLAN 300 and make port 7 an access port in it.
    let desired = SwitchDesired {
        vlans: Some(vec![
            DesiredVlan {
                id: 1,
                name: String::new(),
            },
            DesiredVlan {
                id: 100,
                name: String::from("data"),
            },
            DesiredVlan {
                id: 200,
                name: String::from("backend"),
            },
            DesiredVlan {
                id: 300,
                name: String::from("storage"),
            },
        ]),
        ports: vec![DesiredPort {
            port: 7,
            mode: Some(VlanMode::Access),
            vlan: Some(300),
            node: Some(7),
            ..DesiredPort::default()
        }],
        ..SwitchDesired::default()
    };
    assert!(desired.validate().is_empty(), "{:?}", desired.validate());

    // The controller's own MAC is not on this switch (the fixtures come
    // from a bench unit), so protection falls back to "everything",
    // which is exactly what the ADR asks for.
    let blind = plan::plan(&state, &desired, &Protected::new(Some(9), None));
    assert!(
        blind.refusals.iter().any(|r| r.reason.contains("own MAC")),
        "{:?}",
        blind.refusals
    );
    assert_eq!(
        blind.ops.iter().filter(|op| op.kind == OpKind::PortVlan).count(),
        0,
        "no port moves while the controller cannot see itself"
    );

    // With the controller found on the uplink, port 7 is free to move.
    let found = Protected::new(Some(9), Some(9));
    let made = plan::plan(&state, &desired, &found);
    assert!(made.refusals.is_empty(), "{:?}", made.refusals);
    assert!(made.unread.is_empty());
    assert_eq!(
        made.summary(),
        vec![
            String::from("create VLAN 300 \"storage\""),
            String::from("port 7 access VLAN 300"),
        ]
    );
    assert_eq!(
        forms::encode(&made.ops[0].form),
        "vid=300&name=storage",
        "the VLAN is created first"
    );
    assert_eq!(
        forms::encode(&made.ops[1].form),
        "port6=on&vlan_type=0&vlan_access=300"
    );

    // The revert: port 7 back to the trunk it was, then VLAN 300 away.
    let mut after = state.clone();
    after
        .vlans
        .as_mut()
        .expect("read")
        .vlans
        .push(granite_core::switch::model::Vlan {
            id: 300,
            name: String::from("storage"),
        });
    if let Some(pv) = after.port_vlans.iter_mut().find(|p| p.port == 7) {
        pv.mode = VlanMode::Access;
        pv.access = Some(300);
        pv.native = None;
        pv.allowed = Vec::new();
    }
    let back = plan::undo(&after, &state, &desired);
    assert_eq!(
        back.summary(),
        vec![
            String::from("port 7 trunk native 1 allowed 1"),
            String::from("delete VLAN 300"),
        ]
    );
    assert_eq!(
        forms::encode(&back.ops[0].form),
        "port6=on&vlan_type=1&vlan_trunk=1&VLAN1=on"
    );
    assert_eq!(forms::encode(&back.ops[1].form), "remove_300=on");
}

#[test]
fn a_full_desired_document_plans_against_the_real_state() {
    let state = read_everything();
    let desired = SwitchDesired {
        ports: vec![
            DesiredPort {
                port: 1,
                mode: Some(VlanMode::Access),
                vlan: Some(100),
                enabled: Some(true),
                speed: Some(Speed::G2P5),
                node: Some(1),
                ..DesiredPort::default()
            },
            DesiredPort {
                port: 2,
                enabled: Some(false),
                ..DesiredPort::default()
            },
        ],
        jumbo: Some(9_216),
        loop_mode: Some(LoopMode::Loop),
        storm: Some(granite_core::switch::model::DesiredStorm {
            ports: String::from("1-8"),
            bcast_kbps: Some(10_000),
            unknown_ucast_kbps: Some(50_000),
            ..Default::default()
        }),
        ip: Some(DesiredIp {
            mode: DesiredIpMode::Static,
            addr: String::from("10.0.202.5/24"),
            gw: String::from("10.0.202.1"),
        }),
        ..SwitchDesired::default()
    };
    assert!(desired.validate().is_empty(), "{:?}", desired.validate());

    let made = plan::plan(&state, &desired, &Protected::new(Some(9), Some(9)));
    assert!(made.refusals.is_empty(), "{:?}", made.refusals);
    let kinds: Vec<OpKind> = made.ops.iter().map(|op| op.kind).collect();
    // Port VLAN first, then the middle band, the address last.
    assert_eq!(kinds[0], OpKind::PortVlan);
    assert_eq!(kinds[kinds.len() - 1], OpKind::Ip);
    assert!(kinds.iter().filter(|k| **k == OpKind::Storm).count() == 8);
    assert!(kinds.contains(&OpKind::Jumbo) && kinds.contains(&OpKind::Loop));
    // Port writes have to settle before the next write goes out.
    for op in made.ops.iter().filter(|op| op.kind == OpKind::PortConfig) {
        assert_eq!(op.settle_ms, plan::PORT_SETTLE_MS);
    }
    // The address change may never get an answer.
    assert!(made.ops.last().expect("ip op").may_timeout);
    // 2.5G on port 1 means a fixed duplex, as the switch demands.
    let speed_op = made
        .ops
        .iter()
        .find(|op| op.ports == vec![1] && op.kind == OpKind::PortConfig)
        .expect("port 1 speed");
    assert_eq!(
        forms::encode(&speed_op.form),
        "port0=on&state=1&duplex=1&speed=5&flowctrl=0&cmd=port"
    );
}
