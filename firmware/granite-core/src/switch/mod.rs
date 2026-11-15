//! The block switch: model, page parsers, form builders and the
//! reconcile planner (ADR 0006 component 15).
//!
//! Everything here is pure and host-tested. The firmware's worker
//! (`granite-fw/src/switch/`) owns the socket and the schedule; this
//! crate owns what a page means and what a write looks like, so a
//! firmware change cannot quietly alter either.
//!
//! Which units the image knows is [`profile`]: one row per (model,
//! firmware) pair whose pages were captured, carrying the port layout,
//! the page dialect and whether writes are allowed (ADR 0006 component
//! 24). Nothing else in here knows a port count.
//!
//! The switch is a Sodola SL902-SWTGW218AS with a CGI web UI and
//! nothing else: no SSH, SNMP or API. The protocol was
//! reverse-engineered in `Granite/infra` (`docs/sodola.md`,
//! `tools/sodola.py`); the rules it imposes - the `Referer` header, the
//! `md5(user + password)` cookie, 0-based port fields, forms that need
//! every field, an empty rate field meaning unlimited - live in
//! [`forms`] and [`parse`] and nowhere else.

pub mod counters;
pub mod forms;
pub mod md5;
pub mod model;
pub mod obs;
pub mod parse;
pub mod plan;
pub mod profile;
pub mod scan;

pub use counters::{PortCounters, PortRates};
pub use obs::{SwitchObs, SwitchPortObs};
pub use forms::Form;
pub use md5::session_token;
pub use model::{
    DesiredIp, DesiredIpMode, DesiredPort, DesiredStorm, DesiredVlan, Duplex, IpCfg, Jumbo,
    LoopCfg, LoopMode, MacEntry, MacPage, PortCfg, PortStats, PortVlan, RatePort, Speed,
    StormPort, SwitchDesired, SwitchInfo, SwitchState, Vlan, VlanMode, VlanTable, same_mac,
};
pub use profile::{Dialect, Layout, PROFILES, Profile, Tier, lookup};
pub use parse::ParseError;
pub use plan::{Op, OpKind, Plan, Protected, Refusal, drift, plan, undo, undo_desired};
