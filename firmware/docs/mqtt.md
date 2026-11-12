# The controller over MQTT: operator guide

How to watch and drive a Granite controller over MQTT: the topics, the
command envelope, every verb, the configuration sections, what the
controller publishes, recipes and troubleshooting. The block switch has
its own chapter, `docs/switch.md`; this one only points at it.

Design and reasons: `docs/adr/0001-firmware-architecture.md`
(components 6, 7, 10), `docs/adr/0004-controller-rev-d-node-model.md`
(nodes, planes, verbs) and `docs/adr/0006-block-switch.md`. Where this
guide and an ADR disagree, this guide follows the code and says so.

Topics below use the bench prefix `granite/bench/granite-cb374b`;
replace it with your controller's `<topic_root>/<site>/<device id>`.
Examples use `mosquitto_pub` / `mosquitto_sub` against a broker on
127.0.0.1. Set a shell variable once:

```sh
T=granite/bench/granite-cb374b
```

**Hardware status.** The bench controller has no plane attached, so
every node, plane and unit verb in this guide has run against the host
simulator (`granite-sim`) and the plane logic's host tests, not against
a plane. Payloads marked "captured" come from the simulator.

## 1. The rules that trip people up

1. **No `target` means `all`.** A node verb without a target runs on
   every present node. `{"action":"off"}` shuts the whole block down.
   Always write the target.
2. **The ack comes when the work is done, not when it is queued.** A
   node verb's ack on `.../ack/<id>` can take minutes (an `off` waits
   up to `t_soft_off_s` for the host). The "it was queued" signal is an
   `accepted` event on `.../event`.
3. **`config_set` replaces a whole section.** Keys you leave out go
   back to their defaults; lists you leave out are emptied (section 8).
   Always start from `config_get` and send the edited result back.
4. **`config_set` without `args.section` replaces the entire
   configuration.** Every section you do not include is reset to its
   defaults, MQTT included. See section 7.
5. **`net`, `mqtt`, `sec` and `switch` are staged.** A change to them
   is undone after `net.t_confirm_s` (default 300 s) unless you send
   `config_confirm`. `nodes`, `rules` and `sys` are saved at once and
   have no undo.
6. **A command must fit in 2048 bytes.** A bigger message (a CA
   certificate in `mqtt.ca_pem`, a whole configuration) is dropped by
   the controller with no ack at all. Use the HTTPS API for those.
7. **Never publish `cmd` retained.** And remember the controller keeps
   a persistent broker session (clean session off, QoS 1 subscription):
   commands sent while it is offline are delivered, and run, when it
   reconnects.
8. **Use a fresh, plain `id` per command** (letters, digits, `-`, `_`).
   It becomes a topic level (`.../ack/<id>`); a reused id can pick up
   the wrong ack. A command with no `id` is dropped silently.

## 2. Topics

All under `<topic_root>/<site>/<device id>` (defaults `granite`,
`default`, `granite-<last 6 hex of the MAC>`; `mqtt.topic_root`,
`mqtt.site`, `sys.device_id`).

| Topic | Dir | Retained | When | Payload |
|---|---|---|---|---|
| `.../status` | out | yes | on connect, then every 60 s | `{"v":1,"online":true,"fw","ip","uptime_s","boot_reason","ota"}` (section 9.1) |
| `.../status` (LWT) | out | yes | the broker loses the controller, or the controller drops the session on purpose | `{"v":1,"online":false}` |
| `.../state` | out | yes | on connect, on change (coalesced 200 ms), and every `mqtt.t_state_s` (60 s) | full snapshot (section 9.2) |
| `.../event` | out | no | as things happen | one event (section 9.3) |
| `.../log` | out | no | each log line at or above `sys.log_level` | `{"v":1,"ts","level","target","msg"}` |
| `.../cmd` | in | - | you publish | a command (section 4) |
| `.../ack/<id>` | out | no | when the command is finished (section 4.3) | the reply |
| `.../switch/counters` | out | no | every `switch.poll.counters_s` | see `docs/switch.md` section 9 |

Everything is published at `mqtt.qos` (default 1), and `.../cmd` is
subscribed at the same QoS.

- `status` and `state` are retained, so a new subscriber gets the
  current picture at once. `online:false` on `status` means the
  controller is gone (LWT) or left on purpose (reboot, broker change).
- `state` is not republished for noise: `ts` and `uptime_s` are
  ignored, and `vin_v` has to move 0.05 V before it counts.
- Events that happen while there is no broker session are held (the
  last 64) and sent on reconnect. Acks are not held: an ack that falls
  due while the session is down is lost (logged on the console).
- Reconnect: 1, 2, 4 ... 60 s, reset on a successful connect.

## 3. Watch while you work

```sh
mosquitto_sub -h 127.0.0.1 -v -t "$T/event" -t "$T/ack/#" -t "$T/status"
```

The current state, once:

```sh
mosquitto_sub -h 127.0.0.1 -C 1 -t "$T/state" | jq .
```

## 4. Sending commands

### 4.1 The envelope

```json
{"v":1,"id":"c-42","action":"on","target":3,"args":{}}
```

| Field | Required | Meaning |
|---|---|---|
| `v` | no (default 1) | Payload version. Anything but 1 is refused (`unsupported payload version 2`). |
| `id` | yes | Your id, echoed in the ack and in the topic `.../ack/<id>`. Empty or missing: the command is dropped with no ack. |
| `action` | yes | The verb (sections 5, 6). |
| `target` | no (default `all`) | What it acts on (below). Only node, unit and plane verbs read it. |
| `args` | no | Verb arguments. Unknown keys are ignored. |

`target` is one of:

| Form | Example | Means |
|---|---|---|
| `"all"` | `"all"` | Every present node. |
| number 1..64 (or the same as a string) | `3`, `"3"` | One node. Node id = 8 x plane + slot + 1. |
| `"planeN"`, N 0..7 | `"plane1"` | One plane. |
| `"planeN.unitU"`, U 0..3 | `"plane1.unit2"` | One unit board. A unit carries slots 2U and 2U+1. |

`"node3"` is not a target; neither is 0 or 65.

### 4.2 The reply (ack)

```json
{"v":1,"id":"c-42","ok":true,"result":"done","error":null,"job":7}
{"v":1,"id":"c2","ok":false,"result":null,"error":"node 6 is not present"}
{"v":1,"id":"c7","ok":true,"result":"ok","error":null,"data":{...}}
```

(captured)

- `ok` true: `result` says what happened, `error` is null.
- `ok` false: `error` is the reason in plain text, `result` is null.
  There are no error codes; match on `ok`, show `error`.
- `job`: the plane job the command became (node, unit and plane verbs
  only). The `accepted` event carries the same number.
- `data`: for `config_get` (the configuration) and `config_confirm` /
  `config_revert` (the list of sections).

A payload that is not a valid command but has a string `id` (64
characters at most) is answered on that id with
`"error":"bad command: <parser message>"`, e.g. a bad target:
`bad command: target must be "all", 1..=64, "planeN" or "planeN.unitU"`.

### 4.3 When the ack comes

| Command | Ack is published | `result` on success |
|---|---|---|
| Node, unit and plane verbs (section 5) | when the plane reports the result; refusals (absent node, bad target) at once | `done` |
| `on_all` with nothing left to start | at once | `done` |
| `rule_ack`, `config_get`, `config_set`, `config_confirm`, `config_revert` | at once | `ok`, `saved` / `staged`, `confirmed`, `reverted` |
| `reboot`, `factory_reset` | at once, before the restart | `rebooting` |
| `ota` | when the image is verified (then it reboots) or the pull failed; at most 600 s | `verified, rebooting into the new image` |
| switch verbs | at once; the outcome comes as `switch` events (`docs/switch.md`) | `accepted` |
| `probe_scan` | after about 75 s, as a failure (section 6.2) | - |

"Accepted" means the controller checked the target and handed the job
to the plane. For node, unit and plane verbs you never see `accepted`
on the ack topic: you see an `accepted` event, then later one ack.

Time limits:

- The controller task has 20 s to answer at all; otherwise the ack is
  `the controller task did not answer in time`.
- A plane job gets the longest its sequence can take with that node's
  settings, plus 75 s (60 s plane-client margin, 15 s grace). With the
  defaults that is about 110 s for `on`, 206 s for `off`, 86 s for
  `force_off`, 115 s for `cycle`, 80 s for `reset`, `power_on`,
  `power_off`, `clear_latch`, `unit_reset` and `plane_reboot`. A larger
  `t_soft_off_s` makes the `off` wait longer. On `all` the slowest
  configured node sets the limit. Normally the plane client gives up
  first and the ack says `timed_out`; if it does not, the ack is
  `the action did not finish in time`.
- A change of broker settings (`mqtt` section) rebuilds the client and
  loses the acks of commands still in flight.

Failure texts from a plane job: `failed: <code>`, `absent`,
`link_lost`, `timed_out`, where `<code>` is the plane's result
(`busy`, `bad_argument`, `conflict`, `not_present`, `refused`,
`failed`). A command on `all` acks once with the worst outcome, e.g.
`2 of 5 failed: failed`.

## 5. Node, unit and plane verbs

All need a target. The controller refuses at once, before anything is
written, when the target is not there: `node 6 is not present`,
`node 20 is on plane 2 which is not present`, `plane 3 is not present`,
`unit 2 of plane 1 is not present`, `no node is present` (for `all`).
A node whose plane answers but whose slot has not been read yet is let
through; the plane has the last word.

| Action | Target | Args | Does |
|---|---|---|---|
| `on` | node or `all` | - | Rail on if it is off, wait for the host's power LED, press PWR once if it stayed dark, wait again. Already lit: done at once. |
| `off` | node or `all` | `no_escalate` | Short PWR press, wait up to `t_soft_off_s` for the LED to go dark, then escalate per the node's `escalation`. Already dark: done at once. |
| `force_off` | node or `all` | `no_escalate` | Skip the short press: hold PWR 5 s, then (escalation `hold_then_cut`) cut the rail if the LED is still lit. Ends with the node off. |
| `reset` | node or `all` | - | 300 ms RST press. |
| `cycle` | node or `all` | `hard` | Rail off, 5 s, rail on, then like `on` (PWR if the host stays dark). |
| `press` | node or `all` | `switch` (`"pwr"` default / `"rst"`), `duration_ms` (default 300, clamped to 50..10000) | One raw button press. |
| `power_on` | node or `all` | - | Rail on. No buttons. |
| `power_off` | node or `all` | - | Rail off. No buttons: the host loses power without a shutdown. |
| `clear_latch` | node or `all` | - | Clear the slot's overvoltage latch (`ov_latched`). Does not power the node on. |
| `on_all` | ignored | - | Staggered `on` of every present node whose host is dark (section 5.3). |
| `unit_reset` | `planeN.unitU` | - | Release the plane's hold on a unit whose hot-swap latched off. Answer about 1 s later: `done` when the unit carries power again, `failed` when the fault is still there. Harmless on a unit that is not latched. |
| `plane_reboot` | `planeN` | - | Reboot the plane. The slots keep their state. Acked `done` as soon as the write is delivered (the plane resets before it can answer); watch for the `plane` event `plane_rebooted`. |

`args` for these verbs:

| Arg | Effect |
|---|---|
| `no_escalate: true` | `off` / `force_off` use escalation `none` this once. `off` then fails if the host stays up; `force_off` with `none` does nothing and fails (it skips the short press and has no escalation left). |
| `hard: true` | `cycle` (and `on`) ask the plane not to wait for the LED (LED timeout 0, see 5.1). |
| `force: true` | Accepted for rev C clients and ignored: an absent node is still refused. |

### 5.1 How the host LED is used

Every node has two states in `state` and in `node_state` events:
`power` (the rail: `off`, `on`, `starting`, `fault`, `absent`,
`unknown`) and `host` (the motherboard's power LED as the plane reads
it: `off`, `on`, `sleep` = blinking, `unknown` = rail off, LED not
trusted, or no data).

`on`, `off`, `force_off` and `cycle` decide from the LED. The per-node
setting `led_sense` (default true) says whether the LED is trusted:

- `led_sense: true`: the plane waits up to 15 s for the LED in `on`
  and `cycle`.
- `led_sense: false` (or `args.hard`): the controller sends an LED
  timeout of 0. **The plane clamps that to its blink window plus 1 s**,
  so it still looks at the LED briefly, still presses PWR once when
  it reads dark, and reports `failed` if it stays dark. ADR 0004 says
  "never wait for the light"; on a node with no LED wired, expect `on`
  to press PWR and fail. Use `power_on` / `power_off` for such nodes.
- `force_off` always holds PWR for 5 s, even on a host that is already
  dark.

### 5.2 Escalation

Per node, `nodes.nodes[].escalation`:

| Value | `off` after `t_soft_off_s` | `force_off` |
|---|---|---|
| `hold_then_cut` (default) | hold PWR 5 s, then cut the rail if still lit | hold 5 s, cut if still lit |
| `hold` | hold PWR 5 s; fails if still lit | hold 5 s; fails if still lit |
| `none` | fails if still lit | fails at once |

A host turned off by a button keeps its standby power; only a cut rail
removes it.

### 5.3 `on_all`

- Takes every present node, leaves out the ones whose host is `on` or
  `sleep`, staggers the rails that are down 2 s apart (one plane-level
  sequence per plane, planes side by side), then does `on` on each.
  Nodes whose rail is already up are not cut.
- Uses the shortest LED timeout of the nodes in the set.
- When every present node is already running, the ack is `done` at
  once and no job is made.
- One ack, with the worst outcome.

### 5.4 Boot policy

Not a verb, but it moves nodes: `nodes.nodes[].boot_policy` `on` /
`off` runs that verb once, 3 s after a **plane** boot (its boot count
moved, or it came back with its intended state lost). A controller
reboot moves nothing. The commands use the id `boot-<node>`.

## 6. Other verbs

None of these reads `target`.

### 6.1 `rule_ack`

```sh
mosquitto_pub -h 127.0.0.1 -t "$T/cmd" -m '{"v":1,"id":"ra2","action":"rule_ack","args":{"rule":2}}'
```

Re-arms a `manual` rule (one whose `rule_fired` event said
`"needs_ack":true`) so it can fire again. Acks `ok` for any rule id
that exists, fired or not; `no rule 9` otherwise. It does not undo
what the rule did.

### 6.2 `probe_scan`

Not wired on the firmware: the controller logs
`probe_scan: ... not wired yet` and does nothing. Over MQTT the ack
then times out after about 75 s with
`the action did not finish in time` (over HTTPS the reply is
`accepted`). The probe list is read from `nodes.probes`, or found by a
bus scan at boot when that list is empty.

### 6.3 `ota`

```sh
mosquitto_pub -h 127.0.0.1 -t "$T/cmd" -m '{"v":1,"id":"ota1","action":"ota","args":{
 "url":"https://fw.example.net/granite-fw-0.4.0.bin",
 "sha256":"<64 hex digits>"}}'
```

The controller downloads the image itself (HTTPS, redirects followed),
checks the SHA-256 and the image's link-kind marker, writes it to the
other slot and reboots into it. The server certificate must chain to
`mqtt.ca_pem`, the only CA the pull uses; with an empty `ca_pem` the
pull effectively cannot verify any server.

- Refused at once unless `url` is non-empty and `sha256` is 64
  characters (the hex is not checked further before the download).
- Progress events (`kind:"ota"`): `started`, `downloading` (every 10 %
  or 256 KiB), `verified` or `failed`; after the reboot
  `pending_verify`, then `valid`, or `rolled_back` / `failed`.
- The ack: `verified, rebooting into the new image`, or the failure
  text.
- Probation: the new image must, within `sys.t_validate_s` (600 s),
  bring up its plane link, get an IP and connect to the broker (or
  serve an authenticated HTTPS request), or it rolls back. `status.ota`
  reads `pending` meanwhile.

### 6.4 `reboot`

`{"v":1,"id":"rb1","action":"reboot"}`. Acks `rebooting`, flushes
MQTT, restarts. Nodes are not touched; planes keep running. Anything
staged and not confirmed is discarded by the reboot.

### 6.5 `factory_reset`

```sh
mosquitto_pub -h 127.0.0.1 -t "$T/cmd" -m '{"v":1,"id":"fr1","action":"factory_reset","args":{"confirm":"granite-cb374b"}}'
```

`confirm` must be the device id, else `args.confirm must be the device
id`. Erases the configuration, staging and secrets (admin password,
API tokens, broker password, switch password), keeps the recovery
token, fleet key and device certificate, and reboots. **MQTT is off in
the default configuration, so the controller does not come back on the
broker**: it comes back on DHCP with the setup page in first-setup mode.

### 6.6 Configuration verbs

| Action | Args | Does |
|---|---|---|
| `config_get` | `section` (optional) | `data` = that section as it runs now (a staged change included), or without `section` the whole exportable configuration (no secrets). |
| `config_set` | `section` + `config`, or `config` alone | Section 7. Ack `saved` or `staged`, or `ok:false` with every validation problem joined by `; `. Nothing changes on a refusal. |
| `config_confirm` | - | Promote every staged section. Ack `confirmed`, `data` = the sections, e.g. `["net"]`. `nothing is staged` when nothing is. |
| `config_revert` | - | Put every staged section back now. Ack `reverted`, `data` = the sections. With `net` among them the controller reboots into its stored network configuration. |

Section names: `net`, `mqtt`, `switch`, `nodes`, `rules`, `sec`,
`sys`. A wrong name is a `bad command: unknown variant ...`.

### 6.7 Switch verbs

`switch_port`, `switch_save`, `switch_reconcile`, `switch_reboot`,
`switch_factory_reset`: see `docs/switch.md` section 8.

## 7. Changing the configuration

### 7.1 What `config_set` does

```json
{"v":1,"id":"n1","action":"config_set","args":{"section":"nodes","config":{ ...the whole section... }}}
```

- **One section is replaced whole.** The `config` object is parsed as
  the complete section: a key you leave out takes its default, a list
  you leave out becomes its default (usually empty). There is no patch
  or merge. Unknown keys are ignored, so a typo silently becomes a
  default.
- Some values are fixed up silently instead of refused (section 8
  notes which), then the whole configuration is validated. Any problem
  refuses the command and changes nothing.
- **Without `args.section`, `config` is a whole configuration
  document** (what `config_get` without a section returns). It
  **replaces everything**: every section you leave out goes back to
  its defaults, including `mqtt` (off), `net` (DHCP), `rules` (the three
  examples, disabled) and `sec.modbus` (off). Each section that changed
  is then staged or saved by the rules below, and the ack says
  `staged` if any of them was staged. In practice a whole document
  rarely fits in the 2048-byte command limit; use
  `POST /api/v1/config/import` over HTTPS instead.

Lists whose absence deletes:

| List | Left out means |
|---|---|
| `nodes.nodes` | every per-node setting (names, boot policies, escalation, ...) is gone; every node runs on defaults |
| `nodes.probes` | no configured probes: names are lost and the controller falls back to the probes a boot-time bus scan found |
| `rules.rules` | **the three shipped example rules come back** (disabled). `"rules":[]` is what deletes them all |
| `net.dns` | no static name servers |
| `sec.modbus.allow` | empty allow-list; with `sec.modbus.enabled` true that is refused |
| `switch.desired.vlans` | see `docs/switch.md` rule 2 |

### 7.2 Saved or staged

| Section | On `config_set` | Undo |
|---|---|---|
| `nodes`, `rules`, `sys` | saved to flash at once, ack `saved`, event `config` `set` | none: send the old section back |
| `net`, `mqtt`, `sec`, `switch` | applied live and staged, ack `staged`, event `config` `staged` | automatic unless confirmed |

The staged cycle:

```
config_set -----> ack "staged"            event config staged (section)
                  ... running, NOT stored ...
config_confirm -> ack "confirmed" data [..] event config confirmed (section null)
  or
config_revert --> ack "reverted" data [..]  event config reverted, detail "reverted on request"
  or net.t_confirm_s passes                 event config reverted, detail "not confirmed in time"
```

- One window for everything staged: staging another section restarts
  the clock for all of them, and one confirm or revert closes them all.
- The window length is `net.t_confirm_s` as it is after the change, so
  a `net` change can carry its own longer window.
- How each section goes back: `net` reboots the controller into the
  stored network configuration (nodes are not touched); `mqtt` and
  `sec` get their stored value back in the running configuration (the
  MQTT client is rebuilt, Modbus rebinds); `switch` is undone on the
  switch (`docs/switch.md` section 5).
- A reboot before the confirm discards the staged value too.
- Over HTTPS the same cycle is `PUT /api/v1/config/<section>` then
  `POST /api/v1/config/confirm` or `/revert` (409 when nothing is
  staged).

### 7.3 `mqtt.auto_confirm_on_connect`

When the **staged** `mqtt` section has `auto_confirm_on_connect: true`,
the first successful connection made with it confirms the `mqtt`
section (event `config` `confirmed`, section `mqtt`, detail
`auto_confirm_on_connect`). Only `mqtt` is promoted: a `net` or `sec`
change staged at the same time still needs its own `config_confirm`.
The flag must be in the section you stage; the one stored before does
not count.

## 8. Section reference

Defaults in brackets. "Clamped" means fixed up silently, not refused.

### 8.1 `net` (staged)

| Key | Default | Meaning / limits |
|---|---|---|
| `ip_mode` | `"dhcp"` | `"dhcp"` (AutoIP added after 30 s without a lease) or `"static"`. |
| `address` | `""` | CIDR, e.g. `"192.168.64.50/24"`. Required with `static`. Only emptiness is checked by validation; a malformed address is refused when applied, so nothing comes up on it (do not confirm; the window reboots back). |
| `gateway` | `""` | Default gateway. Empty also disarms the dead-man timer. |
| `dns` | `[]` | Name servers for static. |
| `hostname` | `""` | Empty = `granite-<mac6>`. Also the mDNS name. |
| `mdns` | `true` | Advertise over mDNS. |
| `vlan` | `null` | **Not implemented**: a value is logged and ignored, the port runs untagged. |
| `sntp` | `""` | Empty = from DHCP option 42. |
| `t_confirm_s` | `300` | The commit-confirm window for every staged section. |
| `t_deadman_s` | `3600` | Static address with no gateway reply and no accepted connection this long: DHCP is started alongside. `0` = off. |

### 8.2 `mqtt` (staged)

| Key | Default | Meaning / limits |
|---|---|---|
| `enabled` | `false` | Talk to a broker at all. `true` needs `host`. |
| `host` | `""` | Broker host or IP. |
| `port` | `8883` | |
| `tls` | `true` | `mqtts://`. Off is allowed and logged as a warning: the password and every command cross the network in the clear. |
| `ca_pem` | `""` | CA chain for the broker (and for `ota` pulls). With `tls` and an empty CA the handshake fails. A PEM rarely fits in a 2048-byte command: set it over HTTPS. |
| `username` | `""` | Broker user. |
| `client_id` | `""` | Empty = the device id. Must be unique on the broker. |
| `site` | `"default"` | Second topic level. Empty = `default`. |
| `topic_root` | `"granite"` | First topic level. Empty = `granite`. |
| `qos` | `1` | 0, 1 or 2 for everything published and the `cmd` subscription. Above 2: clamped to 1. |
| `keepalive_s` | `30` | At least 5. |
| `t_state_s` | `60` | Republish `state` at least this often. |
| `auto_confirm_on_connect` | `false` | Section 7.3. |

The broker password and the client certificate / key are secrets, not
in this section, and cannot be set over MQTT: HTTPS
`POST /api/v1/security/mqtt-credentials` with `username`, `password`,
`client_cert_pem`, `client_key_pem`. The client reads them at each
connection attempt.

`enabled`, `host`, `port`, `tls`, `ca_pem`, `username`, `client_id`,
`site`, `topic_root` and `keepalive_s` take effect by rebuilding the
client (within about 5 s, no reboot); `qos`, `t_state_s` and
`sys.log_level` are picked up live.

### 8.3 `nodes` (saved at once)

| Key | Default | Meaning / limits |
|---|---|---|
| `nodes` | `[]` | Per-node settings, below. Kept for nodes that are not present. |
| `plane_update` | `"manual"` | `"manual"` or `"auto"`: plane firmware updates by hand, or whenever a plane's version differs. |
| `probes` | `[]` | `{"rom":"28ff000000000001","name":"inlet"}`, in slot order. At most 8; each `rom` 16 hex digits. |
| `t_probe_s` | `10` | Probe read period. `0` becomes 10. |
| `vin_trim` | `1.0` | Correction for the VIN divider. Not positive becomes 1.0. |
| `vin_min_mv` | `10000` | Below it the `vin` fault bit is set. `0` = unchecked. Must be below `vin_max_mv` when both are set. |
| `vin_max_mv` | `14000` | Above it the `vin` fault bit is set. `0` = unchecked. Clamped to 36000. |

One entry of `nodes` (only `node` is required; the rest default):

| Key | Default | Meaning / limits |
|---|---|---|
| `node` | - | 1..64. Entries for other ids are dropped; duplicates keep the first. |
| `name` | `""` | Display name. Empty = `node<N>`. |
| `boot_policy` | `"leave"` | `"leave"`, `"on"`, `"off"` (section 5.4). |
| `led_sense` | `true` | Trust the host LED (section 5.1). |
| `t_soft_off_s` | `120` | How long `off` waits for the host. `0` becomes 120; above 3600 is clamped to 3600 (not refused). |
| `escalation` | `"hold_then_cut"` | `"none"`, `"hold"`, `"hold_then_cut"` (section 5.2). |

Settings are read when a command is sent, so a change applies to the
next command, no reboot. Probe and VIN changes apply on the next sense
tick.

### 8.4 `rules` (saved at once)

`{"rules":[ ...up to 16 rules... ]}`. Evaluated once a second, broker
or not. More than 16: the extra are dropped silently. Leaving out the
`rules` key brings back the three shipped examples (all disabled):

```json
{"rules":[
 {"id":1,"enabled":false,"name":"probe over temperature","source":"probe_max","op":">","threshold":7000,
  "hysteresis":500,"hold_s":30,"action":"act","kind":"force_off","target":"all","rearm":"auto","missing":"trip"},
 {"id":2,"enabled":false,"name":"leak float closed","source":"dry_in","n":1,"op":"==","threshold":1,
  "hysteresis":0,"hold_s":2,"action":"act","kind":"force_off","target":"all","rearm":"manual","missing":"ignore"},
 {"id":3,"enabled":false,"name":"bus voltage low","source":"vin","op":"<","threshold":10000,
  "hysteresis":500,"hold_s":5,"action":"event","target":"all","rearm":"auto","missing":"ignore"}]}
```

(captured)

| Key | Default | Meaning |
|---|---|---|
| `id` | - | 1..255, unique. 0 is refused. |
| `enabled` | `false` | |
| `name` | `""` | Label, copied into `rule_fired`. |
| `source` + its index | `probe_max` | What to read (table below). |
| `op` | `">"` | `">"`, `"<"`, `"=="`, `"changed"` (true for one tick when the value differs from the last one; `hold_s` and `hysteresis` do not apply). |
| `threshold` | `0` | In the source's unit. |
| `hysteresis` | `0` | How far back past the threshold the value must come before an `auto` rule re-arms. |
| `hold_s` | `0` | The condition must hold this long. |
| `action` | `"event"` | `"event"` (publish `rule_fired` only) or `"act"` with `"kind":"<verb>"`. Use the section 5 verbs; others are not carried out from a rule. |
| `target` | `"all"` | Target of the verb (section 4.1 forms). |
| `rearm` | `"auto"` | `"auto"`, or `"manual"`: stays fired until `rule_ack`. |
| `missing` | `"ignore"` | No value (probe gone, not read yet): `"ignore"` = condition false; `"trip"` = condition true (not for `changed`). |

A rule's `force_off` carries `force`, which (section 5) changes
nothing today: an absent node is still refused. A rule's command id is
`rule-<id>`; when it cannot act, an `action_failed` event with that id
says why.

Sources (thresholds in the unit shown):

| `source` | Index keys | Unit |
|---|---|---|
| `probe` | `n` 1..8 | centi-degrees C |
| `probe_max` | - | centi-degrees C, hottest probe that answered |
| `board_temp` | - | centi-degrees C |
| `vin` | - | mV |
| `dry_in` | `n` 1..4 | 0/1, 1 = closed |
| `node_on` | `n` 1..64 | 0/1, rail on |
| `node_host_on` | `n` | 0/1, LED steady |
| `node_power` | `n` | 0 unknown, 1 off, 2 on, 3 starting, 4 fault, 5 absent |
| `node_host` | `n` | 0 unknown, 1 off, 2 on, 3 sleep |
| `node_volts` | `n` | mV |
| `node_amps` | `n` | mA |
| `node_watts` | `n` | 0.1 W |
| `node_temp` | `n` | centi-degrees C |
| `plane_bus_volts` | `p` 0..7 | mV |
| `unit_amps`, `unit_watts` | `p` 0..7, `u` 0..3 | mA, 0.1 W |
| `mqtt_connected`, `link_up` | - | 0/1 |
| `uptime_s` | - | s |

Index ranges are not validated: an out-of-range index just never has a
value (so `missing` decides).

### 8.5 `sec` (staged)

| Key | Default | Meaning |
|---|---|---|
| `admin_password_set` | `false` | Informational, written by the password route. First-setup mode follows the stored password hash, not this flag. |
| `session_hours` | `12` | HTTPS session cookie lifetime. |
| `max_login_fails` | `5` | Failed logins before the lockout. |
| `lockout_s` | `60` | Lockout length. |
| `modbus.enabled` | `false` | Modbus TCP server. Needs a non-empty `allow`. |
| `modbus.port` | `502` | |
| `modbus.unit_id` | `1` | |
| `modbus.max_conn` | `4` | |
| `modbus.allow` | `[]` | Peers as addresses or CIDRs. |
| `id_on_plain_http` | `true` | Serve `/id` on plain HTTP too (the recovery path). |
| `fleet_recovery_pubkey_pem` | `""` | Copy written by the HTTPS route. The key the controller verifies with is kept in the identity store; set it over HTTPS. |
| `device_cert_pem` | `""` | Same: the HTTPS certificate in use lives in the identity store. |

### 8.6 `sys` (saved at once)

| Key | Default | Meaning |
|---|---|---|
| `device_id` | `""` | Empty = `granite-<mac6>`. Read at boot only: takes effect after a `reboot`, and then changes the topic prefix, the default client id and the `factory_reset` confirm string. |
| `log_level` | `"warn"` | `error`, `warn`, `info`, `debug`, `trace`: what goes to `.../log`. Live. |
| `t_validate_s` | `600` | OTA probation window. `0` = 600. |
| `coredump_report` | `true` | Not used by the firmware yet. |
| `timezone` | `"UTC"` | Not used by the firmware yet; wire timestamps are monotonic ms. |

### 8.7 `switch` (staged)

Every key, `desired` and the confirm cycle on the switch:
`docs/switch.md` sections 4 and 5.

## 9. What the controller publishes

### 9.1 `status`

```json
{"v":1,"online":true,"fw":"0.1.0","ip":"192.168.64.50","uptime_s":5321,"boot_reason":"software","ota":"valid"}
```

- `boot_reason`: `power_on`, `software`, `task_watchdog`,
  `int_watchdog`, `brown_out`, `panic`, `unknown`.
- `ota`: the running image: `valid`, `pending` (on probation),
  `invalid`, `factory`, `unknown`.

### 9.2 `state`

Captured from the simulator (two planes), trimmed to two nodes, one
plane and one unit. Node 9's `power`, `host` and `last_job` are from
a later capture, after a `power_off` on `all`; its telemetry is from
the first. The simulator's HTTP route prints `a` as
`1.7999999523162842`, MQTT prints `1.8`:

```json
{"v":1,"ts":3248,"uptime_s":3,
 "nodes":[{"node":1,"name":"node1","plane":0,"slot":0,"unit":0,"power":"on","host":"on",
           "v":48.0,"a":1.8,"w":86,"w_min":86,"w_max":86,"temp_c":25,"energy_wh":0,"module_class":5},
          {"node":9,"name":"node9","plane":1,"slot":0,"unit":0,"power":"off","host":"unknown",
           "v":48.0,"a":1.3,"w":62,"w_min":62,"w_max":62,"temp_c":25,"energy_wh":0,"module_class":5,
           "last_job":{"id":"api-3","job":5,"verb":"power_off","started_ms":21217,"finished_ms":21317,"result":"done"}}],
 "planes":[{"plane":0,"address":48,"type":"pp48_chained","firmware":"0.1.0","bus_v":48.0,"temp_c":25,
            "boot_count":1,"uptime_s":2,"image_state":"confirmed","intended_state_lost":true,
            "units":[{"unit":0,"present":true,"v":48.0,"a":3.0,"w":144,"temp_c":25}]}],
 "probes":[{"slot":1,"rom":"28ff000000000001","name":"","temp_c":42.0}],
 "board_temp_c":34.0,"vin_v":12.0,"dry_in":[false,false,false,false],
 "link_up":true,"mqtt_connected":true}
```

| Field | Meaning |
|---|---|
| `ts` | Controller monotonic ms of the snapshot. |
| `uptime_s` | Controller uptime. |
| `nodes[]` | Every **present** node, by id. A node that is not there is not listed (its settings are kept). |
| `nodes[].node`, `name` | Id 1..64 and display name. |
| `nodes[].plane`, `slot`, `unit` | Where it is: plane 0..7, slot 0..7, unit 0..3 (= slot / 2). |
| `nodes[].power`, `host` | The two axes of section 5.1. |
| `nodes[].busy` | Present and `true` only while the plane runs a sequence on the slot. |
| `nodes[].v`, `a`, `w` | Slot volts (0.1 V), amps (0.1 A), watts (whole). |
| `nodes[].w_min`, `w_max` | Lowest / highest watts since the last min/max clear. |
| `nodes[].temp_c` | Slot temperature, whole degrees. |
| `nodes[].energy_wh` | Energy since the plane's first boot. |
| `nodes[].module_class` | The plane's module class code. |
| `nodes[].faults` | Present only when something is set: `absent`, `ov_latched`, `power_limit`, `retry_limit`, `unit_latched_off`, `cross_check`. |
| `nodes[].last_job` | The last command that touched the node: `id`, `job`, `verb`, `started_ms`, and once done `finished_ms` and `result`. |
| `planes[]` | Every plane that answers: `plane`, I2C `address` (48 = 0x30), `type`, `firmware`, `bus_v`, `temp_c`, `boot_count`, `uptime_s`, `image_state`. `sequence_running` and `intended_state_lost` appear only when true. |
| `planes[].units[]` | `unit`, `present`, input `v` / `a` / `w` / `temp_c`; `latched_off` appears only when the unit's hot-swap latched off. |
| `probes[]` | `slot` (1-based), `rom`, `name`, `temp_c` (`null` when the probe did not answer). |
| `board_temp_c` | Controller board temperature, `null` if not read. |
| `vin_v` | Controller input voltage, `null` if not read. |
| `dry_in` | Four dry contacts, index 0 = input 1; `null` when never read (the rev D firmware does not read them yet: the pins are not assigned). |
| `switch` | Present only when a block switch is configured: `docs/switch.md` section 9. |
| `link_up` | Ethernet link. |
| `mqtt_connected` | Broker session (true in anything you receive over MQTT). |

### 9.3 Events

Every event is `{"v":1,"ts":<ms>,"kind":"<kind>", ...fields}`. Fields
shown as `null` are present with a null value; fields marked
"optional" are left out when empty.

| `kind` | Fields | When |
|---|---|---|
| `node_state` | `node`, `power`, `host` | A node's power or host state changed (and once per node when the controller first sees it). |
| `accepted` | `id`, `action`, `target`, `job` | A node, unit or plane verb became a plane job. |
| `action_done` | `id`, `node` (null for a group, unit or plane), `action`, `result` | That job (or the last of a group) finished as asked. |
| `action_failed` | `id`, `node`, `action`, `error` | It did not; also a rule that could not act. |
| `plane` | `what`; optional `address`, `node`, `unit`, `detail` | The plane client learned something (below). |
| `switch` | `what`; optional `port`, `node`, `detail` | `docs/switch.md` section 9. |
| `rule_fired` | `rule`, `name`, `action` (`"event"` or the verb), `target`, `value` (null when it tripped on a missing value), `needs_ack` | A rule fired. |
| `ota` | `phase`, `progress` (percent or null), `detail` | Section 6.3. |
| `config` | `section` (null for "every staged section"), `change`, `detail` | `set`, `staged`, `confirmed`, `reverted` (detail `reverted on request` / `not confirmed in time`), `fallback` (a stored section was unreadable at boot and fell back to defaults; detail says why), `imported` (HTTPS import). |
| `security` | `what`, `detail`, `peer` | HTTPS logins, failed logins, lockouts, password / token / key changes. Not caused by MQTT. |
| `fault` | `fault`, `node`, `id` | Defined in the vocabulary; the current firmware does not emit it. |

`plane` event `what` values: from the controller `plane_found` (detail
`firmware X, boot N`), `plane_lost`, `plane_rebooted` (detail
`boot N, reset cause C[, intended state lost]`), `address_collision`,
`events_lost`, `config_pushed`, `config_refused`, `update_confirmed`,
`update_failed`, `intended_off_reapplied` (detail: the node ids); from
the plane `slot_on`, `slot_off`, `pgd_lost`, `ov_latched`,
`power_limit_trip`, `retry_limit_reached`, `led_changed`,
`sequence_done`, `sequence_failed`, `unit_present`, `unit_lost`,
`unit_reset_seen`, `unit_hot_swap_latched_off`, `plane_boot`,
`config_stored`, `update_state_changed`, `image_confirmed`,
`rolled_back`.

Examples (from the payload tests in `granite-core/tests/msg_json.rs`):

```json
{"v":1,"ts":1000,"kind":"node_state","node":3,"power":"on","host":"sleep"}
{"v":1,"ts":1100,"kind":"accepted","id":"c-1","action":"on_all","target":"all","job":3}
{"v":1,"ts":1200,"kind":"action_done","id":"c-1","node":3,"action":"on","result":"done"}
{"v":1,"ts":1300,"kind":"action_failed","id":"c-2","node":4,"action":"force_off","error":"failed: failed"}
{"v":1,"ts":1400,"kind":"plane","what":"plane_lost","address":49}
{"v":1,"ts":1,"kind":"plane","what":"ov_latched","address":48,"node":3,"detail":"ov_latched"}
{"v":1,"ts":2000,"kind":"rule_fired","rule":2,"name":"leak float closed","action":"force_off","target":"all","value":1,"needs_ack":true}
{"v":1,"ts":1700,"kind":"ota","phase":"downloading","progress":42,"detail":null}
{"v":1,"ts":1800,"kind":"config","section":"nodes","change":"fallback","detail":"parse error"}
```

## 10. Recipes

A helper for the examples:

```sh
cmd() { mosquitto_pub -h 127.0.0.1 -t "$T/cmd" -m "$1"; }
```

**Power a node on, off, reset, cycle** (node 3):

```sh
cmd '{"v":1,"id":"n3-on","action":"on","target":3}'
cmd '{"v":1,"id":"n3-off","action":"off","target":3}'          # graceful, escalates per setting
cmd '{"v":1,"id":"n3-kill","action":"force_off","target":3}'   # hold 5 s, then cut
cmd '{"v":1,"id":"n3-rst","action":"reset","target":3}'
cmd '{"v":1,"id":"n3-cyc","action":"cycle","target":3}'
```

Wait for `.../ack/n3-on`; `"result":"done"` is success.

**Stagger-start a block**: `cmd '{"v":1,"id":"start","action":"on_all"}'`
(section 5.3). Running hosts are left alone. To bring up only some
nodes with your own spacing, send `on` per node from a script.

**Read a node's host state**:

```sh
mosquitto_sub -h 127.0.0.1 -C 1 -t "$T/state" \
  | jq '.nodes[] | select(.node==3) | {power, host, faults, last_job}'
```

Not listed at all = not present. For changes as they happen, filter
`.../event` on `kind == "node_state"`.

**Clear a latched fault**:

- `faults` has `ov_latched` (power `fault`):
  `cmd '{"v":1,"id":"cl3","action":"clear_latch","target":3}'`, then
  `on`. Find out why it latched first.
- `unit_latched_off` (a `plane` event `unit_hot_swap_latched_off`): the
  unit's hot-swap, shared by two slots. `on` does not clear it:
  `cmd '{"v":1,"id":"ur","action":"unit_reset","target":"plane0.unit1"}'`.
  `failed` means the fault is still there.
- `power_limit`, `retry_limit`, `cross_check`: no controller verb
  clears these; they are the plane's to clear.

**Reboot a plane**: `cmd '{"v":1,"id":"pr1","action":"plane_reboot","target":"plane1"}'`.
Acked `done` on delivery; the plane comes back with a `plane_rebooted`
event and its slots as they were.

**Change the MQTT broker safely**:

1. If the new broker needs other credentials, set them over HTTPS
   (`/api/v1/security/mqtt-credentials`; there is one set, used by
   the next connection attempt). A new CA also goes over HTTPS
   (`PUT /api/v1/config/mqtt`) because it does not fit a command.
2. Get the section: `cmd '{"v":1,"id":"mg","action":"config_get","args":{"section":"mqtt"}}'`.
3. Send it back whole with the new `host` / `port` and
   `"auto_confirm_on_connect":true`:
   ```sh
   cmd '{"v":1,"id":"mq1","action":"config_set","args":{"section":"mqtt","config":{
    "enabled":true,"host":"10.0.0.5","port":8883,"tls":true,"ca_pem":"<as before>",
    "username":"granite","client_id":"","site":"bench","topic_root":"granite",
    "qos":1,"keepalive_s":30,"t_state_s":60,"auto_confirm_on_connect":true}}}'
   ```
4. Within about 5 s the controller says `online:false` on the old
   broker and connects to the new one. Watch the new broker: a
   `config` event `confirmed` with detail `auto_confirm_on_connect`
   means done. Without auto-confirm, send `config_confirm` on the new
   broker within `net.t_confirm_s`.
5. If it never connects, the old section comes back after the window
   and the controller returns to the old broker (`config` `reverted`,
   `not confirmed in time`, delivered there from the held events).

Changing `site` or `topic_root` moves every topic: subscribe to the
new prefix before you send it.

**Change the controller's IP safely**:

```sh
cmd '{"v":1,"id":"ip1","action":"config_set","args":{"section":"net","config":{
 "ip_mode":"static","address":"192.168.64.50/24","gateway":"192.168.64.254",
 "dns":["192.168.64.254"],"hostname":"","mdns":true,"vlan":null,"sntp":"",
 "t_confirm_s":600,"t_deadman_s":3600}}}'
```

The new address is applied at once (the broker session drops and
comes back). When `status` shows the new `ip`, confirm:
`cmd '{"v":1,"id":"ip1-ok","action":"config_confirm"}'`. If the
controller does not come back, do nothing: after `t_confirm_s` it
reboots into the old configuration. Nodes keep running either way.

**Add a rule**: get `rules`, append, send the whole list back (a rule
left out is deleted). Saved at once, no confirm. Node 5 slot above 70 C
for 30 s: shut it off and wait for an ack:

```sh
cmd '{"v":1,"id":"r1","action":"config_set","args":{"section":"rules","config":{"rules":[
 {"id":1,"enabled":false,"name":"probe over temperature","source":"probe_max","op":">","threshold":7000,"hysteresis":500,"hold_s":30,"action":"act","kind":"force_off","target":"all","rearm":"auto","missing":"trip"},
 {"id":2,"enabled":false,"name":"leak float closed","source":"dry_in","n":1,"op":"==","threshold":1,"hysteresis":0,"hold_s":2,"action":"act","kind":"force_off","target":"all","rearm":"manual","missing":"ignore"},
 {"id":3,"enabled":false,"name":"bus voltage low","source":"vin","op":"<","threshold":10000,"hysteresis":500,"hold_s":5,"action":"event","target":"all","rearm":"auto","missing":"ignore"},
 {"id":4,"enabled":true,"name":"node5 hot","source":"node_temp","n":5,"op":">","threshold":7000,"hysteresis":500,"hold_s":30,"action":"act","kind":"force_off","target":5,"rearm":"manual","missing":"ignore"}]}}}'
```

After it fires, `rule_fired` says `needs_ack:true`; send
`{"v":1,"id":"ra4","action":"rule_ack","args":{"rule":4}}` once the
node is sorted.

**OTA update**:

```sh
sha256sum granite-fw-0.4.0.bin
cmd '{"v":1,"id":"ota-040","action":"ota","args":{"url":"https://fw.example.net/granite-fw-0.4.0.bin","sha256":"<the digest>"}}'
```

Watch `kind:"ota"` events and `.../ack/ota-040`; after the reboot
`status.fw` shows the new version and `status.ota` goes from `pending`
to `valid`. How to build the `.bin`: `README.md`, "Making the .bin".

## 11. Troubleshooting

| You see | Means / do |
|---|---|
| No ack at all | Command over 2048 bytes, no or empty `id`, not JSON, or published to the wrong prefix. Check the `.../log` topic and the console. |
| Ack `bad command: ...` | The payload did not parse: wrong verb name, target form, section name, or a wrong type. |
| Ack `unsupported payload version` | `v` is not 1. |
| `node N is not present` / `plane N is not present` | Not in `state`. Check the plane cable and `plane_lost` events. |
| `this action needs a plane target` / `a unit target` / `needs a single node target` | Wrong target kind for the verb (section 5). |
| A whole block went off | A node verb was sent without `target` (rule 1). |
| `on` acks `failed: failed` on a node with no LED | Section 5.1: use `power_on`. |
| `off` takes minutes | It waits `t_soft_off_s` for the host. `force_off` for now; lower the setting for later. |
| `force_off` acks `failed` | Escalation `none` or `hold` on that node, or `no_escalate`. |
| `timed_out` / `the action did not finish in time` | The plane did not report. Check the plane link; the job may still have run. |
| `probe_scan` acks a failure after 75 s | Not wired (section 6.2). |
| A setting went back to its default | `config_set` left the key out (rule 3). |
| All per-node names gone | `config_set nodes` without `nodes`. |
| The example rules came back | `config_set rules` without `rules`. |
| A change undone after 5 minutes | Staged section, no `config_confirm`. |
| `nothing is staged` | Already confirmed, reverted, or the window passed. |
| Controller rebooted after a `net` change | Not confirmed in time, or `config_revert`: it went back to the stored network config. |
| Controller gone after `factory_reset` | Expected: MQTT is off by default. Use the setup page. |
| `status` `online:false` but the controller pings | Broker settings changed and not reachable yet, or a TLS / credential problem: check the console log. |
| Old commands ran after a reconnect | Persistent session (rule 7): the broker delivered what was queued. |
| `state` lacks `dry_in` values (`null`) | The rev D firmware does not read the dry inputs yet. |
| `t_soft_off_s` of 4000 accepted | Clamped to 3600, not refused. |

## 12. HTTP equivalents

The HTTPS API uses the same dispatcher. `POST /api/v1/cmd` takes the
same envelope and returns the reply at queue time (`accepted` for a
plane job); `POST /api/v1/nodes/<target>/<action>?wait=1` waits up to
30 s for the outcome. `GET`/`PUT /api/v1/config/<section>`,
`POST /api/v1/config/confirm` and `/revert`,
`GET /api/v1/config/export`, `POST /api/v1/config/import`,
`GET /api/v1/state`. Secrets (admin password, API tokens, broker
credentials, device certificate, fleet key) are HTTPS-only. Details:
`README.md`.

## 13. Not built yet, and not tested on hardware

- No plane on the bench: every node, unit and plane verb, `state.nodes`,
  `state.planes` and the `plane` events are verified against the
  simulator and host tests only.
- `probe_scan` (1-wire re-scan) is not wired.
- Dry contact inputs: no pin map on rev D, `dry_in` stays `null`.
- `net.vlan`: ignored. `sys.coredump_report`, `sys.timezone`: unused.
- `fault` events: never emitted.
- Per-command signatures (`sig` in the envelope) are reserved and
  ignored: the broker's TLS and credentials are the authentication.
- Payload size on the way out: the MQTT out buffer is 4 KB. A `state`
  with many nodes, or `config_get` without a section, may be larger;
  how the client handles that has not been tested.
- Home Assistant discovery for the dynamic node set
  (`docs/home-assistant.md` describes the rev C controller).

### Where the code and the ADRs differ

- ADR 0004 decision 7 says `state` is published every 10 s; the code's
  default `mqtt.t_state_s` is 60 s.
- ADR 0001 component 7 lists the event kinds `node_state`,
  `action_done`, `action_failed`, `rule_fired`, `fault`, `ota`,
  `config`; the code adds `accepted`, `plane`, `switch`, `security`
  and never emits `fault`.
- ADR 0001 and `README.md` call the per-node record "last action" /
  `last_action`; the payload field is `last_job`.
- ADR 0004 says `led_sense` off means "never wait for the light"; the
  plane clamps the wait to its blink window plus 1 s and still presses
  PWR (section 5.1).
- ADR 0001 says the MQTT client id defaults to `granite-<mac6>`; the
  code uses the device id, which is the same unless `sys.device_id` is
  set.
- ADR 0001 component 5 lists `vin < 15 V` as example rule 3; the code
  ships 10 V (ADR 0004 "fixes after the first pass" records why).
