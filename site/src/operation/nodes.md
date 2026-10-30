# Controlling nodes

Every interface, the HTTP API, MQTT, Modbus and the rules, sends the
same commands through the same dispatcher. The controller does not
press anything itself: it checks the target, hands the verb to the
plane that holds the node, and the plane runs the sequence (rail,
buttons, LED, timeouts, escalation) on its own. This page describes
what the verbs do and what the states mean; the exact message formats
are in the reference.

## Node state

A node has two state axes, the rail and the host, plus telemetry.

`power`, the slot's rail:

| State | Meaning |
|---|---|
| `off` | The rail is switched off: by a command, by the plane's retry limit, or by an escalated `off`. |
| `on` | The hot-swap reports power good; the slot carries the node. |
| `starting` | The rail is enabled and waiting for power good. |
| `fault` | The overvoltage latch is set, the power limit tripped, the retry limit was reached, or the unit board's own hot-swap latched off. |
| `absent` | No module in the slot, or no unit board on that part of the chain. |
| `unknown` | The plane is not answering, or has not answered yet. |

`host`, the motherboard, read from its power LED:

| State | Meaning |
|---|---|
| `off` | The LED is dark. |
| `on` | The LED is steady. |
| `sleep` | The LED blinks: the host sleeps. |
| `unknown` | The rail is off, or there is no reading. |

A hung node is not a state: nothing can tell a frozen OS from a running
one. It is `on`, and its `last_job` tells you when it was last reset.

Each node also reports `v`, `a`, `w` (and `w_min` / `w_max` since the
plane last cleared them), `temp_c`, `energy_wh` (since the plane's
first boot), `module_class`, `busy` while the plane runs a sequence on
the slot, and `faults`, a list of names:

| Fault | Meaning |
|---|---|
| `absent` | No module, or no unit board. |
| `ov_latched` | The slot's overvoltage latch is set; the rail stays off until `clear_latch`, which you send after finding the cause. |
| `power_limit` | The slot exceeded the power limit in the plane's config. |
| `retry_limit` | The hot-swap dropped power good too often within the plane's window, and the plane switched the slot off. |
| `unit_latched_off` | The unit board's own hot-swap latched off; the slots of that unit are down until `unit_reset`. |
| `cross_check` | The LED and the rail disagree. |

Neither latch is ever cleared automatically: an overvoltage is the
fault that kills a node, so a person decides.

## Verbs

| Verb | Target | What happens | Typical duration |
|---|---|---|---|
| `on` | node, `all` | If the host LED is already steady: done, nothing changes. Otherwise rail on if it is off, wait up to 15 s for the LED, press PWR once if it stayed dark, and wait again. | up to about 30 s |
| `off` | node, `all` | If the host is already dark: done, the rail stays up. Otherwise a short PWR press (the OS sees an ACPI power button and shuts down), then wait `t_soft_off_s` (default 120 s) for the LED to go dark, then **escalate** per the node's `escalation`. | up to `t_soft_off_s` + 6 s |
| `force_off` | node, `all` | No short press: hold PWR 5 s at once, then cut the rail if the LED is still on (with the default escalation). | about 6 s |
| `reset` | node, `all` | A 300 ms press on RST. | 0.3 s |
| `cycle` | node, `all` | Rail off, wait 5 s, rail on, then a PWR press if the host stays dark after the LED wait. | about 5 s + up to 15 s |
| `press` | node, `all` | Raw press of PWR (default) or RST, 50..10000 ms (default 300). Ignores the LED. For motherboards that need something unusual. | as given |
| `on_all` | all | Staggered start of the present nodes that are off, see below. | 2 s per slot, then the `on` waits |
| `power_on` | node, `all` | The rail only: enable the slot. No button. | under a second |
| `power_off` | node, `all` | The rail only: disable the slot. No button; the host loses power at once. | under a second |
| `clear_latch` | node, `all` | Clear the slot's overvoltage latch. | under a second |
| `unit_reset` | `planeN.unitU` | Re-arm a unit board's own hot-swap after it latched off (a 100 ms pulse). Never done by `on`. | under a second |
| `plane_reboot` | `planeN` | Reboot the plane. The slots keep their state: the plane saves it first and restores it on the way up. | a few seconds |

`off` and `power_off` are different on purpose. `off` means "shut the
host down" and leaves the standby rail up when the host obeyed; only
`power_off` (or an escalation that ends in a cut) takes the rail down.

The escalation of `off` and `force_off` comes from the node's setting
(see [Nodes and boot policy](../commissioning/nodes.md)): `none` stops
and fails, `hold` holds PWR for 5 s and leaves the rail, and the
default `hold_then_cut` also cuts the rail if the LED is still on after
the hold. With `none`, `force_off` has nothing left to do and fails at
once.

### `on_all`

`on_all` takes every node the controller sees as present, ignores the
target it was sent with, and runs in two stages: first each plane
switches **every one of those slots off** and brings them back one at a
time, 2 s apart, so the inrush of eight boards does not land at once;
then each slot gets an `on`, which presses PWR on the hosts that stayed
dark. A node that was running when `on_all` arrived therefore loses
its rail for a moment and has to boot again. Use it to bring up a dead
frame, not as "make sure everything is on"; for that, send `on` to each
node, which leaves running nodes alone.

### Arguments

| Arg | Used by | Meaning |
|---|---|---|
| `no_escalate` | `off`, `force_off` | Escalation `none` for this command only. |
| `hard` | `on`, `cycle`, `on_all` | Do not wait for the host LED: the plane presses PWR after its minimum settle time. |
| `switch` | `press` | `pwr` (default) or `rst`. |
| `duration_ms` | `press` | 50..10000, clamped to that range. Default 300. |
| `force` | any | Accepted for older clients and carried, but it overrides nothing: the plane decides whether a node is there. |

## Targets

| Target | Spelling | Used by |
|---|---|---|
| One node | `1`..`64` | every node verb |
| Every present node | `all` | every node verb |
| One plane | `plane0`..`plane7` | `plane_reboot` |
| One unit board | `plane0.unit0`..`plane7.unit3` | `unit_reset` |

A node verb on `all` is **one job per present node**, each with that
node's own settings and running on its own slot, exactly as if you had
sent the verb to each node in turn. It is accepted once (with the first
job id of the group) and acked once, when the last of those jobs has
reported; a group whose jobs did not all reach their goal is acked with
`<n> of <m> failed: <outcome>`, and each node's own result is in its
`last_job`. `all` with no present node at all is refused with
`no node is present`.

A node verb sent to a `planeN` or `planeN.unitU` target is refused with
`<verb> needs a single node target`.

## Commands complete later

A command is checked and **accepted**, and the reply carries the plane
job it became:

```json
{"v":1,"id":"api-9","ok":true,"result":"accepted","error":null,"job":12}
```

The plane reports the outcome some time later. It then shows up as an
`action_done` or `action_failed` event, as the node's `last_job` in the
state payload, as the MQTT ack, and as the HTTP result of `?wait=1`:

| Outcome | Meaning |
|---|---|
| `done` | The plane did what was asked. |
| `failed: <code>` | The plane refused it or the sequence did not reach its goal. |
| `absent: <code>` | The node, unit or plane was not there when the plane looked. |
| `link_lost` | The plane stopped answering before it reported a result. |
| `timed_out` | The plane answered but never finished (after 10 minutes). |

The `<code>` is the plane's result code: `busy`, `bad_argument`,
`conflict` (a sequence is already running on that slot, for example an
`off` still waiting for the host), `not_present`, `refused` or `failed`.
A verb sent to a node with a sequence running is not queued behind it;
it fails with `failed: conflict`. Wait for `busy` to clear, or for the
running job's outcome.

### Refused at once

A command the controller can already tell is pointless is refused
before anything is written to a plane, with `ok: false` and a reason:

| Reason | Why |
|---|---|
| `node N is on plane P which is not present` | That plane does not answer. |
| `node N is not present` | The plane answers, but the slot holds no module. |
| `unit U of plane P is not present` | `unit_reset` on a unit board that is not on the chain. |
| `plane P is not present` | `plane_reboot` or `unit_reset` on a plane that does not answer. |
| `<verb> needs a single node target` | A plane or unit target with a node verb. |
| `this action needs a plane target`, `this action needs a unit target` | `plane_reboot` or `unit_reset` with the wrong target. |
| `no node is present` | `on_all`, or a node verb on `all`, with no present node to act on. |

A node on a plane that answers but has not reported its slots yet is
let through; the plane has the last word on presence.

Over HTTP a refusal is a 400 with the reason in `error`. Over MQTT it is
an ack with `ok: false`, published at once. Over Modbus holding
register 9 reads 3 (refused) or 5 (absent).

## When something reboots

**The controller reboots** (power cut, crash, update, `reboot`). The
planes keep their state and finish any sequence they are running; no
rail and no button changes. The controller rediscovers the planes and
reads their state. The only commands it sends on its own after a start
are the boot policies of nodes that have one other than `leave`
([Nodes and boot policy](../commissioning/nodes.md)) and the actions of
enabled rules. Jobs that were pending when it went down are not
reported: their outcome is in the node's state, not in an ack.

**A plane reboots** (watchdog, `plane_reboot`, a firmware update). The
plane restores every slot it had switched off from its backup
registers, so an OFF slot never comes on. The controller sees the boot
count change, raises a `plane_rebooted` event and shows the nodes the
plane reports. Boot policies are not run.

**A plane loses power.** Its backup registers are gone, the hardware
defaults bring every slot up, and the plane flags `intended_state_lost`.
The controller keeps the set of nodes it last commanded rail-off (in
flash, so this also works when both rebooted): every node switched off
with `power_off`, or by an `off` / `force_off` that ended with the rail
cut. While the flag is up, each of those nodes whose slot reports
itself enabled again is commanded off, once per slot report, and an
`intended_off_reapplied` event lists them. A node is taken out of that
set by any verb that wants its rail up (`on`, `on_all`, `power_on`,
`cycle`, `clear_latch`), so the controller never fights an operator
who powered a node on.

**A unit board loses its 3V3** (chain cable). Its slots come up on
the hardware default while it is gone; when it is back the plane
configures it again and puts OFF slots back OFF, with a
`unit_reset_seen` event.

**The controller goes away** (unplugged, link cable cut). The planes
do nothing: every node stays as it is. The controller cannot act on a
node without its plane.

## Safety properties

- One sequence per slot at a time; a second verb on a busy slot fails
  with `conflict` instead of interleaving presses.
- Presses are timed by the plane and released again at every plane
  boot. A plane that crashes mid-press holds the button for well under
  a second, far short of the 4 s a motherboard needs for a forced off.
- A controller reboot, crash or update never presses anything and
  never moves a rail. Only the boot policies and the rules you enabled
  do.
- Neither the overvoltage latch nor a unit's hot-swap latch is cleared
  automatically.

## From scripts

```sh
B=https://granite-37adc7.local; T=<api token>
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/3/on
curl -sk -H "Authorization: Bearer $T" -X POST -d '{"hard":true}' $B/api/v1/nodes/3/cycle
curl -sk -H "Authorization: Bearer $T" -X POST -d '{"switch":"rst","duration_ms":1500}' $B/api/v1/nodes/5/press
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/12/power_off
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/plane1.unit0/unit_reset
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/plane1/plane_reboot
curl -sk -H "Authorization: Bearer $T" $B/api/v1/state
```

The HTTP call returns as soon as the job is accepted, with
`"result":"accepted"` and the `job` number (or a refusal as a 400).
Watch the outcome in `GET /api/v1/state` (`nodes[i].last_job`), in the
MQTT `event` stream, or in the MQTT ack, which is published on
completion.

A script that wants the outcome in the reply adds `?wait=1`:

```sh
curl -sk -H "Authorization: Bearer $T" -X POST "$B/api/v1/nodes/9/clear_latch?wait=1"
# {"v":1,"id":"api-11","ok":true,"result":"done","error":null,"job":3}
curl -sk -H "Authorization: Bearer $T" -X POST "$B/api/v1/nodes/10/on?wait=1"
# 400: {"v":1,"id":"api-14","ok":false,"result":null,"error":"failed: conflict","job":7}
```

The wait is capped at 30 seconds; past that the reply is `accepted`
again and the job carries on. The controller has one HTTP server task,
so while a request waits nothing else is answered - the setup page
included. Keep `?wait=1` for scripts, one at a time, and keep reading
`last_job` for anything slower than the cap (a soft `off` with a long
`t_soft_off_s`).
