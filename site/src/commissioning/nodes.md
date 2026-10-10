# Nodes and boot policy

The Nodes page edits the `nodes` section: names, boot policy, sense,
power-on order, timings and probe slots. Changes here are saved
directly (no confirm step). Names, order, timings and sense mode take
effect at once; probe names, the probe read period and the bus voltage
trim are read at boot, so reboot the controller after changing those.

## Verify every node's LED sense first

Cable the nodes, power the frame, and look at the Status page. Each
node row shows its `state` and the raw `LED` reading:

| What you see | Meaning |
|---|---|
| `off`, LED off, and the node really is off | Good. |
| `on`, LED on, and the node really is on | Good. |
| `off` while the node is running | The PLED wires are not connected, swapped with the switch wires, or the motherboard drives too little current. Fix the cable: without the LED the controller cannot see this node. |
| `unknown` on every node | The sense expander did not answer at boot (fault bit 1 in the register map). Check the board; the console log says why. |
| `unknown` on one node | That node has `sense = ignore` configured; actions on it are timed presses and are not refused. |

Then use the row's action buttons to power one node on and off and
watch the LED follow. Pressing is logged as an event and visible in the
Maintenance log. Do this for all eight nodes before sealing; a cable
mistake is cheap now and expensive later.

A node that reads `unknown` because the sense expander cannot be read
refuses `on`, `off`, `reset` and `cycle`, since the controller cannot
tell what the press would do. Commands can carry `"force": true` to
override that from MQTT or the API, and the raw `press` action never
looks at the LED.

## Per node

| Setting | Values | Meaning |
|---|---|---|
| Name | text, default `node1`..`node8` | Shown on the page and in the state payload. |
| Boot policy | `leave` (default), `on`, `off` | Meant to be applied once per **controller** boot, 5 s after the expanders are up, in the power-on order and with the stagger interval. `leave` never touches the node. **Current firmware:** the setting is stored and shown but not applied on the board; only the simulator runs it. |
| Sense | `enabled` (default), `ignore` | `ignore` treats the node as if it had no LED: every action becomes a timed press with the standard durations, and `force_off` holds PWR for the full hold time. Use it only for a node whose LED cannot be wired. |

Boot policy `on` is meant to bring a frame back after a site power
loss without anyone logging in: the controller boots from the 19 V
bus, waits for its expanders, and presses power on each node that is
still off, staggered. Until the firmware applies it, send `on_all`
from your automation when the controller's retained `status` comes
back online. Boot policy `off` is rarely what you want.

## Power-on order and stagger

`order` is the list of node ids in the sequence used by `on_all` and by
the boot policy. The stagger interval (`t_stagger_ms`, default 5000)
is the delay between successive presses, so eight nodes come up over
about 35 s instead of all at once on the 19 V bus.

## Timings

All in milliseconds; the page shows the allowed range of each, and a
value outside it is clamped to the nearest bound when saved.

| Timing | Default | Range | Used by |
|---|---|---|---|
| `t_short_ms` | 250 | 100..1000 | the short press of `on`, `off`, `reset` |
| `t_hold_ms` | 8000 | 4000..10000 | the maximum hold of `force_off` |
| `t_on_ms` | 10000 | 1000..300000 | how long `on` waits for the LED |
| `t_soft_off_ms` | 120000 | 5000..900000 | how long `off` waits for the OS to shut down before escalating |
| `t_cycle_ms` | 10000 | 1000..300000 | the pause between off and on in `cycle` |
| `t_stagger_ms` | 5000 | 0..60000 | the gap between nodes in `on_all` and the boot policy |
| `t_settle_ms` | 5000 | 0..60000 | the wait after boot before the boot policy runs |

Independently of these, a hardware deadline releases every relay if a
press lasts longer than its planned duration plus 500 ms (12 s at
most), even if the firmware hangs. That is what makes a stuck-button
force-off impossible.

## Probes

The probe table lists the DS18B20 sensors by 64-bit ROM id. Up to eight
slots. Give each a name (`inlet`, `outlet`, `node 3 heatsink`) and the
name travels into the state payload and the rules. The read period
(`t_probe_s`, default 10) is shared by all probes.

"Scan the 1-wire bus" (`POST /api/v1/probes/scan`) is accepted by the
current firmware but the re-scan is not yet wired to the bus; probes
are enumerated at boot. Plug all probes in, reboot the controller,
and the table fills.

## Bus voltage trim

`vin_trim` (default 1.0) multiplies the measured 19 V bus voltage to
calibrate out resistor tolerance. Compare `vin_v` on the Status page
with a meter on J3, set the ratio, and reboot for it to apply.
