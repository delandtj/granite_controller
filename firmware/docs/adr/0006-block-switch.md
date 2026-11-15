# Block switch: the controller reads its Sodola switch, my_metal commissions the nodes

**Status**: Accepted
**Date**: 2026-11-11
**Updated**: 2026-11-11 (review asks answered by the user: desired state
yes, but revert by re-applying the previous state while the switch is
reachable, reboot only as the fallback; drift is report-only; counters
on their own topic; switches are pre-provisioned with the infra script)
**Updated**: 2026-11-12 (switch and controller of a block are paired by
pre-provisioning; node commissioning is done by the my_metal provider,
the controller is a power device and a switch-port source for it;
the controller reads the switch by default, writes become optional)
**Updated**: 2026-11-12 (built: components 15, 16, 17, 18, 21 and the
pairing check of 22 are in the firmware; desired state is in scope with
`manage` gating every write. See "Implementation" for what was built
differently and what is not built yet)
**Updated**: 2026-11-15 (switch profiles: the write gate is keyed on
(model, firmware) instead of the firmware string alone, and the port
layout is profile data instead of constants. New component 24)
**Updated**: 2026-11-15 (VLAN 1 invariant: node ports never carry the
management VLAN; a config check and a report-only exposure list. New
component 25)

Related: firmware/docs/adr/0001-firmware-architecture.md (components 6
Network, 7 MQTT, 8 HTTP API, 10 Configuration; commit-confirm),
firmware/docs/adr/0004-controller-rev-d-node-model.md (nodes).
Node commissioning: geomind_code/my_metal_provider_and_broker
(`crates/my_metal_provider_server`: README scan steps, `src/pdu.rs`,
`oschema/provider/provider.oschema` LanPlan/LanNode/LanPdu/NodeView).
Switch protocol, quirks and a reference client: Granite/infra
`docs/sodola.md` and `tools/sodola.py` (reverse-engineered and tested
against a live unit, 2026-11-11).

---

## Context

A block is one controller plus 7 or 8 nodes. Each block gets its own
switch: a Sodola SL902-SWTGW218AS (8x 2.5G copper, 1x 10G SFP+, Realtek
RTL8372/73, firmware V200.1.8 on the unit at hand). The switch has no
SSH, telnet, SNMP, HTTPS or API: only a CGI web UI on HTTP port 80.

We want the block to be self-contained: the controller reports the
switch to MQTT and applies switch settings that come in over MQTT, the
same way it already does for its nodes. Today the switch is configured
by hand or with the infra script from a laptop.

Physical layout (2026-11-12): controllers are not cabled to their own
block's switch. The controllers of two pods hang off one switch with
one uplink to the core; every block's Sodola has its own uplink to the
pod ToR. All 12 controllers and 12 Sodolas of a pod share one
management VLAN. So "which switch is mine" cannot be read from the
controller's own link; it comes from pre-provisioning and is checked
through the nodes, which are cabled to the block's switch.

Node commissioning already exists in the my_metal provider
(`my_metal_provider_server`): one provider per LAN, a `LanPlan` with
`nodes: [LanNode]` keyed by MAC (`pdu`, `pdu_outlet`, `ip`, `proven`)
and `pdus: [LanPdu]` (`kind` = "netio" | "racktivity"). Its scan cuts
and restores each PDU outlet in turn and takes the machine that then
fetches `boot.ipxe` as the machine on that outlet, writing it into the
plan. The provider serves TFTP and iPXE itself; DHCP is the RouterOS
router. Switch ports are only seen from the router's bridge host table
(`NodeView.port`), so behind a dumb block switch the provider sees the
whole block on one router port. The provider has no MQTT; its RPC is
OpenRPC on a Unix socket behind hero_router, admin verbs gated by a
mycelium key. PDU drivers are a private `enum Drv` in `src/pdu.rs`
picked by `LanPdu.kind`, one credential profile per kind.

What the infra work established about the switch (details in
`docs/sodola.md`):

- Auth is a cookie `admin=md5(user + pass)`, accepted only after a
  login POST to `login.cgi`; the switch forgets logins on reboot. Every
  request needs a `Referer` header or the CGI answers with an empty
  body.
- Writes are form POSTs that change the running config at once;
  `save.cgi` writes it to flash. A reboot drops unsaved changes and
  takes about 3 s. A save + reboot round-trip was verified to keep
  VLANs, port VLANs, static MACs and the name.
- Port config (state, speed, duplex, flow) applies about 1 s after the
  POST; a second port write inside that window is silently dropped.
  Enabling STP flaps the uplink for about 10 s.
- Several forms need every field on every submit; an empty rate or
  storm field means unlimited. Read-modify-write is mandatory.
- Some state is shared server-side with any browser on the web UI
  (MAC table paging and search filter).
- Responses are slow: the stats page takes about 390 ms, the port
  config page about 460 ms, a full MAC table walk about 1.1 s.
- Only good packet and byte counters exist (64-bit, as two u32 halves).
  No error, CRC or drop counters. No push of any kind.
- No management VLAN setting: management most likely sits in VLAN 1.
- Security: CVE-2026-27751..27758 (sibling model, same firmware
  family): default admin/admin, cleartext HTTP, no login rate limit,
  static MD5 cookie, reflected XSS, CSRF. No fixed firmware published.

Constraints from ADR 0001: ESP-IDF Rust on the ESP32-C6, one W5500
interface, payloads built in `granite-core` and host-tested, secrets in
their own NVS namespace, every reachability-affecting change
commit-confirmed, and nothing slow in the actuator or CAN path.

---

## Decision

Add a **switch** component to granite-fw. It owns all traffic to the
block switch, polls it and publishes what changed. Reading is the
default and the first deliverable. Writing the switch configuration as
**desired state** in the controller config, applied by a reconcile step
with commit-confirm, is an optional part (`switch.manage = true`,
default false): switches arrive pre-provisioned by the infra script, so
a block works without it.

Node commissioning stays in the my_metal provider. The controller does
not run its own discovery or registration; it makes the provider's
existing scan work on a Granite block:

- **Power device**: the controller is a my_metal PDU of a new kind
  `granite`. `LanPdu` = the controller, `LanNode.pdu_outlet` = the node
  id (1-8). The provider's scan, `power` and `outlet_*` verbs drive it
  through the controller's existing HTTP API (ADR 0001 component 8).
- **Switch-port source**: the controller publishes which MAC sits on
  which block-switch port, so the provider can show and check the real
  port instead of the router port the whole block shares.
- **Pairing**: switch and controller are paired by pre-provisioning and
  checked by the controller through the nodes (component 22).

1. One worker thread talks to the switch. Nothing else in the firmware
   does. Requests are strictly serial, spaced, and retried as the
   protocol needs (re-login on redirect, 2 s settle after port writes).
2. Monitoring by polling: link state on a short period, counters and
   switch health on longer ones. Changes become events; a summary goes
   into the retained `state`; counters go to their own non-retained
   topic.
3. Configuration is a new config section `switch`, set through the
   existing `config_set` (HTTP and MQTT), staged and confirmed like
   `net` and `mqtt`. A reconcile diffs desired against actual, applies
   the plan without saving, verifies, and saves on confirm. Revert
   re-applies the previous state while the switch is reachable; only
   when it is not (or the undo does not verify) does the controller
   reboot the switch, which discards unsaved changes.
4. A small set of imperative commands for things that are actions, not
   state: port enable/disable (by node or port), password rotation,
   save, reboot, backup.
5. The switch password lives in the secrets namespace, is generated by
   the controller on rotation and never leaves it.
6. The controller protects its own path: it never disables or
   re-VLANs the port it is reached through (found from the MAC table)
   or the configured uplink, unless the command says `force`.
7. Parsing and planning are pure functions in granite-core, tested on
   the host against HTML captured from the real switch.
8. Items 3-6 (desired state, imperative writes other than port
   enable/disable, password rotation, protection) only run with
   `switch.manage = true`. Without it the controller never POSTs to
   the switch except to log in.

---

## Architecture Overview

### Component Breakdown

Numbered after ADR 0001's list.

15. **Switch model and parser** (`granite-core/src/switch/`)
    - `model.rs`: `SwitchState` (fw, uptime, MAC, IP config, VLANs,
      per-port VLAN mode, port admin/oper state and speed, jumbo,
      storm, rate limits, isolation, MAC table) and `SwitchDesired`
      (the config section, below). serde, no_std + alloc.
    - `parse.rs`: one function per CGI page, `&str -> Result<T>`. A
      small hand-written scanner (`<tr>`/`<td>` cells, `name=".." ...
      checked`, `<option ... selected>`, `name=".." value=".."`); no
      `regex` crate (flash size). Every parser rejects a page it does
      not recognise rather than guessing.
    - `forms.rs`: builders for every write as `Vec<(&str, String)>`
      form pairs, including the read-modify-write rules (full field set
      for port, rate, storm, STP forms; repeated keys for isolation;
      0-based port fields).
    - `plan.rs`: `plan(actual, desired, protected) -> Vec<Op>`. Ordered
      ops: create VLANs, then port VLAN changes, then delete VLANs, then
      jumbo / storm / rate / isolation, then IP last. Ops that touch a
      protected port without `force` are refused with a reason.
    - Fixtures:
      `granite-core/tests/fixtures/sl902-swtgw218as-v200.1.8/*.html`,
      captured from the unit with `tools/sodola.py get`. One directory
      per profile row (component 24).

16. **Switch client** (`granite-fw/src/switch/client.rs`)
    - esp-idf-svc `EspHttpConnection` (already used by OTA), plain HTTP,
      one request at a time. Adds `Cookie` and `Referer`; on a redirect
      to `login.cgi` performs the login POST once and retries.
    - Minimum spacing between requests (default 100 ms); 2 s settle
      after a port-config write; timeouts 10 s, read-back retries after
      writes that can flap the uplink (STP, IP).
    - Buffers one page at a time (pages are 2-10 KB); no page outlives
      its parse.
    - Fingerprint check at first contact: the (model, firmware) pair
      from `info.cgi` must resolve to a `verified` profile (component
      24). Anything else: monitoring continues if the parsers accept
      the pages, writes are refused (`switch_fw_unknown` fault).

17. **Switch worker** (`granite-fw/src/switch/mod.rs`)
    - Own thread, low priority, never touches the actuator, CAN link or
      Modbus. Talks to the rest of the firmware through the event bus,
      the `Observed` snapshot and the dispatcher channel, like MQTT does.
    - Poll schedule (all configurable, defaults):

      | What | Page | Period | Output |
      |---|---|---|---|
      | link state + counters | `port.cgi?page=stats` | 5 s | link change -> event + state; counters -> counters topic every 60 s |
      | speed/duplex of a port | `port.cgi` | on link up | `switch_port` event carries the negotiated speed |
      | health | `info.cgi` | 60 s | uptime; uptime going down or counters going down = switch rebooted |
      | MAC table | `mac.cgi` walk | 120 s and on link change | node-to-port map, controller's own port |
      | drift check | VLAN, port VLAN, other config pages | 10 min | `switch_drift` event when actual != desired |

    - Counters: rates computed on the controller from successive reads;
      a decrease is treated as a reset, not a negative rate.
    - Reachability: 3 consecutive failed polls -> `switch_unreachable`
      event and `state.switch.reachable = false`; first success after ->
      `switch_reachable`.

18. **Switch reconcile** (in the worker; logic in `plan.rs`)
    - Triggered by a staged or confirmed change to the `switch` config
      section, by `switch_reconcile` command, and (only if
      `enforce = true`) by drift.
    - Staged change: read actual, plan, apply ops in order, read back,
      check the controller can still reach the switch and the broker.
      Apply does not save. The section stays staged; the existing
      confirm timer (ADR 0001 component 6, `t_confirm`) runs.
    - Before applying, the worker snapshots the actual state it read
      (`SwitchState`); that snapshot is what a revert goes back to.
    - Confirm: `save.cgi`, then download a config backup (component 20).
    - Timeout or failed verify, switch reachable: plan from the current
      actual state back to the snapshot and apply it the same way
      (unsaved), read back, compare with the snapshot. No reboot, no
      traffic interruption beyond the ports the undo itself touches.
      Emit `switch_reverted` with `"how":"undo"`.
    - Switch unreachable, or the undo's read-back does not match the
      snapshot: reboot the switch (unsaved changes gone, saved config
      restored; about 3 s for all ports), wait for it, re-read, emit
      `switch_reverted` with `"how":"reboot"`. If it stays unreachable,
      `switch_unreachable` as usual.
    - Invariant either way: nothing is saved before confirm, so the
      flash always holds the last confirmed state and the reboot path
      is always a valid fallback.
    - The controller's own staged section is restored as for `net`.
    - IP changes to the switch use the same path; the client follows
      the new address for the verify step.

19. **Switch credentials** (`granite-fw/src/switch/creds.rs`,
    secrets namespace)
    - Stores user and password; neither is exported, logged or
      published. Default on first use: `admin` / `admin`.
    - `switch_rotate_password`: generate 20 random chars (esp_random,
      alphanumeric), store as `pending`, apply via `user.cgi`, log in
      fresh with the new password, save the switch, promote `pending`
      to `current`. On boot or failure with a `pending` present, try
      `pending` first, then `current`. Ack carries only success and a
      timestamp.

    Components 18 and 19 are only active with `switch.manage = true`.

20. **Switch backup** (`granite-fw/src/switch/backup.rs`)
    - After every successful save, `config_back.cgi?cmd=conf_backup`
      (about 2.7 KB, contains credential material) is stored in NVS,
      one slot, in the secrets namespace. Downloadable through the
      HTTP API by an admin session; not over MQTT.
    - Restore is out of scope until the restore upload has been tested
      on the switch (open question).

21. **MQTT and API glue** (`granite-fw/src/mqtt.rs`, `http.rs`,
    messages in `granite-core/src/msg.rs`)
    - `state` gains `switch`: `{reachable, fw, uptime_s, mac,
      ports: [{port, enabled, link, speed, vlan_mode, node, macs}],
      drift, last_reconcile}`. Coalesced with the rest of `state`.
    - New events: `switch_port` (port, link, speed, node),
      `switch_reboot`, `switch_unreachable`, `switch_reachable`,
      `switch_drift` (what differs), `switch_reverted`,
      `switch_node_moved` (a known node MAC seen on another port),
      `switch_node_unbound` and `switch_mismatch` (component 22).
    - New topic `.../switch/counters`, out, not retained, every
      `t_counters_s` (default 60 s): per port tx/rx packets and bytes
      (totals) plus pps and bit/s over the interval.
    - Commands: `switch_port` (`{"port":N}` or `{"node":N}`,
      `"enabled":bool`, optional `"force":true`), `switch_rotate_password`,
      `switch_save`, `switch_reboot` and `switch_factory_reset` (both
      need `"confirm":"<device id>"` like `factory_reset`),
      `switch_reconcile`, `switch_backup` (refresh the stored backup).
      Config changes go through `config_set` with section `switch`, and
      the cycle is closed with `config_confirm` or `config_revert`
      (ADR 0001 components 6 and 7), so an operator who only has MQTT
      can stage, confirm and revert a switch change.

22. **Pairing check and node-port map** (`granite-core/src/switch/pair.rs`,
    driven from the worker)
    - The pairing itself is configuration: `switch.host` and
      `switch.mac`, written by the infra provisioning script from its
      inventory (block -> switch MAC, IP; controller). The controller
      refuses to talk to a switch whose MAC (`info.cgi`) differs from
      `switch.mac` (`switch_mismatch` fault, no polling beyond `info`).
    - Node-port map: on every node power change the controller already
      makes (on, off, cycle, on_all), the worker reads link state before
      and after. A port whose link appears, disappears or changes speed
      within `t_link_s` (default 20 s) of the action is bound to that
      node: `{node, port, mac[], since}`. MACs come from the MAC table
      walk on that port. Several MACs on one port are all kept (BMC on a
      shared NIC plus the PXE NIC); the switch cannot tell them apart and
      the provider's scan decides which one booted.
    - Nodes whose NIC keeps link while off (standby, WoL, shared BMC)
      give no link edge; for them only a MAC appearing on a port after
      power-on binds, which may take until the node first sends a frame.
    - Check, not discovery: if a node action is followed by no edge on
      any port for 3 actions in a row while other nodes bind fine, raise
      `switch_node_unbound`; if no node binds at all, raise
      `switch_mismatch` (wrong switch, or cabling into another block).
    - The map is kept in RAM and in the retained `state`; it is rebuilt
      by normal operation and never needs to be configured.

23. **my_metal integration** (provider side lives in
    geomind_code/my_metal_provider_and_broker, not in this repo)
    - Provider: a new `Drv::Granite` in `src/pdu.rs`, `kind = "granite"`.
      Calls `POST /api/v1/nodes/<n>/{on,off,force_off,cycle}?wait=1`
      on the controller with `Authorization: Bearer <token>`; outlet
      list and state from `GET /api/v1/nodes`. `cycle` is done by the
      controller, like Netio ShortOff. TLS: the controller's cert is
      self-signed; the provider pins `cert_sha256` from the
      unauthenticated `/id` endpoint at `pdu_set` time.
    - Credentials: my_metal keeps one profile per PDU kind, so all
      controllers of a provider accept the same API token. The infra
      provisioning script installs it on each controller.
    - Switch ports: `GET /api/v1/switch/hosts` -> `[{port, mac, vlan,
      node}]` and `GET /api/v1/switch/ports` -> `[{port, link, speed,
      node}]` (HTTP, same data as the MQTT `state.switch`). The
      provider reads them for a `granite` PDU and fills a block-switch
      port next to the router port in `NodeView` (provider change:
      new optional field, e.g. `switch_port`).
    - Commissioning is the provider's scan, unchanged in shape: it
      powers each controller outlet off and on (one at a time, which
      also spreads inrush), takes the MAC that fetches `boot.ipxe`, and
      writes `LanNode {mac, pdu = <controller>, pdu_outlet = <node>}`.
      The controller adds the switch port and the link evidence through
      the endpoints above; with the provider's boot.ipxe proof plus
      the controller's port binding, outlet, MAC and switch port all
      line up or the mismatch is visible on one screen.
    - Cold start of a whole block (after install or a pod power loss)
      is the controller's own `on_all` stagger (ADR 0004), not the
      provider's job.
    - MQTT stays the controller's telemetry path to whoever listens;
      my_metal does not need it. Nothing in commissioning depends on
      MQTT delivery, because the provider's plan is the durable record.

24. **Switch profiles** (`granite-core/src/switch/profile.rs`)
    - A static table, one row per switch the image knows. A row is:

      | Field | Meaning |
      |-------|---------|
      | `model` | `Device Model` of `info.cgi`, exact (`SL902-SWTGW218AS`) |
      | `firmware` | `Firmware Version` of `info.cgi`, exact (`V200.1.8`) |
      | `layout` | port count and which ports are SFP+ |
      | `dialect` | which parser and form set speaks to it |
      | `tier` | `verified` (writes allowed) or `read_only` |

    - Lookup is on both strings, trimmed, exact. No prefix, range or
      "newer than" match: the pages are scraped, so a version nobody
      captured is a version nobody tested. The model is part of the key
      because a sibling model can report the same firmware string with
      another port count.
    - Write gate: writes need a row with tier `verified`. No row, or a
      `read_only` row, is monitored while its pages parse and never
      written; both raise `switch_fw_unknown` (the event name is kept,
      its detail is now `model firmware`).
    - Port layout is data. Nothing outside the profile knows that the
      switch has 9 ports or that port 9 is the SFP+: the parsers, the
      form builders, the planner, the speed rules and the observed
      snapshot take the layout of the resolved profile. A switch
      without a row is read with the widest layout in the table.
    - Config is checked twice. At config time, without a switch to
      ask, a port number is only checked against the widest layout in
      the table. Once the switch is fingerprinted the planner refuses
      any op on a port the unit does not have, with a reason, and an
      `uplink_port` the unit does not have counts as "no uplink
      declared" and is logged.
    - Dialect is an enum, not a trait object. One variant today
      (`SodolaV200`); `parse.rs` and `forms.rs` are that variant. A
      firmware that changes some pages gets a variant that overrides
      those page functions and falls through for the rest; the
      dispatch is added with the second variant, not before.
    - The table lives in the image, not in the config: "tested against
      these pages" is a property of the build. Enabling a version is a
      controller OTA. There is no config override that forces writes.
    - Fixtures: one directory per row under
      `granite-core/tests/fixtures/`, named `<model>-<firmware>` in
      lower case (`sl902-swtgw218as-v200.1.8`). A test walks the table:
      every row must have its directory, its `info.html` must parse to
      the row's own (model, firmware) and resolve back to that row,
      and every page must parse with the row's layout.
    - Adding a switch or a version: capture the pages with
      `tools/sodola.py get`, add the directory and a `read_only` row,
      fix or fork the page functions until the tests pass, do one
      write-and-revert on a live unit, promote the row to `verified`.
    - Out of scope: a switch with another management protocol (SNMP, a
      real API). That needs a driver seam in granite-fw above the page
      layer, producing `SwitchState` and applying `Op`; it is designed
      when such a switch is on the bench.
    - Fleet side: the infra provisioning script already touches every
      switch before it goes into a block. It should record model and
      firmware and refuse a unit without a `verified` row, which keeps
      the table short. The controller never updates switch firmware.

25. **VLAN 1 invariant** (`granite-core/src/switch/mgmt.rs`, the
    config check in `model.rs`, the report in the worker)
    - The switch has no management VLAN setting; its CPU (the web UI)
      answers in VLAN 1. So VLAN 1 is the management VLAN of a block,
      and the rule is: only the uplink and the controller's own port
      carry VLAN 1. A node port never does. The ToR maps the untagged
      traffic of the uplink into its own management VLAN with the
      port's PVID; that is the ToR's business, not the controller's.
    - "Carries VLAN 1" means any of: access VLAN 1, trunk with native
      1, trunk with 1 in its allowed list.
    - Config check, at config time: a `switch.desired.ports` entry that
      names a `node` and carries VLAN 1 is rejected. The node label is
      the operator's own statement that the port is a node port. The
      controller's own port is only known from the MAC table, so an
      entry without `node` cannot be judged here.
    - Exposure report, at run time: once the controller has found its
      own port, every port of the unit that carries VLAN 1 and is
      neither the configured uplink nor the controller's own port is
      listed in `state.switch.mgmt_exposed` (ascending, left out when
      empty). A change of the list raises `switch_mgmt_exposed` with
      the port list as detail, and `switch_mgmt_closed` when it
      becomes empty. While the own port is not known nothing is
      reported: there is no telling which port to exempt.
    - Report only, like drift: the controller never moves a port out
      of VLAN 1 by itself. The fix is a desired state that makes the
      node ports access ports in a data VLAN, applied through the
      normal reconcile and commit-confirm path.
    - Not enforced in the planner: the revert path plans back to a
      snapshot that may itself have VLAN 1 on node ports (a fresh or
      factory-reset unit has it on every port), and a revert must
      never be refused.
    - No fault bit and no Modbus change: the field and the two events
      are additive.
    - Unconfirmed: that the CPU answers in VLAN 1 only. Test once on
      the bench: an access port in a data VLAN must not reach the web
      UI. Recorded in Granite/infra `docs/sodola.md` as a TODO.

### Config section `switch`

```json
{
  "enabled": false,
  "manage": false,
  "host": "192.168.2.1",
  "mac": "1c:2a:a3:24:26:7e",
  "user": "admin",
  "uplink_port": 9,
  "enforce": false,
  "poll": { "stats_s": 5, "info_s": 60, "mac_s": 120, "drift_s": 600, "counters_s": 60 },
  "desired": {
    "vlans": [ { "id": 100, "name": "data" } ],
    "ports": [
      { "port": 1, "mode": "access", "vlan": 100, "node": 1 },
      { "port": 9, "mode": "trunk", "native": 1, "allowed": [1, 100, 200] }
    ],
    "jumbo": 9216,
    "storm": { "ports": "1-8", "bcast_kbps": 10000, "unknown_ucast_kbps": 50000 },
    "loop": "loop",
    "ip": { "mode": "static", "addr": "10.0.202.5/24", "gw": "10.0.202.1" }
  }
}
```

`desired` is ignored unless `manage` is true. `mac` is the pairing
from pre-provisioning. Absent keys in `desired` mean "do not manage", so a partly described
switch is valid and the rest is left alone. `node` on a port is the
operator's statement of cabling; the MAC table check reports when it
does not hold.

### Data flow

```
             config_set (HTTP/MQTT)          cmd (MQTT)
                     |                           |
                     v                           v
   +----------- dispatcher ------------------------------+
   |   staged/confirmed `switch` section, commands       |
   +-----------------------------+-----------------------+
                                 | channel
                                 v
   +---------------- switch worker thread ---------------+
   |  poll schedule  |  reconcile  |  commands           |
   |        \            |              /                |
   |         +---- client (serial, spaced, re-login) ----+---> HTTP :80 switch
   |        parse (granite-core)   plan (granite-core)   |
   +-----+---------------------------+-------------------+
         | Observed.switch           | events
         v                           v
       MQTT state             MQTT event, switch/counters
```

---

## Alternatives Considered

### Controller-driven commissioning, registration over MQTT
- **The idea**: the controller powers its nodes one by one, binds
  power port -> switch port -> MAC itself and publishes
  `node_discovered`; something registers it.
- **Optimizes for**: a block that commissions itself with nothing else
  running.
- **Sharpest tradeoff**: duplicates the my_metal scan, and my_metal has
  no MQTT and admits admin callers only by mycelium key, so it needs a
  new MQTT-to-RPC gateway with its own auth. The MAC seen on the switch
  is also weaker proof than the provider's "this MAC fetched boot.ipxe
  after that outlet came back".
- **Bets on**: blocks get commissioned somewhere without a my_metal
  provider.

### Central switch service instead of the controller
- **The idea**: one host process (tools/sodola.py logic) reads all 12
  switches of a pod and feeds my_metal directly.
- **Optimizes for**: less firmware (no client, parser, worker).
- **Sharpest tradeoff**: needs the node power events from the
  controllers to bind ports, so it ends up talking to both anyway; one
  more always-on service per pod.
- **Bets on**: firmware flash and effort are scarcer than a pod host.

### Sidecar on a host (the infra Python tool + an MQTT bridge)
- **The idea**: run `tools/sodola.py` logic as a small daemon on some
  always-on machine per site, bridging to MQTT.
- **Optimizes for**: speed of delivery; the client exists and is
  tested.
- **Sharpest tradeoff**: the block is no longer self-contained; a
  block's switch depends on a host outside the block.
- **Bets on**: there is always a reliable host with a path to every
  block's management VLAN.

### Imperative MQTT commands only, no desired state
- **The idea**: expose `add_vlan`, `set_port_vlan`, `set_ip` ... as
  commands; the controller keeps no switch config.
- **Optimizes for**: less code (no planner, no drift check).
- **Sharpest tradeoff**: no commit-confirm story for a change sequence,
  no drift detection, and replacing a dead switch means replaying
  history from somewhere else.
- **Bets on**: switch config is set once and never touched again.

### Replace the switch with one that has SNMP or a real API
- **The idea**: buy a managed switch with SNMP/NETCONF/REST.
- **Optimizes for**: robustness of the management path; push (traps).
- **Sharpest tradeoff**: cost per block, and 8x 2.5G + 10G at this
  price does not come with a real API.
- **Bets on**: per-block switch cost is not a driver.

### Open firmware on the switch (RTLPlayground)
- **The idea**: flash the community firmware and talk to its API.
- **Optimizes for**: a cleaner management surface we control.
- **Sharpest tradeoff**: per-board-revision breakage (LEDs, dead SFP,
  boot failures reported), no management VLAN or TLS yet, and recovery
  needs a SOIC clip and a flash dump per unit.
- **Bets on**: the project matures and our board revisions are covered.

---

## Consequences

### Positive
- The block reports its own network: link changes, which node sits on
  which port, throughput, switch reboots.
- Switch config is versioned with the controller config and survives
  replacing the switch (re-apply desired state onto a fresh unit).
- Every risky switch change gets the same commit-confirm safety net as
  the controller's own network changes.
- The password stops being admin/admin without anyone typing one.

### Negative
- Screen-scraping a vendor UI: a firmware update can break parsers.
  Mitigated by the fingerprint check and fixtures per firmware.
- Detection latency is the poll period (5 s default); there is no push.
- Roughly 900-1200 lines of Rust and 30-80 KB of flash, plus fixtures;
  the read-only part (client, parsers, worker, pairing) is about half.
- A change in another repo (my_metal `Drv::Granite`, `switch_port` in
  `NodeView`) is needed before commissioning works end to end.
- The switch credentials sit on the controller; anyone with the
  controller's secrets has the switch.

### Risks
- **The switch web server degrades under polling** (hangs, leaks,
  reboots). Mitigation: the soak test (`tools/sodola_soak.py`, 1 s
  stats + 10 s info/port + 60 s MAC walk + 5 min write); defaults are
  5x slower than the soak. Result 2026-11-11, 3 h, about 12,900
  requests: 0 errors, 0 lost logins, 0 reboots, latency flat (stats
  median 390 ms, max 436 ms in every 30 min bucket). Not covered: a
  human on the web UI at the same time, runs longer than 3 h.
- **Lock-out**: a switch IP or VLAN change that cuts the controller off.
  Mitigation: protected ports, commit-confirm, revert by undo or, when
  the switch is unreachable, by reboot.
- **A human on the web UI at the same time**: shared paging/search
  state can confuse a MAC walk; forms are last-writer-wins. Mitigation:
  the walk resets the filter with a GET first; drift detection reports
  manual changes; document "do not use the web UI on managed switches".
- **CVEs**: cleartext credentials and CSRF. Mitigation: the switch's
  management address is reachable only on the controller's management
  VLAN; the controller never runs a browser, so CSRF needs a human
  browser on that VLAN.

---

## What an Expert Would Ask

**Q: What happens when the switch hangs or its web server dies, but
keeps forwarding?**
A: Forwarding is in the switch ASIC; the web server runs on the
embedded 8051-class CPU. The worker sees timeouts, raises
`switch_unreachable` after 3 polls, and keeps retrying with backoff.
It does not reboot the switch on its own: a reboot interrupts every
node for about 3 s, so that stays an explicit `switch_reboot`. Whether
the web server hangs at all under our load is what the soak test
answers; if it does, the polling defaults go up and an optional
"reboot after N minutes unreachable" setting gets added, off by
default.

**Q: Desired state says VLAN 100 on port 3, but someone changed it on
the web UI. Who wins?**
A: With `enforce = false` (default) nobody automatically: the drift
check raises `switch_drift` with the difference and leaves the switch
alone. With `enforce = true` the reconcile puts the desired state back
(applied and saved at once, no confirm, since the desired state was
already confirmed). Default off because silently undoing a human's
emergency change is worse than reporting it.

**Q: How does the controller know which port is its own, and what if
it is wrong?**
A: Its Ethernet MAC (ADR 0001 component 6) appears in the switch MAC
table on exactly one port. That port and `uplink_port` are protected.
If the controller's MAC is not in the table (aged out, just booted),
the protection falls back to "every port is protected" for VLAN and
enable changes until it is found; a forced command still works. If the
controller hangs off another switch upstream, its MAC shows on the
uplink port, which is protected anyway.

**Q: Why does the controller bind ports at all, if my_metal's scan
already proves which MAC is on which outlet?**
A: The scan proves outlet -> MAC. It cannot see the block switch: the
router sees all 8 nodes on its one port toward the pod ToR. The switch
port matters for cabling faults (two cables swapped between nodes look
fine to the scan), for port-level telemetry per node, and for
disabling a node's port. The binding costs no extra node actions: it
rides on power changes that happen anyway.

**Q: What if the scan runs while the controller's own rules also switch
nodes?**
A: The provider's `power` calls go through the same dispatcher and
queue as every other node command, so they serialise with rule actions.
A rule that turns a node back on during a scan would confuse the scan's
timing; the provider's outlet scan should be run with the block's
rules paused. Open question whether the controller should pause rules
itself when a `granite` PDU caller sends a cycle with a scan flag.

**Q: Why not always revert by rebooting the switch? It is simpler.**
A: A reboot interrupts every port for about 3 s, including nodes the
change never touched, so it is the fallback, not the default. While the
switch is reachable the undo is the same planner run in reverse (current
-> snapshot), which costs nothing new. The undo cannot help when the
change cut the controller off, and that is exactly when the reboot is
used. Both paths depend on "never save before confirm"; that is the
invariant the code must hold. One gap: an undo that half-applies and
then loses the switch is caught by the read-back check and falls
through to the reboot.

**Q: How do the parsers cope with firmware 200.1.16 / 200.1.20 /
200.2.4, which exist and which we have not seen?**
A: Not handled beyond refusing writes on an unknown version. Adding a
version means capturing its pages into a new fixture directory and
running the parser tests against them. Until then the switch is
monitored if the pages parse and otherwise reported as unsupported.

---

## Implementation Plan

### Decisions you will probably want to tweak

- **Config shape**: desired state inside the controller config section
  `switch`, partial descriptions allowed.
  - Alternative: a separate document pushed only over MQTT.
  - Cost to change later: medium; config schema version bump and an
    import migration.
- **MQTT surface**: switch summary inside `state`, counters on a new
  `.../switch/counters` topic, events through `.../event`.
  - Alternative: a separate retained `.../switch` state topic.
  - Cost to change later: medium; consumers have to follow.
- **Revert by undo, reboot as fallback** (user decision 2026-11-11).
  - Alternative: always reboot (simpler, interrupts every port).
  - Cost to change later: low inside the worker, but the "never save
    before confirm" invariant shapes the whole reconcile.
- **Drift is report-only; `enforce` defaults to false** (user decision
  2026-11-11).
  - Alternative: true, the controller is the source of truth.
  - Cost to change later: trivial (a default).
- **Read-only by default, `manage` opt-in** (user direction 2026-11-12).
  - Alternative: the controller always owns the switch config.
  - Cost to change later: trivial; a default, the code is there.
- **The controller is a my_metal PDU (`kind = "granite"`)**, using the
  existing HTTP API, rather than a new my_metal device type.
  - Alternative: a dedicated "block" entity in my_metal (controller +
    switch + nodes).
  - Cost to change later: medium; my_metal schema and UI change, the
    controller API stays the same.
- **Poll defaults** (5 s stats, 60 s info, 120 s MAC, 10 min drift).
  - Cost to change later: trivial; set from the soak result.

### Known unknowns and how the plan absorbs them

- **Soak result.** Answered 2026-11-11: 3 h clean (see Risks), so the
  periods above stand and the optional auto-reboot is not added. Pivot
  signal now: `switch_unreachable` events in the field. Note each stats
  page costs about 0.4 s of switch time, so the worker does at most one
  request at a time and never polls faster than the soak did.
- **Restore upload.** Default: backup only. Pivot signal: a tested
  restore on the unit -> add `switch_restore` (admin HTTP only).
- **Newer firmware, other models.** Default: SL902-SWTGW218AS on
  V200.1.8 only. Pivot signal: a unit arrives with another model or
  version -> a new profile row with its own fixtures before enabling
  writes (component 24).
- **Adoption of a factory-fresh switch** (192.168.2.1, needs a
  192.168.2.0/24 address). Default: switches are pre-provisioned with
  `tools/sodola.py` (IP, password) before they go into a block. Pivot
  signal: provisioning by hand becomes a bottleneck -> an adoption mode
  with a temporary second address on the controller (needs lwIP/netif
  work, its own ADR).
- **Node MACs.** Answered 2026-11-12: the node-port map is built from
  the controller's own power actions (component 22) and MACs come from
  the switch; my_metal's scan decides which MAC is the machine. `node`
  on a port in `desired` stays an optional cabling statement.
- **Standby link.** Default: link edges, with MAC-appearance fallback.
  Pivot signal: most of our nodes keep link while off -> bind on MAC
  appearance only and lengthen `t_link_s`.

### The mechanical work

- Capture fixtures for every page used (V200.1.8) into granite-core.
- granite-core: model, scanner, page parsers, form builders, planner,
  message types; host tests against the fixtures, planner tests for
  ordering and protection.
- granite-fw: client, worker thread, creds, backup, config section with
  staging/confirm hooks, MQTT and HTTP glue, faults in the fault word
  (`switch_unreachable`, `switch_fw_unknown`).
- granite-sim: a fake switch serving the fixtures over HTTP for
  integration tests and UI work.

### Review asks (answered 2026-11-11)

1. Desired state in the controller config with commit-confirm: **yes**,
   with revert by undo while the switch is reachable and reboot only as
   the fallback.
2. `enforce` default: **false**, drift is reported, not fixed.
3. Counters on their own topic `.../switch/counters`: **yes**.
4. Pre-provision switches with the infra script: **yes**; no adoption
   mode for now.

---

## Implementation (2026-11-12)

What is in the firmware, where, and where it differs from the plan
above. The differences are all reversible; they are recorded here
rather than left in the code only.

### Built

- **Component 15** in `granite-core/src/switch/`: `scan.rs` (the
  hand-written scanner), `model.rs`, `parse.rs`, `forms.rs`, `plan.rs`,
  `counters.rs`, `obs.rs`, `md5.rs`. Fixtures for every page used are
  in `granite-core/tests/fixtures/sl902-swtgw218as-v200.1.8/`, captured
  read-only from the bench unit; `granite-core/tests/switch.rs` parses all of
  them, checks that each parser **rejects** the other fourteen pages,
  and plans this ADR's own example plus its revert.
- **Component 16** in `granite-fw/src/switch/client.rs`: one
  `EspHttpConnection` per request, cookie and `Referer`, one re-login
  on the login redirect, 100 ms minimum spacing, 10 s timeout, the 2 s
  settle after a port write, and "a timeout is the expected answer" for
  the STP and address writes. Pages are capped at 48 KB and dropped
  after their parse. The firmware fingerprint is checked at every
  `info.cgi`: an unknown version is monitored while its pages parse and
  never written.
- **Component 17** in `granite-fw/src/switch/mod.rs`: the worker
  thread, the poll schedule from `switch.poll`, reachability after
  three failed polls with an exponential retry, counter rates with a
  decrease treated as a reset, and the reboot check on `info.cgi`
  uptime.
- **Component 18**: the `switch` config section goes through the
  existing staging and commit-confirm path
  (`Section::affects_reachability` is true for it), so `config_set`
  stages it over MQTT and HTTP alike. The worker applies the plan
  without saving and keeps the snapshot; a confirm (`promote`) is the
  only caller of `save.cgi`; the deadline's revert plans back to the
  snapshot and falls through to a switch reboot when that fails or the
  switch is gone.
- **Component 21**: `state.switch`, the `.../switch/counters` topic,
  the commands `switch_port`, `switch_reconcile`, `switch_save`,
  `switch_reboot` and `switch_factory_reset`. A staged switch change is
  confirmed or reverted with `config_confirm` / `config_revert`
  (added to ADR 0001 components 6 and 7 for this: the confirm used to
  exist only as an HTTP route, which would have made every switch
  change from an MQTT-only client revert after `t_confirm_s`).
- **Component 22, the pairing check only**: a switch whose `info.cgi`
  MAC is not `switch.mac` raises `switch_mismatch` and is not talked to
  beyond `info.cgi`.

### Differences from the plan above

- **Events are one kind, not nine.** `EventKind::Switch { what, port,
  node, detail }`, exactly like `EventKind::Plane`, with `what` one of
  `switch_port`, `switch_reachable`, `switch_unreachable`,
  `switch_reboot`, `switch_drift`, `switch_reconciled`,
  `switch_reverting`, `switch_reverted`, `switch_refused`,
  `switch_mismatch`, `switch_fw_unknown`, `switch_saved`,
  `switch_factory_reset`. It follows the vocabulary ADR 0001 already
  has and keeps `msg.rs` from growing nine near-identical variants.
  `switch_refused` is new and carries every reason a write was not
  attempted (not managed, protected port, unknown firmware, a refusal
  from the planner).
- **MD5 is written out** in `granite-core/src/switch/md5.rs` rather
  than pulled in as a crate: the cookie is a payload like any other, it
  is built and tested on the host (RFC 1321 vectors), and no crate
  lands in the flash budget for it. It is not used as a security
  primitive; the scheme it serves is CVE-2026-27754.
- **The MAC walk posts in read-only mode too.** The paging is
  server-side state, so the walk is a GET (which drops a stuck search
  filter) plus `cmd=perpage` / `cmd=goto` POSTs. Those move the table's
  view and no configuration. The alternative - reading only the first
  ten rows - would lose the node-to-port map, which is the main thing
  the read-only mode is for. So "never POSTs except to log in" now
  reads "except to log in and to page the MAC table".
- **Two Modbus fault bits**, not the names in component 21: bit 8
  `switch_unreachable`, bit 9 `switch_unsupported` (unknown firmware,
  a page that does not parse, or a MAC mismatch).
- **`switch_port` resolves a node through `switch.desired.ports[].node`**,
  because the link-edge binding of component 22 is not built yet. A
  command that names a node the configuration does not place is refused
  by the dispatcher before any traffic.
- **The counters payload goes through a slot**: the worker builds it
  and the MQTT worker publishes it on its next pass, because the broker
  session belongs to the MQTT worker. Nothing queues up while there is
  no session.
- **`switch_reconcile` applies without saving**, like a staged change;
  `switch_save` is what persists. That keeps the "nothing is saved
  before a confirm" invariant true on every path.
- **Jumbo `0` means off** in `desired`, and a storm rate of `0` means
  unlimited (which is an empty field on the wire). An absent key still
  means "do not manage".
- **The drift pass reads the storm page only when `desired` mentions
  storm**: two more 400 ms pages per 10 minutes otherwise buy nothing.
- **Flash cost**: +159,856 bytes (1,706,432 to 1,866,288, 46.7 % of
  the slot), against the 30-80 KB this ADR guessed at. Roughly half is
  the parsers, the planner and their strings; the rest is the `switch`
  config section's serde and the worker.
- **No schema bump.** `SCHEMA_VERSION` stays 1: a document without a
  `switch` section loads with the section at its default, which is
  "off", so an older export needs no migration.

### Not built yet

- **Component 19** (password rotation) and **component 20** (config
  backup). `Secrets::switch_password` exists and is used - empty means
  the factory `admin` - so rotation is a command and a `user.cgi` form
  away.
- **Component 22's node-port binding** from link edges around a power
  change, `switch_node_moved` and `switch_node_unbound`. The MAC table
  is walked and published, so the map is there as an operator
  statement, not yet as something the controller proves.
- **Component 23** (the my_metal `/api/v1/switch/...` endpoints).
- **Desired-state keys not covered**: rate limits (read and published,
  not managed), port isolation, LAG, mirroring, IGMP snooping, STP
  parameters beyond the `loop` mode, and the device name. The pages for
  rate limits are parsed; the others are captured as fixtures but have
  no parser.
- **The granite-sim fake switch.** The simulator records a switch
  request and logs that it is not simulated; it has no HTTP server
  serving the fixtures, because the worker that would drive it lives in
  `granite-fw` and does not build on the host.

### Implementation (2026-11-15): switch profiles

Component 24 is built. `granite-core/src/switch/profile.rs` holds
`Layout` (port count plus which ports are SFP+, `is_port`, `is_sfp`,
`widest`), `Dialect::SodolaV200`, `Tier::{Verified, ReadOnly}`,
`Profile` with `fixture_dir()`, the `PROFILES` table with its one row
(SL902-SWTGW218AS / V200.1.8 / 9 ports, SFP+ 9 / `SodolaV200` /
`Verified`) and `lookup(model, firmware)`, trimmed and exact on both.

`PORT_COUNT`, `SFP_PORT`, `KNOWN_FIRMWARE`, `is_port` and
`firmware_known` are gone. A `&Layout` is threaded through instead: the
per-port page parsers, `forms::rate_max`, `Speed::impossible_on`,
`parse_port_list`, every `plan.rs` entry point (`plan`, `drift`, `undo`,
`undo_desired`, `port_enable_op`, `Protected::new`) and
`SwitchObs::of_state`. Config-time checks (`Config::validate`,
`SwitchCfg::uplink`, the `switch_port` dispatcher,
`SwitchDesired::validate`) use `Layout::widest()`, the unit's own layout
being unknown there. `switch.uplink_port` keeps its default of 9 as a
documented plain default (`config::DEFAULT_UPLINK_PORT`), not as a fact
about the unit.

The worker resolves the profile at every `info.cgi`
(`Worker::resolve_profile`) and hands the layout to the client, which
carries it for its page parses. Writes need a `Verified` row; no row and
a `ReadOnly` row behave as before (monitor, refuse writes,
`switch_fw_unknown`, `SWITCH_UNSUPPORTED` fault), the event detail is
now `model firmware` and the log line says which of the two cases it
is. The planner refuses an op on a port the resolved layout has not got
("this switch has no port N: it has M port(s)"), and an `uplink_port`
outside the layout protects nothing and is logged once. Wire formats are
unchanged: MQTT `state.switch`, the obs field names (`fw_unknown` kept),
the event names and the Modbus map all stay as they were.

Fixtures moved to `tests/fixtures/sl902-swtgw218as-v200.1.8/` and
`tests/switch.rs` now walks the table: every row must have its
directory, its `info.html` must parse to the row's own (model,
firmware) and resolve back to that row, and every page must parse with
the row's layout, with as many per-port rows as the layout has ports.

Differences from component 24 as written:

- **`parse::info` takes no layout.** It is the page the profile is
  resolved from, so there is nothing to pass yet; it reads its port
  rows with `Layout::widest()` and the narrower layout applies from the
  next page on. Everything else takes the layout explicitly.
- **The layout travels in the client**, as a field set by the worker
  (`SwitchClient::set_layout`), rather than as an argument to every
  `read_*` call. One owner of the current layout per connection, and no
  call site can pass the wrong one.
- **No `Dialect` dispatch**, as the component asks: the field exists and
  is read nowhere. `parse.rs` and `forms.rs` are `SodolaV200`.

### Implementation (2026-11-15): VLAN 1 invariant

Component 25 is built. `granite-core/src/switch/mgmt.rs` holds the pure
logic: `MGMT_VLAN` (1), `port_carries_mgmt(&PortVlan)` and
`desired_carries_mgmt(&DesiredPort)`, `exposed(layout, state, uplink,
own) -> Option<Vec<u8>>` and `port_list(&[u8])` for the event detail.
`exposed` is `None` while the controller's own port is unknown or
`vlan.cgi?page=port_based` has not been read, and otherwise the
ascending ports that carry VLAN 1 minus the exempt pair; the pair is
`plan::Protected::new`'s, so "the uplink and the controller's own port"
has one definition in the image and an uplink outside the layout
exempts nothing.

The config check is in `SwitchDesired::validate`: an entry with a
`node` that carries VLAN 1 is an error ("a node port may not carry VLAN
1, the switch's management VLAN"). The worker
(`granite-fw/src/switch/mod.rs`) keeps `mgmt_exposed: Vec<u8>` and
recomputes it in `check_mgmt_exposure` after the MAC walk, after the
drift read, after a reconcile's read-back and on a config change that
moves the uplink; on a change it publishes
`state.switch.mgmt_exposed` (left out when empty) and raises
`switch_mgmt_exposed` with the port list as detail, or
`switch_mgmt_closed` when the list empties, with one `warn` line on
exposure. No write, no fault bit, no Modbus change, nothing in the
planner, and it does not depend on `manage`: the configuration pages
are read either way.

Differences from component 25 as written:

- **A desired trunk is judged on what it names, and the trunk write
  now says the same.** `forms::port_trunk` used to add VLAN 1 to every
  trunk, on the belief that the web UI insists on it, so a `node`
  trunk that named no VLAN 1 passed the check and still ended up
  carrying it. The bench unit accepts a trunk without VLAN 1 (port 1,
  native 200, allowed 300 and 400, 2026-11-15), so the form and the
  planner now put VLAN 1 on a trunk only when `native` or `allowed`
  names it. An uplink entry must therefore name it.
- **The event detail is a plain comma list** (`"2,3,4"`), not the range
  form `desired.storm.ports` uses: it is read by a human and parsed by
  nothing.
- **`None` keeps the last report** rather than clearing it: the list is
  only reset when the worker starts talking to another switch. A
  cleared list would mean "closed", which is not what "no telling"
  means.
- **The bench check of the unconfirmed assumption is still open**: that
  the CPU answers in VLAN 1 only (an access port in a data VLAN must
  not reach the web UI) was not tested here, no hardware was touched.

## Open Questions

**Architecture-changers**
- [ ] Management network: which VLAN carries the controller and the
      switch management address, and is the switch's VLAN 1 mapped to
      it through the upstream PVID (the workaround in docs/sodola.md)?
      This decides what "protected" must cover.
- [ ] my_metal side: who owns `Drv::Granite` and the `switch_port`
      field in `NodeView`, and is a `granite` PDU one controller per
      `LanPdu` (12 per pod), or should my_metal grow a block entity?
- [ ] API token for my_metal: one shared token across all controllers
      of a provider (my_metal's per-kind credential model), or
      per-controller tokens (needs a my_metal change)?

**Behavior definers**
- [ ] Should the controller pause its rules while my_metal scans its
      outlets, and how does it know a scan is running?
- [ ] Auto-reboot the switch after N minutes unreachable: wanted at
      all, and what N?
- [ ] Should `switch_port` disable of a node also power-cycle or
      soft-off the node (actuator), or stay network-only?
- [ ] Storm control and loop detection as defaults applied on first
      reconcile, or only when desired state says so?
