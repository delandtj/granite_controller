//! The planner: what has to be written to make the switch match the
//! desired state, in an order that cannot cut the controller off halfway
//! (ADR 0006 components 15 and 18).
//!
//! [`plan`] is a pure function of the state that was read, the desired
//! document and the protected ports. It never talks to anything; the
//! worker carries the ops out one at a time and the same function run
//! backwards ([`undo`]) is what a revert applies.
//!
//! Ordering, which is the part that matters:
//!
//!   1. create VLANs - a port cannot be put in a VLAN that is not there,
//!   2. port VLAN changes,
//!   3. delete VLANs - only once no port refers to them,
//!   4. jumbo frames, storm control, rate limits, loop prevention,
//!   5. the management address, last, because it is the change that can
//!      end the conversation.
//!
//! Protection: an op that touches the uplink port, or the port the
//! controller's own MAC is seen on, is refused without `force`. While
//! the controller has not found its own MAC in the table, every port
//! counts as protected for VLAN and admin-state changes (ADR 0006,
//! "What an Expert Would Ask").

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::forms::{self, Form};
use super::model::{
    DesiredIpMode, DesiredPort, DesiredStorm, Duplex, LoopMode, Speed, SwitchDesired, SwitchState,
    VlanMode, netmask_of_prefix, parse_cidr, parse_port_list,
};

/// What an op is about, which is also what fixes its order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpKind {
    /// Create a VLAN.
    VlanCreate,
    /// Change a port's VLAN membership.
    PortVlan,
    /// Delete VLANs.
    VlanDelete,
    /// Change a port's admin state, speed or duplex.
    PortConfig,
    /// Jumbo frame setting.
    Jumbo,
    /// Storm control.
    Storm,
    /// Rate limits.
    Rate,
    /// Loop prevention.
    Loop,
    /// The management address.
    Ip,
}

impl OpKind {
    /// Name used in logs and events.
    pub const fn as_str(self) -> &'static str {
        match self {
            OpKind::VlanCreate => "vlan_create",
            OpKind::PortVlan => "port_vlan",
            OpKind::VlanDelete => "vlan_delete",
            OpKind::PortConfig => "port_config",
            OpKind::Jumbo => "jumbo",
            OpKind::Storm => "storm",
            OpKind::Rate => "rate",
            OpKind::Loop => "loop",
            OpKind::Ip => "ip",
        }
    }

    /// Where this op goes in the plan. Everything that is neither a
    /// VLAN nor the address applies in one middle band.
    pub const fn rank(self) -> u8 {
        match self {
            OpKind::VlanCreate => 1,
            OpKind::PortVlan => 2,
            OpKind::VlanDelete => 3,
            OpKind::PortConfig | OpKind::Jumbo | OpKind::Storm | OpKind::Rate | OpKind::Loop => 4,
            OpKind::Ip => 5,
        }
    }
}

/// One form POST, with what the client has to know about carrying it
/// out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    /// What it is about.
    pub kind: OpKind,
    /// CGI path, relative to the switch's root.
    pub path: String,
    /// The form to post.
    pub form: Form,
    /// Human-readable description, for the log and the event.
    pub detail: String,
    /// How long to wait after the POST before anything else is written
    /// \[ms\]. A port write applies about a second later and a second
    /// write inside that window is dropped.
    pub settle_ms: u32,
    /// True when no reply is the expected outcome: an STP or IP change
    /// can cut the answer off and still have applied.
    pub may_timeout: bool,
    /// The ports this op touches, for the protection check.
    pub ports: Vec<u8>,
}

/// Settle time after a port-configuration write \[ms\].
pub const PORT_SETTLE_MS: u32 = 2_000;

impl Op {
    fn new(kind: OpKind, path: &str, form: Form, detail: String) -> Self {
        Op {
            kind,
            path: String::from(path),
            form,
            detail,
            settle_ms: 0,
            may_timeout: false,
            ports: Vec::new(),
        }
    }

    fn on_ports(mut self, ports: &[u8]) -> Self {
        self.ports = ports.to_vec();
        self
    }

    fn settles(mut self, ms: u32) -> Self {
        self.settle_ms = ms;
        self
    }

    fn may_timeout(mut self) -> Self {
        self.may_timeout = true;
        self
    }
}

/// An op that was not planned, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What it would have been about.
    pub kind: OpKind,
    /// Why it was refused.
    pub reason: String,
}

/// The ports no plan may touch without `force`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Protected {
    /// The uplink and the controller's own port.
    pub ports: Vec<u8>,
    /// True while the controller has not found its own MAC in the
    /// table: then every port is protected.
    pub all: bool,
}

impl Protected {
    /// The protected set: the configured uplink, and the port the
    /// controller's own MAC was seen on.
    ///
    /// `own` is `None` when the MAC table has not been walked or the
    /// controller's MAC is not in it, which is what turns `all` on.
    pub fn new(uplink: Option<u8>, own: Option<u8>) -> Self {
        let mut ports: Vec<u8> = Vec::new();
        for port in [uplink, own].into_iter().flatten() {
            if super::model::is_port(port) && !ports.contains(&port) {
                ports.push(port);
            }
        }
        ports.sort_unstable();
        Protected {
            ports,
            all: own.is_none(),
        }
    }

    /// Nothing is protected: what `force` means.
    pub fn none() -> Self {
        Protected {
            ports: Vec::new(),
            all: false,
        }
    }

    /// True when `port` may not be touched.
    pub fn covers(&self, port: u8) -> bool {
        self.all || self.ports.contains(&port)
    }

    /// Why `port` is protected.
    fn why(&self, port: u8) -> String {
        if self.ports.contains(&port) {
            format!("port {port} is the uplink or the controller's own port")
        } else {
            String::from(
                "the controller has not found its own MAC in the switch's table, so every port \
                 is protected until it does",
            )
        }
    }
}

/// Everything one reconcile run would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// The ops, in the order they must be carried out.
    pub ops: Vec<Op>,
    /// What was left out, and why.
    pub refusals: Vec<Refusal>,
    /// Pages the desired state needs but that were not read, so nothing
    /// could be planned for them.
    pub unread: Vec<&'static str>,
}

impl Plan {
    /// True when the switch already matches the desired state.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// One line per op, for an event or a log line.
    pub fn summary(&self) -> Vec<String> {
        self.ops.iter().map(|op| op.detail.clone()).collect()
    }
}

/// The ops that bring `actual` to `desired`.
///
/// Keys the desired document does not name are left alone. A key whose
/// page was not read lands in [`Plan::unread`] instead of being guessed
/// at, so the worker can read it and plan again.
pub fn plan(actual: &SwitchState, desired: &SwitchDesired, protected: &Protected) -> Plan {
    let mut out = Plan::default();
    plan_vlans(actual, desired, &mut out);
    plan_port_vlans(actual, desired, protected, &mut out);
    plan_port_config(actual, desired, protected, &mut out);
    plan_jumbo(actual, desired, &mut out);
    plan_storm(actual, desired, &mut out);
    plan_loop(actual, desired, &mut out);
    plan_ip(actual, desired, &mut out);
    out.ops.sort_by_key(|op| op.kind.rank());
    out
}

/// The desired state that puts the switch back as `snapshot` had it,
/// restricted to the keys `desired` manages.
///
/// A revert is the same planner run from the current state towards the
/// snapshot (ADR 0006 component 18, "revert by undo"), so nothing new
/// has to be written to undo a change.
pub fn undo_desired(snapshot: &SwitchState, desired: &SwitchDesired) -> SwitchDesired {
    let mut back = SwitchDesired::default();
    if desired.vlans.is_some()
        && let Some(table) = &snapshot.vlans
    {
        back.vlans = Some(
            table
                .vlans
                .iter()
                .map(|v| super::model::DesiredVlan {
                    id: v.id,
                    name: v.name.clone(),
                })
                .collect(),
        );
    }
    for want in &desired.ports {
        let mut port = DesiredPort {
            port: want.port,
            node: want.node,
            ..DesiredPort::default()
        };
        if want.mode.is_some()
            && let Some(was) = snapshot.port_vlan(want.port)
        {
            port.mode = Some(was.mode);
            port.vlan = was.access;
            port.native = was.native;
            port.allowed = was.allowed.clone();
        }
        if let Some(was) = snapshot.port(want.port) {
            if want.enabled.is_some() {
                port.enabled = Some(was.enabled);
            }
            if want.speed.is_some() {
                port.speed = Some(was.speed);
            }
        }
        back.ports.push(port);
    }
    if desired.jumbo.is_some()
        && let Some(was) = snapshot.jumbo
    {
        back.jumbo = Some(if was.enabled { was.size } else { 0 });
    }
    if let Some(storm) = &desired.storm
        && let Some(ports) = parse_port_list(&storm.ports)
    {
        // The previous values differ per port, so the undo is one
        // single-port document per port that was touched.
        let mut undone: Vec<DesiredStorm> = Vec::new();
        for port in ports {
            if let Some(was) = snapshot.storm_port(port) {
                undone.push(DesiredStorm {
                    ports: port.to_string(),
                    bcast_kbps: Some(was.bcast.unwrap_or(0)),
                    mcast_kbps: Some(was.mcast.unwrap_or(0)),
                    unknown_ucast_kbps: Some(was.unknown_ucast.unwrap_or(0)),
                    unknown_mcast_kbps: Some(was.unknown_mcast.unwrap_or(0)),
                });
            }
        }
        // A document holds one storm block, so the ports that shared
        // the same previous values are folded into it and the rest
        // comes back on the next pass; in practice a block is one set
        // of values over a range, so this is the common case.
        back.storm = fold_storm(undone);
    }
    if desired.loop_mode.is_some()
        && let Some(was) = snapshot.loop_cfg
    {
        back.loop_mode = Some(was.mode);
    }
    if desired.ip.is_some()
        && let Some(was) = &snapshot.ip
    {
        back.ip = Some(super::model::DesiredIp {
            mode: if was.dhcp {
                DesiredIpMode::Dhcp
            } else {
                DesiredIpMode::Static
            },
            addr: format!("{}/{}", was.ip, prefix_of_netmask(&was.netmask)),
            gw: was.gateway.clone(),
        });
    }
    back
}

/// The plan that undoes `desired` on a switch that now reads `current`.
pub fn undo(current: &SwitchState, snapshot: &SwitchState, desired: &SwitchDesired) -> Plan {
    // An undo may have to touch the very ports the change touched,
    // including a protected one: the snapshot is where the switch was
    // when the controller could still reach it.
    plan(current, &undo_desired(snapshot, desired), &Protected::none())
}

/// Every difference between the desired state and what was read, as
/// text: the `switch_drift` event's payload. Protection does not come
/// into it, because nothing is written.
pub fn drift(actual: &SwitchState, desired: &SwitchDesired) -> Vec<String> {
    plan(actual, desired, &Protected::none()).summary()
}

/// Ports that share one set of storm values, folded back into one
/// block. `None` when the list is empty.
fn fold_storm(mut blocks: Vec<DesiredStorm>) -> Option<DesiredStorm> {
    let first = blocks.first().cloned()?;
    let same = |a: &DesiredStorm, b: &DesiredStorm| {
        a.bcast_kbps == b.bcast_kbps
            && a.mcast_kbps == b.mcast_kbps
            && a.unknown_ucast_kbps == b.unknown_ucast_kbps
            && a.unknown_mcast_kbps == b.unknown_mcast_kbps
    };
    let mut ports: Vec<String> = Vec::new();
    blocks.retain(|b| same(b, &first));
    for block in &blocks {
        ports.push(block.ports.clone());
    }
    Some(DesiredStorm {
        ports: ports.join(","),
        ..first
    })
}

/// A dotted netmask as a prefix length.
fn prefix_of_netmask(netmask: &str) -> u8 {
    super::model::parse_ipv4(netmask)
        .map(|o| u32::from_be_bytes(o).count_ones() as u8)
        .unwrap_or(24)
}

// ---------------------------------------------------------------------
// The per-key planners
// ---------------------------------------------------------------------

fn plan_vlans(actual: &SwitchState, desired: &SwitchDesired, out: &mut Plan) {
    let Some(want) = &desired.vlans else {
        return;
    };
    let Some(have) = &actual.vlans else {
        out.unread.push("vlan.cgi?page=static");
        return;
    };
    for vlan in want {
        if !have.has(vlan.id) {
            out.ops.push(Op::new(
                OpKind::VlanCreate,
                "vlan.cgi?page=static",
                forms::vlan_add(vlan.id, &vlan.name),
                format!("create VLAN {} \"{}\"", vlan.id, vlan.name),
            ));
        }
    }
    // VLAN 1 cannot be deleted and the CPU most likely sits in it, so
    // it is never planned for deletion whatever the document says.
    let doomed: Vec<u16> = have
        .vlans
        .iter()
        .map(|v| v.id)
        .filter(|id| *id != 1 && !want.iter().any(|w| w.id == *id))
        .collect();
    if !doomed.is_empty() {
        let names: Vec<String> = doomed.iter().map(|id| id.to_string()).collect();
        out.ops.push(Op::new(
            OpKind::VlanDelete,
            "vlan.cgi?page=getRmvVlanEntry",
            forms::vlan_remove(&doomed),
            format!("delete VLAN {}", names.join(", ")),
        ));
    }
}

fn plan_port_vlans(
    actual: &SwitchState,
    desired: &SwitchDesired,
    protected: &Protected,
    out: &mut Plan,
) {
    let wanted: Vec<&DesiredPort> = desired
        .ports
        .iter()
        .filter(|p| p.mode.is_some())
        .collect();
    if wanted.is_empty() {
        return;
    }
    if actual.port_vlans.is_empty() {
        out.unread.push("vlan.cgi?page=port_based");
        return;
    }
    for want in wanted {
        let Some(mode) = want.mode else { continue };
        let Some(have) = actual.port_vlan(want.port) else {
            out.refusals.push(Refusal {
                kind: OpKind::PortVlan,
                reason: format!("the switch has no port {}", want.port),
            });
            continue;
        };
        let (changed, op) = match mode {
            VlanMode::Access => {
                let Some(vid) = want.vlan else { continue };
                if let Some(missing) = missing_vlan(actual, desired, &[vid]) {
                    out.refusals.push(Refusal {
                        kind: OpKind::PortVlan,
                        reason: missing,
                    });
                    continue;
                }
                (
                    have.mode != VlanMode::Access || have.access != Some(vid),
                    Op::new(
                        OpKind::PortVlan,
                        "vlan.cgi?page=port_based",
                        forms::port_access(&[want.port], vid),
                        format!("port {} access VLAN {vid}", want.port),
                    ),
                )
            }
            VlanMode::Trunk => {
                let Some(native) = want.native else { continue };
                let mut vids: Vec<u16> = want.allowed.clone();
                vids.push(native);
                vids.push(1);
                vids.sort_unstable();
                vids.dedup();
                if let Some(missing) = missing_vlan(actual, desired, &vids) {
                    out.refusals.push(Refusal {
                        kind: OpKind::PortVlan,
                        reason: missing,
                    });
                    continue;
                }
                let names: Vec<String> = vids.iter().map(|v| v.to_string()).collect();
                (
                    have.mode != VlanMode::Trunk
                        || have.native != Some(native)
                        || have.allowed != vids,
                    Op::new(
                        OpKind::PortVlan,
                        "vlan.cgi?page=port_based",
                        forms::port_trunk(&[want.port], native, &want.allowed),
                        format!(
                            "port {} trunk native {native} allowed {}",
                            want.port,
                            names.join(",")
                        ),
                    ),
                )
            }
        };
        if !changed {
            continue;
        }
        if protected.covers(want.port) {
            out.refusals.push(Refusal {
                kind: OpKind::PortVlan,
                reason: format!(
                    "{}: {} needs \"force\"",
                    protected.why(want.port),
                    op.detail
                ),
            });
            continue;
        }
        out.ops.push(op.on_ports(&[want.port]));
    }
}

/// Why a port cannot be put in these VLANs, if it cannot: a VLAN that
/// is neither on the switch nor created by this very plan.
fn missing_vlan(actual: &SwitchState, desired: &SwitchDesired, vids: &[u16]) -> Option<String> {
    let have = actual.vlans.as_ref()?;
    for vid in vids {
        let created = desired
            .vlans
            .as_ref()
            .is_some_and(|list| list.iter().any(|v| v.id == *vid));
        if !have.has(*vid) && !created {
            return Some(format!(
                "VLAN {vid} is not on the switch and switch.desired.vlans does not create it"
            ));
        }
    }
    None
}

fn plan_port_config(
    actual: &SwitchState,
    desired: &SwitchDesired,
    protected: &Protected,
    out: &mut Plan,
) {
    let wanted: Vec<&DesiredPort> = desired
        .ports
        .iter()
        .filter(|p| p.enabled.is_some() || p.speed.is_some())
        .collect();
    if wanted.is_empty() {
        return;
    }
    if actual.ports.is_empty() {
        out.unread.push("port.cgi");
        return;
    }
    for want in wanted {
        let Some(have) = actual.port(want.port) else {
            out.refusals.push(Refusal {
                kind: OpKind::PortConfig,
                reason: format!("the switch has no port {}", want.port),
            });
            continue;
        };
        let enabled = want.enabled.unwrap_or(have.enabled);
        let speed = want.speed.unwrap_or(have.speed);
        if speed.impossible_on(want.port) {
            out.refusals.push(Refusal {
                kind: OpKind::PortConfig,
                reason: format!("port {} has no {speed}", want.port),
            });
            continue;
        }
        if enabled == have.enabled && speed == have.speed {
            continue;
        }
        let duplex = forms::duplex_for(speed, have.duplex);
        let detail = format!(
            "port {} {} at {speed}",
            want.port,
            if enabled { "enabled" } else { "disabled" }
        );
        if protected.covers(want.port) {
            out.refusals.push(Refusal {
                kind: OpKind::PortConfig,
                reason: format!("{}: {detail} needs \"force\"", protected.why(want.port)),
            });
            continue;
        }
        out.ops.push(
            Op::new(
                OpKind::PortConfig,
                "port.cgi",
                forms::port_config(want.port, enabled, speed, duplex, have.flow),
                detail,
            )
            .on_ports(&[want.port])
            .settles(PORT_SETTLE_MS),
        );
    }
}

fn plan_jumbo(actual: &SwitchState, desired: &SwitchDesired, out: &mut Plan) {
    let Some(want) = desired.jumbo else {
        return;
    };
    let Some(have) = actual.jumbo else {
        out.unread.push("fwd.cgi?page=jumboframe");
        return;
    };
    if want == 0 {
        if have.enabled {
            out.ops.push(Op::new(
                OpKind::Jumbo,
                "fwd.cgi?page=jumboframe",
                forms::jumbo_toggle(false),
                String::from("jumbo frames off (the switch resets the size to 1522)"),
            ));
        }
        return;
    }
    let Some(form) = forms::jumbo_size(want) else {
        out.refusals.push(Refusal {
            kind: OpKind::Jumbo,
            reason: format!("the switch does not offer a {want} byte frame size"),
        });
        return;
    };
    // Turning jumbo frames on keeps the size at 1522, so the size is
    // always written after the toggle.
    if !have.enabled {
        out.ops.push(Op::new(
            OpKind::Jumbo,
            "fwd.cgi?page=jumboframe",
            forms::jumbo_toggle(true),
            String::from("jumbo frames on"),
        ));
    }
    if !have.enabled || have.size != want {
        out.ops.push(Op::new(
            OpKind::Jumbo,
            "fwd.cgi?page=jumboframe",
            form,
            format!("jumbo frame size {want}"),
        ));
    }
}

fn plan_storm(actual: &SwitchState, desired: &SwitchDesired, out: &mut Plan) {
    let Some(want) = &desired.storm else {
        return;
    };
    let Some(ports) = parse_port_list(&want.ports) else {
        out.refusals.push(Refusal {
            kind: OpKind::Storm,
            reason: format!("\"{}\" is not a port list", want.ports),
        });
        return;
    };
    if actual.storm.is_empty() {
        out.unread.push("fwd.cgi?page=storm_ctrl");
        return;
    }
    for port in ports {
        let Some(have) = actual.storm_port(port) else {
            continue;
        };
        let target = |given: Option<u32>, current: Option<u32>| -> Option<u32> {
            match given {
                // 0 is how a document says "unlimited", which is an
                // empty field on the wire.
                Some(0) => None,
                Some(rate) => Some(rate.clamp(forms::RATE_MIN, forms::rate_max(port))),
                None => current,
            }
        };
        let bcast = target(want.bcast_kbps, have.bcast);
        let mcast = target(want.mcast_kbps, have.mcast);
        let uucast = target(want.unknown_ucast_kbps, have.unknown_ucast);
        let umcast = target(want.unknown_mcast_kbps, have.unknown_mcast);
        if (bcast, mcast, uucast, umcast)
            == (have.bcast, have.mcast, have.unknown_ucast, have.unknown_mcast)
        {
            continue;
        }
        out.ops.push(
            Op::new(
                OpKind::Storm,
                "fwd.cgi?page=storm_ctrl",
                forms::storm(port, bcast, mcast, uucast, umcast),
                format!(
                    "port {port} storm control bcast {} mcast {} unknown-ucast {} \
                     unknown-mcast {} (Kbit/s)",
                    rate_text(bcast),
                    rate_text(mcast),
                    rate_text(uucast),
                    rate_text(umcast)
                ),
            )
            .on_ports(&[port]),
        );
    }
}

fn rate_text(rate: Option<u32>) -> String {
    rate.map(|r| r.to_string())
        .unwrap_or_else(|| String::from("unlimited"))
}

fn plan_loop(actual: &SwitchState, desired: &SwitchDesired, out: &mut Plan) {
    let Some(want) = desired.loop_mode else {
        return;
    };
    let Some(have) = actual.loop_cfg else {
        out.unread.push("loop.cgi");
        return;
    };
    if have.mode == want {
        return;
    }
    // Enabling spanning tree flaps the uplink for about 10 s and the
    // POST usually gets no reply (docs/sodola.md, "Quirks").
    let op = Op::new(
        OpKind::Loop,
        "loop.cgi",
        forms::loop_mode(want, have.interval_s, have.recover_s),
        format!("loop prevention {} (was {})", want.as_str(), have.mode.as_str()),
    );
    out.ops.push(if want == LoopMode::Stp || have.mode == LoopMode::Stp {
        op.may_timeout()
    } else {
        op
    });
}

fn plan_ip(actual: &SwitchState, desired: &SwitchDesired, out: &mut Plan) {
    let Some(want) = &desired.ip else {
        return;
    };
    let Some(have) = &actual.ip else {
        out.unread.push("ip.cgi");
        return;
    };
    match want.mode {
        DesiredIpMode::Dhcp => {
            if have.dhcp {
                return;
            }
            out.ops.push(
                Op::new(
                    OpKind::Ip,
                    "ip.cgi",
                    forms::ip_dhcp(),
                    String::from("management address from DHCP"),
                )
                .may_timeout(),
            );
        }
        DesiredIpMode::Static => {
            let Some((addr, bits)) = parse_cidr(&want.addr) else {
                out.refusals.push(Refusal {
                    kind: OpKind::Ip,
                    reason: format!("\"{}\" is not an address/prefix", want.addr),
                });
                return;
            };
            let ip = super::model::ipv4_string(addr);
            let netmask = netmask_of_prefix(bits);
            if !have.dhcp && have.ip == ip && have.netmask == netmask && have.gateway == want.gw {
                return;
            }
            out.ops.push(
                Op::new(
                    OpKind::Ip,
                    "ip.cgi",
                    forms::ip_static(&ip, &netmask, &want.gw),
                    format!("management address {ip}/{bits} gateway {}", want.gw),
                )
                .may_timeout(),
            );
        }
    }
}

/// An imperative port enable/disable, as the `switch_port` command asks
/// for it: the same form the planner builds, with the port's current
/// speed, duplex and flow control kept (ADR 0006 component 21).
pub fn port_enable_op(
    actual: &SwitchState,
    port: u8,
    enabled: bool,
    protected: &Protected,
) -> Result<Op, String> {
    let have = actual
        .port(port)
        .ok_or_else(|| format!("port {port} has not been read yet"))?;
    if protected.covers(port) {
        return Err(format!(
            "{}: a port enable/disable needs \"force\"",
            protected.why(port)
        ));
    }
    let speed = have.speed;
    let duplex = if speed == Speed::Auto {
        Duplex::Auto
    } else {
        have.duplex
    };
    Ok(Op::new(
        OpKind::PortConfig,
        "port.cgi",
        forms::port_config(port, enabled, speed, duplex, have.flow),
        format!("port {port} {}", if enabled { "enabled" } else { "disabled" }),
    )
    .on_ports(&[port])
    .settles(PORT_SETTLE_MS))
}

#[cfg(test)]
mod tests {
    use super::super::model::{
        DesiredIp, DesiredVlan, IpCfg, Jumbo, LoopCfg, PortCfg, PortVlan, StormPort, Vlan,
        VlanTable,
    };
    use super::*;

    fn state() -> SwitchState {
        SwitchState {
            vlans: Some(VlanTable {
                enabled: true,
                vlans: alloc::vec![
                    Vlan {
                        id: 1,
                        name: String::new()
                    },
                    Vlan {
                        id: 100,
                        name: String::from("data")
                    },
                ],
            }),
            port_vlans: (1..=9)
                .map(|port| PortVlan {
                    port,
                    mode: VlanMode::Trunk,
                    access: None,
                    native: Some(1),
                    allowed: alloc::vec![1],
                })
                .collect(),
            ports: (1..=9)
                .map(|port| PortCfg {
                    port,
                    enabled: true,
                    duplex: Duplex::Auto,
                    duplex_actual: Duplex::Auto,
                    speed: Speed::Auto,
                    speed_actual: Speed::Auto,
                    flow: false,
                    flow_actual: false,
                })
                .collect(),
            jumbo: Some(Jumbo {
                enabled: true,
                size: 12_000,
            }),
            storm: (1..=9).map(|port| StormPort {
                port,
                ..StormPort::default()
            }).collect(),
            loop_cfg: Some(LoopCfg::default()),
            ip: Some(IpCfg {
                dhcp: false,
                ip: String::from("192.168.64.241"),
                netmask: String::from("255.255.255.0"),
                gateway: String::from("192.168.64.254"),
            }),
            ..SwitchState::default()
        }
    }

    fn desired() -> SwitchDesired {
        SwitchDesired {
            vlans: Some(alloc::vec![
                DesiredVlan {
                    id: 1,
                    name: String::new()
                },
                DesiredVlan {
                    id: 100,
                    name: String::from("data")
                },
                DesiredVlan {
                    id: 300,
                    name: String::from("storage")
                },
            ]),
            ports: alloc::vec![DesiredPort {
                port: 7,
                mode: Some(VlanMode::Access),
                vlan: Some(300),
                ..DesiredPort::default()
            }],
            jumbo: Some(9_216),
            loop_mode: Some(LoopMode::Loop),
            ip: Some(DesiredIp {
                mode: DesiredIpMode::Static,
                addr: String::from("10.0.202.5/24"),
                gw: String::from("10.0.202.1"),
            }),
            ..SwitchDesired::default()
        }
    }

    #[test]
    fn the_plan_creates_vlans_before_it_moves_ports_and_sets_the_address_last() {
        let made = plan(&state(), &desired(), &Protected::new(Some(9), Some(9)));
        let kinds: Vec<OpKind> = made.ops.iter().map(|op| op.kind).collect();
        assert_eq!(
            kinds,
            alloc::vec![
                OpKind::VlanCreate,
                OpKind::PortVlan,
                OpKind::Jumbo,
                OpKind::Loop,
                OpKind::Ip
            ],
            "{:?}",
            made.summary()
        );
        assert!(made.refusals.is_empty(), "{:?}", made.refusals);
        assert!(made.unread.is_empty());
        // The VLAN the port is moved into is created by this same plan.
        assert_eq!(made.ops[0].detail, "create VLAN 300 \"storage\"");
        assert_eq!(made.ops[1].detail, "port 7 access VLAN 300");
        assert_eq!(
            forms::encode(&made.ops[1].form),
            "port6=on&vlan_type=0&vlan_access=300"
        );
        assert!(made.ops[4].may_timeout, "an IP change may not answer");
    }

    #[test]
    fn a_vlan_no_longer_wanted_is_deleted_after_the_ports_move_and_vlan_1_never_is() {
        let mut want = desired();
        want.vlans = Some(alloc::vec![DesiredVlan {
            id: 300,
            name: String::from("storage"),
        }]);
        let made = plan(&state(), &want, &Protected::new(Some(9), Some(9)));
        let order: Vec<&str> = made.ops.iter().map(|op| op.kind.as_str()).collect();
        let create = order.iter().position(|k| *k == "vlan_create").expect("create");
        let moved = order.iter().position(|k| *k == "port_vlan").expect("port");
        let delete = order.iter().position(|k| *k == "vlan_delete").expect("delete");
        assert!(create < moved && moved < delete, "{order:?}");
        let del = &made.ops[delete];
        assert_eq!(
            forms::encode(&del.form),
            "remove_100=on",
            "VLAN 1 is never deleted"
        );
    }

    #[test]
    fn protected_ports_are_refused_without_force() {
        let mut want = desired();
        want.ports = alloc::vec![
            DesiredPort {
                port: 9,
                mode: Some(VlanMode::Access),
                vlan: Some(100),
                ..DesiredPort::default()
            },
            DesiredPort {
                port: 3,
                enabled: Some(false),
                ..DesiredPort::default()
            },
        ];
        // Port 9 is the uplink, the controller's MAC is on port 9 too.
        let made = plan(&state(), &want, &Protected::new(Some(9), Some(9)));
        assert_eq!(made.refusals.len(), 1, "{:?}", made.refusals);
        assert_eq!(made.refusals[0].kind, OpKind::PortVlan);
        assert!(made.refusals[0].reason.contains("force"));
        assert!(
            made.ops.iter().any(|op| op.kind == OpKind::PortConfig),
            "port 3 is not protected"
        );

        // With force, the uplink moves.
        let forced = plan(&state(), &want, &Protected::none());
        assert!(forced.refusals.is_empty(), "{:?}", forced.refusals);
        assert!(forced.ops.iter().any(|op| op.ports == alloc::vec![9]));

        // Without the controller's own port known, everything is
        // protected.
        let blind = plan(&state(), &want, &Protected::new(Some(9), None));
        assert_eq!(blind.refusals.len(), 2, "{:?}", blind.refusals);
        assert!(blind.refusals[1].reason.contains("own MAC"));
    }

    #[test]
    fn a_port_write_settles_and_keeps_what_it_was_not_asked_to_change() {
        let mut want = SwitchDesired {
            ports: alloc::vec![DesiredPort {
                port: 4,
                speed: Some(Speed::G1),
                ..DesiredPort::default()
            }],
            ..SwitchDesired::default()
        };
        let made = plan(&state(), &want, &Protected::new(Some(9), Some(9)));
        assert_eq!(made.ops.len(), 1);
        let op = &made.ops[0];
        assert_eq!(op.settle_ms, PORT_SETTLE_MS);
        assert_eq!(
            forms::encode(&op.form),
            "port3=on&state=1&duplex=1&speed=2&flowctrl=0&cmd=port",
            "the port stays enabled, a fixed speed takes full duplex"
        );

        // 10G on a copper port is refused, not attempted.
        want.ports[0].speed = Some(Speed::G10);
        let refused = plan(&state(), &want, &Protected::none());
        assert!(refused.ops.is_empty());
        assert_eq!(refused.refusals.len(), 1);
    }

    #[test]
    fn nothing_is_planned_for_a_page_that_was_not_read() {
        let empty = SwitchState::default();
        let made = plan(&empty, &desired(), &Protected::none());
        assert!(made.ops.is_empty());
        assert!(made.unread.contains(&"vlan.cgi?page=static"));
        assert!(made.unread.contains(&"fwd.cgi?page=jumboframe"));
        assert!(made.unread.contains(&"loop.cgi"));
        assert!(made.unread.contains(&"ip.cgi"));
    }

    #[test]
    fn a_switch_that_already_matches_gets_an_empty_plan() {
        let mut have = state();
        have.vlans = Some(VlanTable {
            enabled: true,
            vlans: alloc::vec![
                Vlan {
                    id: 1,
                    name: String::new()
                },
                Vlan {
                    id: 100,
                    name: String::from("data")
                },
                Vlan {
                    id: 300,
                    name: String::from("storage")
                },
            ],
        });
        if let Some(pv) = have.port_vlans.iter_mut().find(|p| p.port == 7) {
            pv.mode = VlanMode::Access;
            pv.access = Some(300);
            pv.native = None;
            pv.allowed = Vec::new();
        }
        have.jumbo = Some(Jumbo {
            enabled: true,
            size: 9_216,
        });
        have.loop_cfg = Some(LoopCfg {
            mode: LoopMode::Loop,
            ..LoopCfg::default()
        });
        have.ip = Some(IpCfg {
            dhcp: false,
            ip: String::from("10.0.202.5"),
            netmask: String::from("255.255.255.0"),
            gateway: String::from("10.0.202.1"),
        });
        let made = plan(&have, &desired(), &Protected::new(Some(9), Some(9)));
        assert!(made.is_empty(), "{:?}", made.summary());
        assert!(drift(&have, &desired()).is_empty());
    }

    #[test]
    fn an_undo_plans_the_snapshot_back() {
        let snapshot = state();
        let want = desired();
        // The switch as it is after the change was applied.
        let mut after = state();
        after.vlans = Some(VlanTable {
            enabled: true,
            vlans: alloc::vec![
                Vlan {
                    id: 1,
                    name: String::new()
                },
                Vlan {
                    id: 100,
                    name: String::from("data")
                },
                Vlan {
                    id: 300,
                    name: String::from("storage")
                },
            ],
        });
        if let Some(pv) = after.port_vlans.iter_mut().find(|p| p.port == 7) {
            pv.mode = VlanMode::Access;
            pv.access = Some(300);
            pv.native = None;
            pv.allowed = Vec::new();
        }
        after.jumbo = Some(Jumbo {
            enabled: true,
            size: 9_216,
        });
        after.ip = Some(IpCfg {
            dhcp: false,
            ip: String::from("10.0.202.5"),
            netmask: String::from("255.255.255.0"),
            gateway: String::from("10.0.202.1"),
        });

        let back = undo(&after, &snapshot, &want);
        let summary = back.summary();
        assert!(back.refusals.is_empty(), "{:?}", back.refusals);
        // Port 7 goes back to the trunk it was, VLAN 300 goes away
        // after it, the frame size and the address come back.
        assert!(
            summary.iter().any(|s| s.starts_with("port 7 trunk native 1")),
            "{summary:?}"
        );
        assert!(summary.iter().any(|s| s == "delete VLAN 300"), "{summary:?}");
        assert!(
            summary.iter().any(|s| s == "jumbo frame size 12000"),
            "{summary:?}"
        );
        assert!(
            summary
                .iter()
                .any(|s| s == "management address 192.168.64.241/24 gateway 192.168.64.254"),
            "{summary:?}"
        );
        let delete = back
            .ops
            .iter()
            .position(|op| op.kind == OpKind::VlanDelete)
            .expect("the VLAN goes away");
        let moved = back
            .ops
            .iter()
            .position(|op| op.kind == OpKind::PortVlan)
            .expect("the port moves back");
        assert!(moved < delete, "the port leaves VLAN 300 before it is deleted");

        // And after the undo the switch matches the snapshot again:
        // planning the undone state against the snapshot's own
        // description is empty.
        let undone = undo_desired(&snapshot, &want);
        let plan_again = plan(&snapshot, &undone, &Protected::none());
        assert!(plan_again.is_empty(), "{:?}", plan_again.summary());
    }

    #[test]
    fn storm_control_keeps_what_it_was_not_asked_to_change() {
        let mut have = state();
        have.storm = alloc::vec![StormPort {
            port: 1,
            bcast: Some(5_000),
            mcast: Some(6_000),
            unknown_ucast: None,
            unknown_mcast: None,
        }];
        let want = SwitchDesired {
            storm: Some(DesiredStorm {
                ports: String::from("1"),
                bcast_kbps: Some(10_000),
                ..DesiredStorm::default()
            }),
            ..SwitchDesired::default()
        };
        let made = plan(&have, &want, &Protected::none());
        assert_eq!(made.ops.len(), 1);
        assert_eq!(
            forms::encode(&made.ops[0].form),
            "port0=on&bc_rate=10000&mc_rate=6000&uuc_rate=&umc_rate=&cmd=storm",
            "the multicast limit is re-sent, the unlimited fields stay empty"
        );
    }

    #[test]
    fn an_imperative_port_disable_keeps_the_rest_of_the_port() {
        let have = state();
        let op = port_enable_op(&have, 3, false, &Protected::new(Some(9), Some(9)))
            .expect("port 3 is not protected");
        assert_eq!(
            forms::encode(&op.form),
            "port2=on&state=0&duplex=2&speed=10&flowctrl=0&cmd=port"
        );
        assert_eq!(op.settle_ms, PORT_SETTLE_MS);
        assert!(
            port_enable_op(&have, 9, false, &Protected::new(Some(9), Some(9))).is_err(),
            "the uplink needs force"
        );
        assert!(port_enable_op(&have, 9, false, &Protected::none()).is_ok());
    }
}
