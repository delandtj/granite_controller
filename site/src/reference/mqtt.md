# MQTT topics and commands

Topic base: `<topic_root>/<site>/<device>`, by default
`granite/default/granite-xxxxxx`. Every payload is JSON and carries
`"v":1`. The topics are the same as on the rev C controller; the
`state` payload, the node verbs and the events changed with the plane
model.

## Topics

| Topic | Direction | Retained | Payload |
|---|---|---|---|
| `<base>/status` | out | yes | `{"v":1,"online":true,"fw":"0.1.0","ip":"..","uptime_s":N,"boot_reason":"..","ota":"<slot state>"}`; the last will writes `{"v":1,"online":false}` |
| `<base>/state` | out | yes | Full snapshot, on change (coalesced 200 ms) and every `t_state_s` |
| `<base>/event` | out | no | One event object per message |
| `<base>/log` | out | no | One JSON line per log record at or above `sys.log_level` |
| `<base>/cmd` | in | | A command object |
| `<base>/ack/<id>` | out | no | The reply, published when the command **completes** |

`boot_reason` is one of `power_on`, `software`, `task_watchdog`,
`int_watchdog`, `brown_out`, `panic`, `unknown`.

## State payload

```json
{"v":1,"ts":13471,
 "nodes":[{"node":9,"name":"web-1","plane":1,"slot":0,"unit":0,
           "power":"on","host":"on",
           "v":48.0,"a":1.3,"w":62,"w_min":62,"w_max":62,"temp_c":25,
           "energy_wh":0,"module_class":5,
           "last_job":{"id":"api-11","job":3,"verb":"clear_latch",
                       "started_ms":20652,"finished_ms":20802,"result":"done"}}, "..."],
 "planes":[{"plane":1,"address":49,"type":"pp48_chained","firmware":"0.1.0",
            "bus_v":48.0,"temp_c":25,"boot_count":1,"uptime_s":12,"image_state":"confirmed",
            "units":[{"unit":0,"present":true,"v":48.0,"a":2.0,"w":96,"temp_c":25},
                     {"unit":1,"present":false,"v":0.0,"a":0.0,"w":0,"temp_c":0}, "..."]}],
 "probes":[{"slot":1,"rom":"28ff000000000001","name":"inlet","temp_c":25.37}],
 "board_temp_c":34.0,"vin_v":12.0,
 "dry_in":[false,false,false,false],
 "link_up":true,"mqtt_connected":true,"uptime_s":4242}
```

`nodes[]` lists every **present** node in id order, and only those: a
slot with no module, behind a missing unit board or on a plane that
does not answer is left out. Do not index it by position; select by
`node`.

| Field | Meaning |
|---|---|
| `node` | Node id 1..64, `8 x (plane address - 0x30) + slot + 1`. |
| `name` | The configured name, or `node<N>`. |
| `plane`, `slot`, `unit` | Where it sits: plane 0..7, slot 0..7, unit board 0..3. |
| `power` | The rail: `off`, `on`, `starting`, `fault`, `absent`, `unknown`. |
| `host` | The host LED: `off`, `on`, `sleep`, `unknown`. |
| `busy` | `true` while the plane runs a sequence on the slot; left out otherwise. |
| `v`, `a`, `w` | Slot volts (0.1 V), amperes (0.1 A) and whole watts. |
| `w_min`, `w_max` | Lowest and highest power since the plane last cleared them, whole watts. |
| `temp_c` | Slot temperature, whole degrees. |
| `energy_wh` | Energy since the plane's first boot, whole watt-hours. |
| `module_class` | The module class code the plane read from the slot. |
| `faults` | Names of the faults that are set: `absent`, `ov_latched`, `power_limit`, `retry_limit`, `unit_latched_off`, `cross_check`. Left out when there are none. |
| `last_job` | The last job the controller gave this node: `id` (the command id), `job`, `verb`, `started_ms`, and once it ended `finished_ms` and `result` (`done`, or `failed: <code>`, `absent: <code>`, `link_lost`, `timed_out`). Left out until the first command. |

`planes[]` lists every plane that answers: `plane` (0..7), `address`,
`type`, `firmware`, `bus_v` (the plane's input, 0.1 V), `temp_c`,
`boot_count`, `uptime_s`, `image_state`, `sequence_running` and
`intended_state_lost` (both left out when false), and `units[]` with
`unit`, `present`, `latched_off` (left out when false) and the unit's
input `v`, `a`, `w`, `temp_c`.

The rest:

- `probes[i].temp_c` is `null` when the probe did not answer.
- `dry_in[0]` is dry input 1; `true` = contact closed.
- `board_temp_c`, `vin_v` and `dry_in` are `null` until first read.
  `vin_v` is the controller's own 12 V input.
- The numbers are rounded before they are published so an on-change
  publish does not fire on sensor noise, but they travel as 32-bit
  floats: `1.3` may arrive as `1.2999999523`.

## Commands

```json
{"v":1,"id":"<client chosen>","action":"<name>","target":<target>,"args":{...}}
```

`id` is echoed in the ack topic and payload. `target` is a node id
`1`..`64`, `"all"`, `"planeN"` (0..7) or `"planeN.unitU"` (unit 0..3),
and defaults to `"all"`. `args` may be omitted. Unknown actions, and
targets outside those forms, are rejected.

| `action` | `target` | `args` | Notes |
|---|---|---|---|
| `on` | node, `all` | `hard` | Rail on if needed, PWR if the host stays dark |
| `off` | node, `all` | `no_escalate` | Soft off, escalating per the node's `escalation` unless `no_escalate` |
| `force_off` | node, `all` | `no_escalate` | Hold PWR 5 s, then cut the rail if the host is still lit |
| `reset` | node, `all` | | 300 ms RST press |
| `cycle` | node, `all` | `hard` | Rail off 5 s, on, PWR if the host stays dark; `hard` skips the LED wait |
| `press` | node, `all` | `switch` (`pwr` default, or `rst`), `duration_ms` (50..10000, default 300) | Raw press, ignores the LED |
| `on_all` | `all` (any target is ignored) | `hard` | Staggered start of the present nodes that are off, 2 s apart |
| `power_on` | node, `all` | | Rail only |
| `power_off` | node, `all` | | Rail only |
| `clear_latch` | node, `all` | | Clear the slot's overvoltage latch |
| `unit_reset` | `planeN.unitU` | | Let the plane's hold on the unit board's hot-swap go |
| `plane_reboot` | `planeN` | | Reboot the plane; slots keep their state |
| `rule_ack` | `all` | `rule` | Re-arm a manual rule |
| `probe_scan` | `all` | | Accepted; the re-scan is not yet wired to the bus |
| `config_get` | `all` | `section` (optional) | Reply carries the JSON in `data` |
| `config_set` | `all` | `section` (optional), `config` | Same staging rules as HTTP; `result` is `staged` or `saved` |
| `ota` | `all` | `url`, `sha256` (64 hex) | Pull update over HTTPS |
| `reboot` | `all` | | |
| `factory_reset` | `all` | `confirm` = the device id | |

Every node verb takes a node id or `all`. With `all` the controller
submits one plane job per present node, each with that node's own
settings, and acks the command once, when the last of those jobs has
reported; the ack carries the first job id of the group and an error
like `2 of 8 failed: failed` when some of them did not reach their
goal. A node verb sent to a `planeN` or `planeN.unitU` target is
refused with `<verb> needs a single node target`. `args.force` is
accepted from older clients and ignored. What the verbs do, and how
they fail, is in [Controlling nodes](../operation/nodes.md).

Examples:

```json
{"v":1,"id":"c-1","action":"on","target":3}
{"v":1,"id":"c-2","action":"off","target":3,"args":{"no_escalate":true}}
{"v":1,"id":"c-5","action":"cycle","target":10,"args":{"hard":true}}
{"v":1,"id":"c-6","action":"press","target":5,"args":{"switch":"rst","duration_ms":1500}}
{"v":1,"id":"c-7","action":"on_all","target":"all"}
{"v":1,"id":"c-8","action":"power_off","target":12}
{"v":1,"id":"c-9","action":"unit_reset","target":"plane1.unit0"}
{"v":1,"id":"c-10","action":"plane_reboot","target":"plane1"}
{"v":1,"id":"c-11","action":"rule_ack","args":{"rule":2}}
{"v":1,"id":"c-12","action":"config_get","args":{"section":"nodes"}}
{"v":1,"id":"c-13","action":"config_set","args":{"section":"sys","config":{"log_level":"info"}}}
{"v":1,"id":"c-14","action":"ota","args":{"url":"https://fw.example/granite-0.2.0.bin","sha256":"<64 hex>"}}
{"v":1,"id":"c-15","action":"factory_reset","args":{"confirm":"granite-aabbcc"}}
```

## Acks

A plane verb is acked when the plane reports the job finished; the
`accepted` event (with the `job` number) comes first. The completion
ack carries the **same `job`**, so a client can match the two without
keeping the command id. Everything else, and every refusal, is acked at
once.

```json
{"v":1,"id":"c-1","ok":true,"result":"done","error":null,"job":12}
{"v":1,"id":"c-2","ok":false,"result":null,"error":"failed: conflict","job":13}
{"v":1,"id":"c-3","ok":false,"result":null,"error":"node 3 is not present"}
{"v":1,"id":"c-12","ok":true,"result":"ok","error":null,"data":{"nodes":[],"plane_update":"manual","..."}}
```

`result` values: `done` (a plane job that did what was asked), `ok`,
`staged`, `saved`, `rebooting` (for `reboot` and `factory_reset`).
`error` values for a plane job: `failed: <code>`, `absent: <code>`,
`link_lost`, `timed_out`, where `<code>` is the plane's result code
(`busy`, `bad_argument`, `conflict`, `not_present`, `refused`,
`failed`).

How long a job is given is not a fixed number: it is the longest
sequence the node's own settings allow - an `off` on a node with
`t_soft_off_s` of 3600 gets its hour plus the escalation - with a
margin for the link's resends. Past that the job ends as `timed_out`,
and a little later the ack goes out as `the action did not finish in
time`. The plane carries on either way, and the outcome still reaches
the node's `last_job`.

## Events

Common fields `v`, `ts` (milliseconds since boot), `kind`.

| `kind` | Fields |
|---|---|
| `node_state` | `node`, `power`, `host` (published when either axis changes) |
| `accepted` | `id`, `action`, `target`, `job` |
| `action_done` | `id`, `node` (null for a unit, plane or group job), `action`, `result` (`done`) |
| `action_failed` | `id`, `node`, `action`, `error` |
| `plane` | `what`, and when they apply `address`, `node`, `unit`, `detail`; see below |
| `rule_fired` | `rule`, `name`, `action`, `target`, `value`, `needs_ack` |
| `ota` | `phase` (`started`, `downloading`, `verified`, `failed`, `pending_verify`, `valid`, `rolled_back`), `progress`, `detail` |
| `security` | `what` (`login`, `login_failed`, `login_lockout`, `password_set`, `password_changed`, `token_created`, `token_deleted`, `fleet_key_set`, `device_cert_set`, `mqtt_credentials_set`, `recover_accepted`, `recover_failed`), `detail`, `peer` |
| `config` | `section`, `change` (`set`, `fallback`, `imported`, `staged`, `confirmed`, `reverted`, `factory_reset`), `detail` |
| `fault` | `fault`, `node`, `id`; defined, but not emitted by the current firmware |

The `plane` event carries what the plane client learns and what the
planes report. `what` is one of:

| `what` | From | Meaning |
|---|---|---|
| `plane_found` | controller | A plane answered; `detail` has its firmware and boot count. |
| `plane_lost` | controller | A plane stopped answering. |
| `plane_rebooted` | controller | A plane's boot count changed; `detail` has the reset cause and says `intended state lost` after a power loss. |
| `address_collision` | controller | An address keeps answering with corrupt frames, probably two planes on the same straps. |
| `events_lost` | controller | The plane's event ring overflowed; `detail` says how many. |
| `config_pushed`, `config_refused` | controller | The plane config was written, or the plane did not take it. |
| `update_confirmed`, `update_failed` | controller | A plane firmware update ended. |
| `intended_off_reapplied` | controller | After a plane power loss, these nodes (`detail`, comma-separated ids) were commanded off again. |
| `slot_on`, `slot_off`, `pgd_lost`, `ov_latched`, `power_limit_trip`, `retry_limit_reached`, `led_changed`, `sequence_done`, `sequence_failed` | plane, per node | What the slot did. |
| `unit_present`, `unit_lost`, `unit_reset_seen`, `unit_hot_swap_latched_off` | plane, per unit | What a unit board did. `unit_hot_swap_latched_off` is the plane holding a faulted unit off, once per fault. |
| `plane_boot`, `config_stored`, `update_state_changed`, `image_confirmed`, `rolled_back` | plane | What the plane itself did. |

```json
{"v":1,"ts":602,"kind":"node_state","node":9,"power":"on","host":"on"}
{"v":1,"ts":20647,"kind":"accepted","id":"api-9","action":"unit_reset","target":"plane1.unit0","job":1}
{"v":1,"ts":20652,"kind":"action_done","id":"api-9","node":null,"action":"unit_reset","result":"done"}
{"v":1,"ts":20652,"kind":"plane","what":"slot_off","address":49,"node":9,"detail":"slot_off"}
{"v":1,"ts":20956,"kind":"plane","what":"plane_rebooted","address":49,"detail":"boot 2, reset cause 4"}
{"v":1,"ts":1500,"kind":"rule_fired","rule":10,"name":"node 4 hot","action":"force_off","target":4,"value":8620,"needs_ack":false}
```

## Session parameters

TLS 1.2 or later with the configured CA, username and password or a
client certificate, QoS from the configuration (default 1), keepalive
30 s, last will on `status`. Reconnects back off from 1 s to 60 s. The
broker is trusted for control: there is no second signature layer on
commands (a `sig` field is reserved for one).
