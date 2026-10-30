# Nodes and boot policy

A node is a slot on a power plane: the slot's hot-swap rail, the
motherboard's PWR and RST buttons, and its power LED all sit on the
plane, and the plane runs every press and every sequence itself. The
controller finds the planes, gives each node its number, and keeps
what the operator decides about a node: its name, its boot policy,
whether its LED can be trusted, and how far an `off` may escalate.

The `nodes` section holds these settings (`PUT
/api/v1/config/nodes`, or `config_set` over MQTT). Changes are saved directly
(no confirm step) and take effect with the next command: the per-node
settings are read every time a verb is sent, so nothing needs a
reboot. The probe names, the probe period, `vin_trim` and the VIN
window are applied on the next sensor read.

## Node ids come from the planes

Every plane has an address strapped on the board, 0x30 to 0x37, so up
to eight planes share the plane link. A plane has eight slots, 0 to 7,
two per unit board. The node id is fixed by the two:

```text
node id = 8 x (plane address - 0x30) + slot + 1
```

| Plane address | Plane | Slots 0..7 are nodes |
|---|---|---|
| 0x30 | `plane0` | 1..8 |
| 0x31 | `plane1` | 9..16 |
| 0x32 | `plane2` | 17..24 |
| ... | ... | ... |
| 0x37 | `plane7` | 57..64 |

There is no table that maps slots to ids, and no way to renumber a
node. A node keeps its id across controller and plane reboots, across
re-discovery, and across a plane swap with the same straps. If a frame
numbers its nodes left to right, strap the planes in frame order; the
names carry whatever labels the site uses.

A slot without a module, or behind a unit board that is not on the
chain, is not listed in the state payload. Its settings are kept
anyway, so a plane that is unplugged for a day comes back with the
names and policies it had.

## Verify every plane and node first

Cable the planes, power the frame, and read `GET /api/v1/planes` and
`GET /api/v1/state`. (The setup page has not caught up with the plane
model yet: in the current firmware its node table still shows the rev C
columns, so use the API, or `mosquitto_sub` on the `state` topic, for
this check.) For each plane check:

| What you see | Meaning |
|---|---|
| The plane is listed with its number, `type`, `firmware` and `bus_v` | Good. The controller found it at boot or on the 10 s re-scan of the missing addresses. |
| A plane is missing | Its address straps, the plane link cable, or the plane itself. A plane that answered once and then stopped raises a `plane_lost` event and drops out of `planes` (and of Modbus input register 15). |
| An `address_collision` event | One address keeps answering with corrupt frames, usually because two planes are strapped the same. Re-strap one. |
| A unit board shows `present: false` | The chain cable to that unit board. Its two slots are not nodes until it answers. |

Then for each node:

| What you see | Meaning |
|---|---|
| `power` `on`, `host` `on`, and the board really is running | Good. |
| `power` `on`, `host` `off`, and the board really is running | The PLED wires are not connected, swapped with the switch wires, or the motherboard drives too little current. Without the LED, `on` and `off` cannot tell what the host does. Fix the cable, or set `led_sense` to false for that node. |
| `power` `on`, `host` `sleep` | The LED blinks: the host sleeps. |
| `power` `off` | The rail is down, and the host with it. `host` is `unknown` while the rail is off. |
| `power` `fault` | The slot latched or tripped; `faults` names why. See [Controlling nodes](../operation/nodes.md). |
| A node you expected is not listed | No module in that slot, or its unit board is missing. |

Send `on` and `off` to one node (`POST /api/v1/nodes/<id>/on`) and
watch `power` and `host` follow. Every command and its outcome is an
event and visible in the Maintenance log. Do this for every node
before sealing; a cable mistake is cheap now and expensive later.

## Per node

The section holds a list of entries keyed by node id. A node with no
entry uses the defaults below, so a new controller needs no entries at
all. An entry for an id outside 1..64 is dropped when saved.

| Setting | Values | Default | Meaning |
|---|---|---|---|
| `node` | 1..64 | | The node id the entry is for. |
| `name` | text | `node<N>` | Shown on the page and in the state payload. Empty means the default. |
| `boot_policy` | `leave`, `on`, `off` | `leave` | What to send the node once after the controller starts. See below. |
| `led_sense` | bool | `true` | Whether the plane's reading of the host LED is trusted. With `false`, `on`, `cycle` and `on_all` do not wait the usual 15 s for the light: the plane waits only its own minimum settle time before it presses PWR. |
| `t_soft_off_s` | seconds, 1..3600 | 120 | How long `off` waits for the host to go dark after the short PWR press before it escalates. `0` means the default; anything above 3600 is clamped to 3600. |
| `escalation` | `none`, `hold`, `hold_then_cut` | `hold_then_cut` | How far `off` and `force_off` go when the host stays lit. `none`: stop and fail. `hold`: hold PWR 5 s and leave the rail alone. `hold_then_cut`: hold PWR 5 s, then cut the rail if the LED is still on. |

With the default `hold_then_cut`, `force_off` always ends with the node
off. A board that obeyed the hold keeps its standby power; only a board
that ignored its button loses the rail.

`PUT /api/v1/config/nodes` replaces the whole section, and a field
left out takes its default, so send every entry you want to keep:

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT -d '{
  "nodes": [
    {"node": 1, "name": "db-1", "boot_policy": "on"},
    {"node": 9, "name": "web-1", "t_soft_off_s": 300, "escalation": "hold"},
    {"node": 12, "led_sense": false}
  ],
  "plane_update": "manual",
  "probes": [{"rom": "28ff000000000001", "name": "inlet"}],
  "t_probe_s": 10, "vin_trim": 1.0, "vin_min_mv": 10000, "vin_max_mv": 14000
}' https://granite-37adc7.local/api/v1/config/nodes
```

## Boot policy

The policies run once per **controller** start, three seconds after
the dispatcher comes up, which is long enough for discovery and a slot
report from every plane. Each node whose policy is not `leave` gets one
command with the id `boot-<node>`:

| Policy | Command | Effect |
|---|---|---|
| `leave` | none | The node is never touched. |
| `on` | `on` | Rail on if it is off, then a PWR press if the host stays dark. A running node is left alone. |
| `off` | `off` | Soft shutdown of the host, escalating per the node's setting. `off` means "shut the host down": a node whose host is already dark keeps its rail up. |

A node whose plane is not there yet is refused like any absent node,
and the log says so; it does not get a second try. A plane that
reboots later on its own does not run the policies again: the plane
keeps its slots as they were (see [Controlling nodes](../operation/nodes.md)).

Boot policy `on` is the one that brings a frame back after a site power
loss without anyone logging in. Keep in mind that it runs after every
controller start, including a firmware update or a reboot from the
API, and presses PWR on any node whose host is dark at that moment.
Boot policy `off` shuts the host down on every controller start and is
rarely what you want.

## Plane firmware updates

`plane_update` decides what happens when a plane runs a firmware
version other than the one the controller carries:

| Value | Meaning |
|---|---|
| `manual` (default) | Nothing happens until an update is asked for. |
| `auto` | After the controller's own image is confirmed, every plane whose version differs is updated, one at a time, never while a sequence runs on it. |

`GET /api/v1/planes` shows the policy as `update_policy`. The update
itself is described in [Firmware updates](../operation/firmware-updates.md).

## Probes

The probe table lists the DS18B20 sensors by 64-bit ROM id. Up to eight
slots. Give each a name (`inlet`, `outlet`, `plane 1 heatsink`) and the
name travels into the state payload and the rules. The read period
(`t_probe_s`, default 10) is shared by all probes.

"Scan the 1-wire bus" (`POST /api/v1/probes/scan`) is accepted by the
current firmware but the re-scan is not yet wired to the bus; probes
are enumerated at boot. Plug all probes in, reboot the controller,
and the table fills.

## Controller input voltage

The rev D controller is fed 12 V from up to two planes. `vin_trim`
(default 1.0) multiplies the measured input voltage to calibrate out
resistor tolerance: compare `vin_v` in the state payload with a meter
and set the ratio.

`vin_min_mv` and `vin_max_mv` (10000 and 14000 by default) are the
window the input is expected to stay in. Outside it the `vin` fault bit
is set (Modbus input register 13, bit 3); either bound at `0` switches
that side of the check off, and `vin_max_mv` cannot exceed 36000. The
window only reports: acting on a sagging supply is a rule's job.
