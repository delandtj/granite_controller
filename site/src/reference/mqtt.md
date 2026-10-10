# MQTT topics and commands

Topic base: `<topic_root>/<site>/<device>`, by default
`granite/default/granite-xxxxxx`. Every payload is JSON and carries
`"v":1`.

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
{"v":1,"ts":123456,
 "nodes":[{"node":1,"name":"node1","state":"on","led":true,
           "last_action":{"id":"c-1","action":"on","started_ms":100,"finished_ms":2300,"result":"ok"},
           "last_reset_ms":null}, "..."],
 "probes":[{"slot":1,"rom":"28ff000000000001","name":"inlet","temp_c":25.37}],
 "board_temp_c":31.0,"vin_v":19.12,
 "dry_in":[false,false,false,false],
 "link_up":true,"mqtt_connected":true,"uptime_s":4242}
```

- `nodes[i].state` is `unknown`, `off`, `on` or `busy`; `busy` entries
  add `busy_action`. `led` is `null` when the sense cannot be read.
- `probes[i].temp_c` is `null` when the probe did not answer.
- `dry_in[0]` is dry input 1; `true` = contact closed.
- `board_temp_c`, `vin_v` and `dry_in` are `null` until first read.

## Commands

```json
{"v":1,"id":"<client chosen>","action":"<name>","target":1..8|"all","args":{...}}
```

`id` is echoed in the ack topic and payload. `target` defaults to
`all`, which the actuator accepts only for `on_all` (anything else is
refused with `bad_target`); `args` may be omitted. Unknown actions and
targets outside 1..8/`all` are rejected.

| `action` | `target` | `args` | Notes |
|---|---|---|---|
| `on` | node | `force` | |
| `off` | node | `force`, `no_escalate` | Escalates to `force_off` after `t_soft_off` unless `no_escalate` |
| `force_off` | node | `force` | |
| `reset` | node | `force` | Refused on an off node |
| `cycle` | node | `force`, `hard` | `hard` uses `force_off` for the off half |
| `press` | node | `switch` (`pwr` or `rst`), `duration_ms` (100..10000) | Raw press, ignores the LED |
| `on_all` | `all` | | Staggered power-on in the configured order |
| `rule_ack` | `all` | `rule` | Re-arm a manual rule |
| `probe_scan` | `all` | | Accepted; the re-scan is not yet wired to the bus |
| `config_get` | `all` | `section` (optional) | Reply carries the JSON in `data` |
| `config_set` | `all` | `section` (optional), `config` | Same staging rules as HTTP; `result` is `staged` or `saved` |
| `ota` | `all` | `url`, `sha256` (64 hex) | Pull update over HTTPS |
| `reboot` | `all` | | |
| `factory_reset` | `all` | `confirm` = the device id | |

Examples:

```json
{"v":1,"id":"c-1","action":"on","target":3}
{"v":1,"id":"c-2","action":"off","target":3,"args":{"no_escalate":true}}
{"v":1,"id":"c-5","action":"cycle","target":2,"args":{"hard":true}}
{"v":1,"id":"c-6","action":"press","target":5,"args":{"switch":"rst","duration_ms":1500}}
{"v":1,"id":"c-7","action":"on_all","target":"all"}
{"v":1,"id":"c-8","action":"rule_ack","args":{"rule":2}}
{"v":1,"id":"c-10","action":"config_get","args":{"section":"net"}}
{"v":1,"id":"c-11","action":"config_set","args":{"section":"sys","config":{"log_level":"info"}}}
{"v":1,"id":"c-12","action":"ota","args":{"url":"https://fw.example/granite-0.2.0.bin","sha256":"<64 hex>"}}
{"v":1,"id":"c-14","action":"factory_reset","args":{"confirm":"granite-aabbcc"}}
```

## Acks

```json
{"v":1,"id":"c-1","ok":true,"result":"ok","error":null}
{"v":1,"id":"c-2","ok":false,"result":null,"error":"failed: power_on_timeout"}
{"v":1,"id":"c-10","ok":true,"result":"ok","error":null,"data":{"ip_mode":"dhcp","..."}}
```

Long actions first produce an `accepted` event, then the ack on
completion. `result` values: `ok`, `accepted`, `shutdown_pending`,
`staged`, `saved`, `refused: <reason>`, `failed: <reason>`.

## Events

Common fields `v`, `ts` (milliseconds since boot), `kind`.

| `kind` | Fields |
|---|---|
| `node_state` | `node`, `state`, `led` |
| `accepted` | `id`, `action`, `target` |
| `action_done` | `id`, `node`, `action`, `result` (`escalated` when `off` fell through to `force_off`) |
| `action_failed` | `id`, `node`, `action`, `error` |
| `soft_off_timeout` | `id`, `node` |
| `rule_fired` | `rule`, `name`, `action`, `target`, `value`, `needs_ack` |
| `fault` | `fault` (`press_deadline`, `expander`, `hal: ..`), `node`, `id` |
| `ota` | `phase` (`verified`, `failed`, `pending_verify`, `valid`, `rolled_back`), `progress`, `detail` |
| `security` | `what` (`login`, `login_failed`, `login_lockout`, `password_set`, `password_changed`, `token_created`, `token_deleted`, `fleet_key_set`, `device_cert_set`, `mqtt_credentials_set`, `recover_accepted`, `recover_failed`), `detail`, `peer` |
| `config` | `section`, `change` (`set`, `fallback`, `imported`, `staged`, `confirmed`, `reverted`, `factory_reset`), `detail` |

```json
{"v":1,"ts":1000,"kind":"node_state","node":3,"state":"on","led":true}
{"v":1,"ts":1500,"kind":"rule_fired","rule":1,"name":"probe over temperature","action":"force_off","target":"all","value":7123,"needs_ack":false}
{"v":1,"ts":1700,"kind":"ota","phase":"pending_verify","progress":null,"detail":"ota_1"}
```

## Session parameters

TLS 1.2 or later with the configured CA, username and password or a
client certificate, QoS from the configuration (default 1), keepalive
30 s, last will on `status`. Reconnects back off from 1 s to 60 s. The
broker is trusted for control: there is no second signature layer on
commands (a `sig` field is reserved for one).
