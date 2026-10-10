# Controlling nodes

Every interface, the setup page, the HTTP API, MQTT and Modbus, drives
the same actuator with the same rules. This page describes what the
actions do and what the states mean; the exact message formats are in
the reference.

## Node states

| State | Meaning |
|---|---|
| `off` | The node's power LED has been off for at least 100 ms. |
| `on` | The LED has been on for at least 100 ms. |
| `busy` | An action is running on this node; the LED is ignored until it finishes. |
| `unknown` | The sense expander cannot be read, or the node has `sense = ignore`. |

A hung node is not a state: the controller cannot tell a frozen OS
from a running one. It is `on`, and the `last_reset_ms` field tells you
when it was last reset.

## Actions

| Action | What happens | Typical duration |
|---|---|---|
| `on` | If off: short press on PWR, then wait for the LED. If already on: done, no press. | up to `t_on` (10 s) |
| `off` | If on: short press on PWR (the OS sees an ACPI power button and shuts down), then wait for the LED to go off. If it does not within `t_soft_off` (2 min), **escalate** to `force_off`. With `no_escalate` it returns `shutdown_pending` instead. | up to 2 min + 8 s |
| `force_off` | If on: hold PWR until the LED goes off plus 0.5 s, at most `t_hold` (8 s). The 4 s hard power-off. | up to 8 s |
| `reset` | If on: short press on RST. Refused when the node is off. | 0.25 s |
| `cycle` | `off` (or `force_off` with `hard`), wait `t_cycle` (10 s), then `on`. Skips the wait if the node was already off. | up to 2.5 min |
| `press` | Raw press of `pwr` or `rst` for 100..10000 ms, regardless of the LED. For motherboards that need something unusual. Always logged. | as given |
| `on_all` | `on` for every node that is off, in the configured order, `t_stagger` (5 s) apart. | about 35 s for 8 nodes |

Targets are a node number 1..8, or `all` for `on_all` only. **Current
firmware:** every other action with target `all` is refused with
`bad_target`; send one command per node.

## Refusals and failures

An action is **refused** before anything is pressed when:

| Reason | Why |
|---|---|
| `unknown_state` | The sense expander is unreadable and the command has no `force: true`. (A node with `sense = ignore` is also shown as `unknown` but is not refused.) |
| `busy` | Another action is running on that node. |
| `node_off` | `reset` on a node that is off. |
| `queue_full` | Too many queued actions. |
| `bad_target`, `bad_duration` | Malformed request. |

An action **fails** after pressing when:

| Reason | Why |
|---|---|
| `power_on_timeout` | The LED did not come on within `t_on`. Check the node's power supply, or the LED cable. |
| `still_on` | `force_off` held PWR for the whole `t_hold` and the LED is still on. The PWR wire is probably not connected to the right pins. |
| `deadline_exceeded`, `expander_fault` | The hardware safety released the relays or the expander readback disagreed. A `fault` event is published; look at the log. |
| `partial` | A group action where some nodes failed. |

Over HTTP a refusal is a 400 with the reason in the reply; a failure
after the press shows up as an `action_failed` event and in the node's
`last_action`, because the HTTP reply is sent at acceptance. Over MQTT
both are an ack with `ok: false`, published on completion. Over Modbus
holding register 9 reads 3 (refused) or 4 (failed).

## Safety properties

- One physical press at a time on the whole board, one action at a
  time per node.
- Every press has a hardware deadline (planned duration + 0.5 s, 12 s
  maximum). If the firmware stops, a timer resets the relay expander
  and every contact opens. A press can never stick.
- The relay expander is held in reset whenever the controller is
  unpowered, resetting, or booting. A controller power cut releases all
  buttons and changes nothing on the nodes.
- A controller reboot, crash or update never presses anything. Only
  the rules you enabled do (and the boot policy, once the firmware
  applies it).

## From the setup page

The Status page has per-node buttons for on, off, reset and cycle, and
a "Power on all (staggered)" button. The last action and its result
stay in the row.

## From scripts

```sh
B=https://granite-37adc7.local; T=<api token>
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/3/on
curl -sk -H "Authorization: Bearer $T" -X POST -d '{"hard":true}' $B/api/v1/nodes/3/cycle
curl -sk -H "Authorization: Bearer $T" -X POST -d '{"switch":"rst","duration_ms":1500}' $B/api/v1/nodes/5/press
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/all/on_all
curl -sk -H "Authorization: Bearer $T" $B/api/v1/state
```

The HTTP call returns as soon as the action is queued, with
`"result":"accepted"` (or a refusal as a 400). Watch the outcome in
`GET /api/v1/state` (`nodes[i].last_action`), in the MQTT `event`
stream, or in the MQTT ack, which is published on completion.
