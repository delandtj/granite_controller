# Rules and standalone behaviour

Rules run on the controller itself, every second, whether or not the
broker, the network or anyone else is reachable. They are the frame's
local reflexes: over-temperature, leak, bus voltage. Up to 16 rules
live in the `rules` section, saved directly without a confirm step.

## Shipped examples

Three rules come with the default configuration, all **disabled**:

| Id | Name | Condition | Hold | Action | Re-arm |
|---|---|---|---|---|---|
| 1 | probe over temperature | hottest probe > 70.00 C | 30 s | force all nodes off | auto, hysteresis 5.00 C |
| 2 | leak float closed | dry contact 1 closed | 2 s | force all nodes off | manual |
| 3 | bus voltage low | VIN < 15.000 V | 5 s | event only | auto, hysteresis 0.5 V |

Review them, change thresholds and targets to your frame, and enable
the ones you want. Rule 2 assumes your leak float is on dry input 1.

**Current firmware:** the actuator accepts target `all` only for
`on_all`; every other action needs one node. A rule with
`force_off` on `all` therefore fires, publishes `rule_fired`, and the
action is refused. Until that is fixed, write one rule per node (eight
rules for a frame-wide force-off), or use `on_all` where that is the
intent.

## Anatomy of a rule

| Field | Values | Meaning |
|---|---|---|
| `id` | 1..255, unique | Used in `rule_fired` events and acks. |
| `enabled` | bool | |
| `name` | text | For humans. |
| `source` | see below | What is measured. |
| `n` | 1..8 or 1..4 | The probe, node or dry input, for sources that need one. |
| `op` | `>`, `<`, `==`, `changed` | `changed` fires once whenever the value differs from the previous second and ignores hold and hysteresis. |
| `threshold` | integer | In the source's **native unit** (see below). |
| `hysteresis` | integer | How far the value must come back before an `auto` rule can fire again. |
| `hold_s` | seconds | The condition must be true continuously this long. |
| `action` | `act` with a `kind`, or `event` | `act` runs a node action (`on`, `off`, `force_off`, `reset`, `cycle`, `press`, `on_all`); `event` only publishes `rule_fired`. |
| `target` | 1..8 or `all` | `all` works only with `on_all` in the current firmware; see above. |
| `rearm` | `auto` (default) or `manual` | `manual` stays fired until acknowledged. |

Sources and their units:

| `source` | Needs `n` | Unit | Example threshold |
|---|---|---|---|
| `probe` | 1..8 | 0.01 C | `7000` = 70.00 C |
| `probe_max` | | 0.01 C | hottest answering probe |
| `board_temp` | | 0.01 C | the controller's own sensor |
| `vin` | | mV | `15000` = 15 V |
| `dry_in` | 1..4 | 0 or 1 | `1` = contact closed |
| `node_on` | 1..8 | 0 or 1 | `1` = power LED on |
| `mqtt_connected` | | 0 or 1 | |
| `link_up` | | 0 or 1 | |
| `uptime_s` | | s | |

A source with no value (a probe that did not answer, a sensor never
read) makes every comparison false, so a missing probe never fires an
over-temperature rule by itself. Pair a critical probe with a
`fault_flags` check on your monitoring side instead.

## Acknowledging a manual rule

A `manual` rule that has fired will not fire again until it is
acknowledged, which is what you want for a leak: the nodes stay off
until a person has looked. Acknowledge from the Rules page, with
`POST /api/v1/rules/<id>/ack`, or over MQTT with
`{"action":"rule_ack","args":{"rule":2}}`. The `rule_fired` event
carries `needs_ack: true` for these.

## Examples

Force node 4 off when its own heatsink probe (slot 4) passes 85 C for
10 s, and allow a retry once it is 5 C cooler:

```json
{"id":10,"enabled":true,"name":"node 4 heatsink","source":"probe","n":4,
 "op":">","threshold":8500,"hysteresis":500,"hold_s":10,
 "action":"act","kind":"force_off","target":4,"rearm":"auto"}
```

Publish an event when the lid switch on dry input 2 opens or closes:

```json
{"id":11,"enabled":true,"name":"lid","source":"dry_in","n":2,
 "op":"changed","threshold":0,"hysteresis":0,"hold_s":0,
 "action":"event","target":"all","rearm":"auto"}
```

Power a node back on 30 s after it was found off, for a node that must
always run (be careful: this fights a deliberate shutdown):

```json
{"id":12,"enabled":false,"name":"keep node 1 up","source":"node_on","n":1,
 "op":"==","threshold":0,"hysteresis":0,"hold_s":30,
 "action":"act","kind":"on","target":1,"rearm":"auto"}
```

## What the controller does on its own

Without any enabled rule the controller never changes node power (the
per-node boot policy is not applied by the current firmware). With the
network gone it keeps
measuring, keeps evaluating rules, keeps serving the local page if
anyone can reach it, and reconnects to the broker when it returns.
Rule firings while offline are in the log ring (Maintenance page,
`log` on the console), and the retained `state` shows their result
once the broker session is back.
