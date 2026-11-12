# The block switch: operator guide

How to read and configure a block's Sodola SL902-SWTGW218AS through the
controller, over MQTT (or the HTTPS API, which takes the same
commands). Design and reasons: `docs/adr/0006-block-switch.md`. The
switch itself, its quirks and a stand-alone CLI: Granite infra
`docs/sodola.md` and `tools/sodola.py`.

Topics below use the bench prefix `granite/bench/granite-cb374b`;
replace it with your controller's `<prefix>/<device id>`. Examples use
`mosquitto_pub` / `mosquitto_sub` against a broker on 127.0.0.1.

## 1. The three rules

1. **`config_set` replaces the whole `switch` section.** It is not a
   patch. Every key you leave out goes back to its default
   (`enabled: false`, `manage: false`, empty `host`, empty `desired`
   ...). Always start from `config_get` and send the edited result back.
2. **`desired` lists are complete where the guide says so.** `vlans` is
   the full VLAN table: a VLAN not in it is deleted. `ports` is not:
   a port not listed is left alone.
3. **Nothing is kept until you confirm.** A `config_set` is applied to
   the switch's running config but not saved. `config_confirm` saves
   it; `config_revert`, or 300 s without a confirm, undoes it.

## 2. Watch while you work

Keep this running in a second terminal:

```sh
mosquitto_sub -h 127.0.0.1 -v \
  -t 'granite/bench/granite-cb374b/event' \
  -t 'granite/bench/granite-cb374b/ack/#'
```

Every command gets an ack on `.../ack/<id>`; what the switch worker
then does arrives as events (`kind:"switch"`, section 7).

## 3. Turning it on

### Read-only (monitoring)

```sh
mosquitto_pub -h 127.0.0.1 -t granite/bench/granite-cb374b/cmd -m '
{"v":1,"id":"sw-on","action":"config_set","args":{"section":"switch","config":{
  "enabled":true,"manage":false,
  "host":"192.168.64.241","mac":"1c:2a:a3:24:26:7e","uplink_port":9}}}'
mosquitto_pub -h 127.0.0.1 -t granite/bench/granite-cb374b/cmd -m \
  '{"v":1,"id":"sw-on-ok","action":"config_confirm"}'
```

The controller logs in, polls, and fills `state.switch` (section 8).
With `manage: false` it never POSTs anything but the login.

### Read-write

Same, with `"manage": true`. Only then is `desired` applied and are
the write commands (`switch_port`, `switch_save`, ...) accepted.

The switch password is not in this section; it is a controller secret
(default `admin`, ADR 0006 component 19, rotation not built yet).

## 4. The `switch` section, every key

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | Talk to the switch at all. |
| `manage` | `false` | Allow writes. Without it `desired` is ignored and write commands are refused. |
| `host` | `""` | Management address. Empty = do not poll. |
| `mac` | `""` | The switch's MAC from provisioning. If set and the switch reports another MAC, the controller stops after `info.cgi` and raises `switch_mismatch`. Empty = no check. |
| `user` | `"admin"` | Login user. |
| `uplink_port` | `9` | Protected port (section 6). `0` = none. |
| `enforce` | `false` | On drift, put `desired` back by itself (and save). Off: drift is only reported. |
| `poll.stats_s` | `5` | Link state and counters. |
| `poll.info_s` | `60` | Uptime, firmware, MAC; a falling uptime = switch rebooted. |
| `poll.mac_s` | `120` | MAC table walk (also after a link change). |
| `poll.drift_s` | `600` | Compare the switch with `desired` (only with `manage`). |
| `poll.counters_s` | `60` | How often `.../switch/counters` is published. |
| `desired` | `{}` | What the switch should look like (below). |

### `desired`

Every key is optional. Absent = not managed, left as it is.

| Key | Type | Meaning |
|---|---|---|
| `vlans` | list of `{id, name}` | **The complete VLAN table.** Ids 1-4094, names up to 14 characters. A VLAN not listed is deleted, after no port uses it. VLAN 1 is never deleted. Leave the key out to not manage VLANs at all. |
| `ports` | list of port objects | Per-port settings, only for the ports listed. |
| `jumbo` | number | Max frame size: `1522`, `1536`, `1552`, `9216`, `10000`, `12000`, or `0` for off. |
| `storm` | object | Storm control (below). |
| `loop` | `"off"` / `"loop"` / `"stp"` | Loop prevention. Turning STP on flaps the uplink for about 10 s. |
| `ip` | object | The switch's management address (below). Applied last. |

Port object (only `port` is required; every other key is managed only
when present):

| Key | Values | Meaning |
|---|---|---|
| `port` | 1-9 | Port 9 is the SFP+. |
| `mode` | `"access"` / `"trunk"` | |
| `vlan` | VLAN id | Access VLAN. Required with `mode: "access"`. |
| `native` | VLAN id | Untagged VLAN of a trunk. Required with `mode: "trunk"`. |
| `allowed` | list of ids | Tagged VLANs a trunk carries. The native VLAN is always added. |
| `enabled` | bool | Admin state. |
| `speed` | `"auto"`, `"10m"`, `"100m"`, `"1g"`, `"2.5g"`, `"10g"` | `10g` only on port 9; `10m` not on port 9. |
| `node` | node id | Which node is cabled here. A statement, never written to the switch; used by `switch_port {"node":N}`. |

`storm`: `{"ports":"1-8", "bcast_kbps":N, "mcast_kbps":N,
"unknown_ucast_kbps":N, "unknown_mcast_kbps":N}`. `ports` is a range
list (`"1-4,7"`); each limit is optional and left alone when absent.
The switch's form fields for the two "unknown" limits look swapped
against its own labels; the controller follows the labels (TODO in
infra `docs/sodola.md`: confirm with traffic).

`ip`: `{"mode":"static","addr":"192.168.64.241/24","gw":"192.168.64.254"}`
or `{"mode":"dhcp"}`. The controller follows the switch to the new
address for its verify step. Get this wrong and confirm it, and only a
console on the switch or a factory reset brings it back: let the
confirm window run out instead if the switch stops answering.

A `config_set` that breaks one of the rules above is refused as a whole:
the ack has `"ok":false` and every problem in `error`. Nothing is
staged.

## 5. The cycle: stage, look, confirm (or not)

```
config_set ----> ack "staged"
                 event config staged
                 event switch_reconciled  (detail: the ops applied)
                 ... the switch now runs the change, NOT saved ...
config_confirm -> ack "confirmed", data ["switch"]
                 event switch_saved        (now in the switch's flash)
  or
config_revert --> ack "reverted"
  or 300 s pass   event switch_reverting  (detail: the undo ops)
                 event switch_reverted     detail "undo"
                 event config reverted     detail "reverted on request"
                                                or "not confirmed in time"
```

- The window is `net.t_confirm_s` (default 300 s), shared with any other
  staged section (`net`, `mqtt`, `sec`): one confirm or revert closes
  them all, and the ack's `data` lists which.
- The undo plans from what the switch has now back to what it had
  before the change. If the switch cannot be reached, or the undo does
  not read back right, the controller reboots the switch instead
  (`switch_reverted` detail `"reboot"`): every port drops for about 3 s
  and the switch comes back with its last saved (= last confirmed)
  config.
- Nothing is ever saved to the switch before a confirm, so power-cycling
  the switch is always a valid way back too.

## 6. Protected ports

The uplink (`uplink_port`) and the port the controller's own MAC is
seen on are protected: a VLAN or admin-state change to them is refused
(`switch_refused` event with the reason) and the rest of the change is
applied. While the controller has not found its own MAC in the table
yet, every port counts as protected for those changes.

`desired` cannot override this. A one-off command can: `switch_port`
with `"force": true`.

## 7. Recipes

All recipes show only `desired`; wrap it in the full section (rule 1).
A complete message for the first one:

```sh
mosquitto_pub -h 127.0.0.1 -t granite/bench/granite-cb374b/cmd -m '
{"v":1,"id":"r1","action":"config_set","args":{"section":"switch","config":{
  "enabled":true,"manage":true,
  "host":"192.168.64.241","mac":"1c:2a:a3:24:26:7e","uplink_port":9,
  "desired":{
    "vlans":[{"id":1},{"id":100,"name":"data"},{"id":200,"name":"backend"},
             {"id":300,"name":"storage"}]}}}}'
```

then `{"v":1,"id":"r1-ok","action":"config_confirm"}`.

**Add a VLAN** - add it to `vlans`, keep the others (a missing one is
deleted):

```json
"vlans":[{"id":1},{"id":100,"name":"data"},{"id":200,"name":"backend"},{"id":300,"name":"storage"}]
```

**Remove a VLAN** - drop it from `vlans`, and move every port that uses
it in the same change. The VLAN is deleted after the ports moved:

```json
"vlans":[{"id":1},{"id":100,"name":"data"},{"id":200,"name":"backend"}],
"ports":[{"port":7,"mode":"trunk","native":1,"allowed":[1]}]
```

**A node on an access port**:

```json
"ports":[{"port":3,"mode":"access","vlan":100,"node":3}]
```

**A node on a trunk** (untagged 100, tagged 200 and 300):

```json
"ports":[{"port":3,"mode":"trunk","native":100,"allowed":[200,300],"node":3}]
```

**Disable a port** (persistent, through config):

```json
"ports":[{"port":4,"enabled":false}]
```

For a quick, temporary disable use the command in section 8 instead.

**Force a speed**: `"ports":[{"port":2,"speed":"1g"}]`; back to
`"auto"` the same way.

**Jumbo frames**: `"jumbo":9216`; off: `"jumbo":0`.

**Storm control on the node ports**:

```json
"storm":{"ports":"1-8","bcast_kbps":10000,"unknown_ucast_kbps":50000}
```

**Loop detection**: `"loop":"loop"`.

**Stop managing something**: remove its key. The switch keeps whatever
it has; the controller just stops looking at it.

## 8. Commands

On `.../cmd`. All need `enabled` and `manage`; the ack is `accepted`,
the outcome comes as events.

| Action | Args | Does |
|---|---|---|
| `switch_port` | `{"port":N}` or `{"node":N}`, `"enabled":bool`, optional `"force":true` | Enable or disable one port now. Not saved: lost on a switch reboot unless followed by `switch_save`. `node` needs `"node"` on that port in `desired`. |
| `switch_save` | - | Write the switch's running config to its flash. |
| `switch_reconcile` | - | Apply `desired` again now (never saves). |
| `switch_reboot` | `"confirm":"<device id>"` | Reboot the switch: every port down about 3 s, unsaved changes lost. |
| `switch_factory_reset` | `"confirm":"<device id>"` | Back to 192.168.2.1, admin/admin. The controller loses it until it is provisioned again. |
| `config_get` | `{"section":"switch"}` | The section as it runs now, a staged change included. |
| `config_confirm` / `config_revert` | - | Section 5. |

Example:

```sh
mosquitto_pub -h 127.0.0.1 -t granite/bench/granite-cb374b/cmd \
  -m '{"v":1,"id":"p4off","action":"switch_port","args":{"port":4,"enabled":false}}'
```

## 9. What the controller publishes

`.../state` (retained) has `switch`:

```json
{"reachable":true,"fw":"V200.1.8","uptime_s":5369,"mac":"1C:2A:A3:24:26:7E",
 "managed":true,"last_reconcile":343212,
 "ports":[{"port":7,"enabled":true,"link":false,"speed":"auto",
           "vlan_mode":"access","vlan":300,"node":7,"macs":["..."]}]}
```

`.../switch/counters` (not retained, every `poll.counters_s`): per port
`tx_packets`, `rx_packets`, `tx_bytes`, `rx_bytes` (totals since the
switch's last clear) and `tx_pps`, `rx_pps`, `tx_bps`, `rx_bps` over the
interval. The switch counts good frames only; there are no error or
drop counters.

Events (`.../event`, `kind:"switch"`, the name in `what`):

| `what` | When |
|---|---|
| `switch_port` | A port's link went up or down, or its speed changed. |
| `switch_reachable` / `switch_unreachable` | First good poll / three failed polls in a row. |
| `switch_reboot` | The switch rebooted (asked for, or seen from its uptime). |
| `switch_reconciled` | A change was applied; `detail` lists the ops. |
| `switch_saved` | Written to the switch's flash. |
| `switch_reverting` / `switch_reverted` | An undo is applied / done (`undo` or `reboot`). |
| `switch_drift` | The switch no longer matches `desired` (someone used its web UI?). |
| `switch_refused` | An op or command was refused; `detail` says why. |
| `switch_mismatch` | The switch's MAC is not `switch.mac`. |
| `switch_fw_unknown` | Firmware other than V200.1.8: monitored, never written. |

`switch_unreachable` and `switch_mismatch` / `switch_fw_unknown` also
set Modbus fault bits (`docs/modbus_map.md`).

## 10. Troubleshooting

| You see | Means / do |
|---|---|
| Ack `"switch.manage is false ..."` | Set `manage: true` (whole section, rule 1). |
| A VLAN disappeared after a change | It was not in `vlans` (rule 2). Re-add it, or revert before confirming. |
| `host` or `mac` went empty | The `config_set` left them out (rule 1). |
| `switch_refused` on port 9 | Uplink protection (section 6). |
| Change undone after 5 minutes | No `config_confirm` was sent. |
| `switch_mismatch` | Wrong switch at `host`, or `mac` mistyped. |
| `state.switch.reachable` false | Wrong `host`, switch off, or not on a subnet the controller can reach; the switch has no management VLAN setting. |
| Someone used the switch web UI | Expect `switch_drift`; with `enforce: false` nothing is put back. Avoid the web UI on a managed switch: it shares paging state with the controller's MAC walk. |

## 11. Not built yet

Password rotation and config backup (ADR 0006 components 19, 20), the
node-to-port binding from power actions (22), the my_metal endpoints
(23), and rate limits, port isolation, LAG, mirroring and IGMP as
`desired` keys. For those, use infra `tools/sodola.py` directly.
