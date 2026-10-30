# Rules and standalone behaviour

Rules run on the controller itself, every second, whether or not the
broker, the network or anyone else is reachable. They are the frame's
local reflexes: over-temperature, leak, a sagging bus, a node that
draws too much. Up to 16 rules live in the `rules` section, saved
directly without a confirm step. A rule reads the same snapshot the
state payload is built from, so it never waits for a plane.

## Shipped examples

Three rules come with the default configuration, all **disabled**:

| Id | Name | Condition | Hold | Action | Re-arm |
|---|---|---|---|---|---|
| 1 | probe over temperature | hottest probe > 70.00 C, **or no probe answering** | 30 s | `force_off` on `all` | auto, hysteresis 5.00 C |
| 2 | leak float closed | dry contact 1 closed | 2 s | `force_off` on `all` | manual |
| 3 | bus voltage low | VIN < 15.000 V | 5 s | event only | auto, hysteresis 0.5 V |

Do not enable them as shipped:

- Rules 1 and 2 target `all`, and the current firmware accepts `all`
  only for `on_all`. When they fire they publish `rule_fired` and then
  an `action_failed` event (`force_off needs a single node target`),
  and no node is switched off. Until that is fixed, give each node its
  own rule with a numeric `target` (16 rules is the limit).
- Rule 3 still carries the 15 V threshold of the 19 V rev C supply.
  The rev D controller runs on 12 V from the planes, so it would fire
  at once. Set the threshold to something like `10500` first, or watch
  a plane's bus instead with `plane_bus_volts`.

Rule 2 assumes your leak float is on dry input 1.

## Anatomy of a rule

| Field | Values | Meaning |
|---|---|---|
| `id` | 1..255, unique | Used in `rule_fired` events and acks. |
| `enabled` | bool | |
| `name` | text | For humans. |
| `source` | see below | What is measured. |
| `n` | 1..8, 1..4 or 1..64 | The probe, dry input or node, for sources that need one. |
| `p` | 0..7 | The plane, for the plane and unit sources. |
| `u` | 0..3 | The unit board, for the unit sources. |
| `op` | `>`, `<`, `==`, `changed` | `changed` fires once whenever the value differs from the previous second and ignores hold and hysteresis. |
| `threshold` | integer | In the source's **native unit** (see below). |
| `hysteresis` | integer | How far the value must come back before an `auto` rule can fire again. |
| `hold_s` | seconds | The condition must be true continuously this long. |
| `action` | `act` with a `kind`, or `event` | `act` sends a verb; `event` only publishes `rule_fired`. |
| `kind` | any verb of [Controlling nodes](../operation/nodes.md) | `on`, `off`, `force_off`, `reset`, `cycle`, `press`, `on_all`, `power_on`, `power_off`, `clear_latch`, `unit_reset`, `plane_reboot`. |
| `target` | `1`..`64`, `all`, `planeN`, `planeN.unitU` | What the verb acts on, with the same rules as any command: a node id for the node verbs, `all` only for `on_all`, `planeN` for `plane_reboot`, `planeN.unitU` for `unit_reset`. |
| `rearm` | `auto` (default) or `manual` | `manual` stays fired until acknowledged. |
| `missing` | `ignore` (default) or `trip` | What a source with no value means: `ignore` waits for a value, `trip` counts it as the condition being true (hold included). Not for `changed`. |

Sources and their units:

| `source` | Needs | Unit | Example threshold |
|---|---|---|---|
| `probe` | `n` 1..8 | 0.01 C | `7000` = 70.00 C |
| `probe_max` | | 0.01 C | hottest answering probe |
| `board_temp` | | 0.01 C | the controller's own sensor |
| `vin` | | mV | `10500` = 10.5 V, the controller's input |
| `dry_in` | `n` 1..4 | 0 or 1 | `1` = contact closed |
| `node_on` | `n` 1..64 | 0 or 1 | `1` = the node's rail is on |
| `node_host_on` | `n` 1..64 | 0 or 1 | `1` = the host LED is steady |
| `node_power` | `n` 1..64 | code | 0 unknown, 1 off, 2 on, 3 starting, 4 fault, 5 absent |
| `node_host` | `n` 1..64 | code | 0 unknown, 1 off, 2 on, 3 sleep |
| `node_volts` | `n` 1..64 | mV | slot voltage |
| `node_amps` | `n` 1..64 | mA | `8000` = 8 A |
| `node_watts` | `n` 1..64 | 0.1 W | `4000` = 400 W |
| `node_temp` | `n` 1..64 | 0.01 C | `8500` = 85 C (the plane reads whole tenths) |
| `plane_bus_volts` | `p` 0..7 | mV | `44000` = 44 V on the plane's input |
| `unit_amps` | `p`, `u` | mA | a unit board's input current |
| `unit_watts` | `p`, `u` | 0.1 W | a unit board's input power |
| `mqtt_connected` | | 0 or 1 | |
| `link_up` | | 0 or 1 | |
| `uptime_s` | | s | |

A source with no value (a probe that did not answer, a sensor never
read, a node that is not present, a plane that does not answer) makes
the comparison false, unless the rule says `"missing": "trip"`. Use
`trip` where a silent sensor is as bad as the reading it guards:
shipped rule 1 has it, because with every probe gone there is no
temperature watch at all. Do not enable such a rule on a frame without
probes; it would fire after its hold time. The `rule_fired` event of a
missing value has `"value": null`.

## What a rule's action can and cannot do

A rule's command goes through the same dispatcher as a command from
MQTT or the API, with the id `rule-<id>`, and is refused or fails for
the same reasons:

- a node, unit or plane that is not present is refused, and the rule
  publishes an `action_failed` event;
- a verb on a slot where the plane is already running a sequence (an
  `off` still waiting for its host, say) fails with `failed: conflict`;
  it does not take over the running sequence. The plane finishes what
  it is doing first.

A `force_off` from a rule carries `force`, as in earlier firmware, but
the plane decides on presence and there is nothing for it to override.

## Acknowledging a manual rule

A `manual` rule that has fired will not fire again until it is
acknowledged, which is what you want for a leak: the nodes stay off
until a person has looked. Acknowledge with
`POST /api/v1/rules/<id>/ack`, or over MQTT with
`{"action":"rule_ack","args":{"rule":2}}`. The `rule_fired` event
carries `needs_ack: true` for these.

## Examples

Force node 4 off when its slot passes 85 C for 10 s, and allow a retry
once it is 5 C cooler:

```json
{"id":10,"enabled":true,"name":"node 4 hot","source":"node_temp","n":4,
 "op":">","threshold":8500,"hysteresis":500,"hold_s":10,
 "action":"act","kind":"force_off","target":4,"rearm":"auto"}
```

Force node 1 off when the leak float on dry input 1 closes, and keep it
off until someone acknowledges (one such rule per node):

```json
{"id":11,"enabled":true,"name":"leak node 1","source":"dry_in","n":1,
 "op":"==","threshold":1,"hysteresis":0,"hold_s":2,
 "action":"act","kind":"force_off","target":1,"rearm":"manual"}
```

Publish an event when node 9 draws more than 450 W for 30 s:

```json
{"id":12,"enabled":true,"name":"node 9 power","source":"node_watts","n":9,
 "op":">","threshold":4500,"hysteresis":200,"hold_s":30,
 "action":"event","target":"all","rearm":"auto"}
```

Publish an event when a node's rail goes to `fault` (4):

```json
{"id":13,"enabled":true,"name":"node 9 fault","source":"node_power","n":9,
 "op":"==","threshold":4,"hysteresis":0,"hold_s":0,
 "action":"event","target":"all","rearm":"auto"}
```

Publish an event when the lid switch on dry input 2 opens or closes:

```json
{"id":14,"enabled":true,"name":"lid","source":"dry_in","n":2,
 "op":"changed","threshold":0,"hysteresis":0,"hold_s":0,
 "action":"event","target":"all","rearm":"auto"}
```

Power a node back on 30 s after its rail was found off, for a node that
must always run (be careful: this fights a deliberate shutdown):

```json
{"id":15,"enabled":false,"name":"keep node 1 up","source":"node_on","n":1,
 "op":"==","threshold":0,"hysteresis":0,"hold_s":30,
 "action":"act","kind":"on","target":1,"rearm":"auto"}
```

## What the controller does on its own

Without any enabled rule, the controller changes node power only
through the boot policies of [Nodes and boot policy](nodes.md) (once
per controller start, `leave` by default) and by re-applying OFF to the
nodes it had switched off when a plane reports a power loss
([Controlling nodes](../operation/nodes.md)). The planes keep running
their nodes whatever the controller does.

With the network gone the controller keeps measuring, keeps evaluating
rules, keeps serving the local page if anyone can reach it, and
reconnects to the broker when it returns. Events while offline,
`rule_fired` included, are held (the last 64) and published when the
broker session is back; the log ring (Maintenance page, `log` on the
console) has them too.

The thread that runs the rules and the plane client, and the one that
reads the sensors, are under the task watchdog: if either stops for
10 s the controller restarts (boot reason `task_watchdog`) instead of
sitting there with its rules silently stopped. A controller restart
does not move any node.
