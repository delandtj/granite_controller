# Controller rev D: nodes on planes

**Status**: Accepted (implemented 2026-10-29; see "As built" below)
**Date**: 2026-10-29

Related: firmware/docs/adr/0001-firmware-architecture.md (the rev C
controller firmware this builds on), 0002-plane-core-firmware.md,
0003-plane-link-protocol.md, docs/adr/0001-48v-power-plane.md
("Controller rev D"), docs/adr/0005-chained-plane-boards.md.

---

## Context

The rev C controller presses each node's PWR / RST button through its
own photoMOS (U14 MCP23017, 16 relays), reads the power LED (U15), and
runs the press timing itself (actuator.rs, hardware deadline on
GPIO10). A node is "a pair of buttons and an LED", nodes 1-8, fixed.

With the plane, all of that sits on the plane: per slot a hot-swap
rail, the buttons, the LED, V / I / P, energy, temperature, module ID,
latches, and the sequencing (ADR 0002). The controller's own node
section is dead weight, and its model is too small: a node now has a
power rail with telemetry and faults, and a controller serves up to 8
planes of up to 8 slots.

What the controller keeps (ADR 0001 of the plane): ESP32-C6, W5500
Ethernet, USB-C service port, DS18B20 probes, TMP1075, J9 Qwiic port,
dry-contact inputs, status LED, and the network side of the firmware.
In the firmware that is most of granite-core: dispatcher, command
vocabulary, MQTT, HTTP API and auth, config with commit-confirm, rules,
Modbus server, OTA, store, identity, granite-sim, web UI.

Rev C stays in granite_controller with its own firmware for demos.
This repo's firmware targets rev D only.

---

## Decision

1. **Rev C code goes.** In this repo: hw/expanders, relays, sense and
   the press engine of actuator.rs are removed; NodeSwitches,
   NodeSense and DryInputs-on-an-expander leave hal.rs. Everything
   else is kept and extended. No rev C / rev D feature flags.

2. **A node is a slot on a plane.**
   - Node id = 8 x (plane address - 0x30) + slot + 1, so 1-64,
     fixed by the plane's address straps and the slot position. A
     node keeps its id across reboots, re-discovery and a plane swap
     with the same straps.
   - Config gives each node an optional name and per-node settings
     (boot policy, LED sense enabled, soft-off timeout). Nodes that
     are not present are not listed in state payloads; their config
     is kept.
   - Planes and units are objects of their own (plane 0-7, unit 0-3
     per plane) with state and telemetry.

3. **Node state has two axes.**
   - Power (the rail): `off` (OFF set), `on` (PGD), `starting`,
     `fault` (OV latched, power limit trip, retry limit, unit hot-swap
     latched off), `absent` (no module or no unit), `unknown` (plane
     not answering).
   - Host (the motherboard, from the LED): `off`, `on`, `sleep`
     (blinking), `unknown` (rail off, LED sense disabled, or no data).
   - Plus telemetry: V, I, P, P min / max, temperature, energy, module
     class.

4. **Commands.** The rev C verbs keep their names and meaning, so MQTT
   and API clients keep working; each becomes one plane command (the
   plane runs the sequence, ADR 0002 decision 6):

   | Verb | Meaning | Plane command |
   |---|---|---|
   | on | rail on if needed, then PWR if the host stays dark | node on (new, see below) |
   | off | soft off: short PWR, wait for the host to go dark, then force_off | node off (new), escalate 3 |
   | force_off | hold PWR 5 s; rail cut if the host is still on | node off, timeout 0, escalate 3 |
   | reset | RST press | press RST |
   | cycle | rail off, wait, rail on, PWR if dark | power-cycle (0x47) |
   | press | raw PWR / RST press | press PWR / RST |
   | on_all | staggered `on` over a set of nodes | stagger on (0x4B) + node on |

   New verbs: `power_on` / `power_off` (rail only, no buttons),
   `clear_latch`, `unit_reset` (unit hot-swap), `plane_reboot`.
   `force_off` keeps its rev C meaning (hold PWR 5 s) and adds a rail
   cut when the host LED is still on after the hold, so it always
   ends with the node off (user 2026-10-29). `off` escalates to the
   same after t_soft_off. A node turned off by the hold keeps its
   standby power; only a node that ignored the button loses it. The
   escalation is a per-node setting (none / hold / hold then cut),
   default hold then cut.

   Two plane commands are added to ADR 0003 (still Proposed): 0x50
   node on [u8 slot, u16 LED timeout s] and 0x51 node off [u8 slot,
   u16 timeout s, u8 escalate].

   The per-node settings travel with the verb as a `NodeOptions` (LED
   timeout, soft-off window, escalation), so the dispatcher is the only
   place that reads a node's configuration. `args.no_escalate` of the
   rev C vocabulary maps to escalation `none` and `args.hard` to "do
   not wait for the host LED"; `args.force` is accepted and carried,
   but the plane has the last word on presence.

5. **Plane client** in granite-core (`plane` module), over one new
   HAL trait:

   ```
   trait PlaneLink {
       fn write_read(&mut self, addr: u8, req: &[u8], resp: &mut [u8]) -> Result<usize, LinkError>;
       fn write(&mut self, addr: u8, frame: &[u8]) -> Result<(), LinkError>;
       fn int_asserted(&mut self) -> bool;
   }
   ```

   The client does, per ADR 0003: discovery (scan 0x30-0x37 at boot
   and every 10 s for absent addresses, INFO on find), the poll
   schedule (STATUS + SUMMARY 1 Hz, SLOT / UNIT every 10 s or on
   change), EXP_INT handling (STATUS of every plane, EVENTS where
   pending, EV_ACK), command seq and retries (at most 4 in flight per
   plane), config push (compare the config hash, write on mismatch),
   plane firmware updates (decision 8). It owns the plane models and
   turns plane events into controller events. It is synchronous and
   host-testable like the rest of granite-core.

6. **Commands complete asynchronously.** dispatch() checks the target
   and queues a plane job; the reply says "accepted". The MQTT ack and
   the HTTP job result are published when the plane reports the
   result (ADR 0003 decision 4), as rev C already does for presses.
   Commands to an absent node or plane fail at once.

7. **Network surfaces.**
   - MQTT: same topics. `state` carries `planes[]` (address, type,
     firmware, bus V, units[] with input V / I / P and hot-swap
     state) and `nodes[]` (id, name, plane, slot, power, host,
     telemetry). Published every 10 s and on change; telemetry
     values rounded so "on change" does not fire on noise. New event
     kinds for the plane events.
   - Modbus: map version 2 (map version register stays at its
     address): a fixed register block per node id 1-64 (power, host,
     V, I, P, temperature, energy as 2 x u32, faults), per plane a
     block, command words per node. Absent nodes read 0 with an
     absent bit. modbus_map.rs stays the single source; the docs page
     is generated from it.
   - HTTP API: same routes; `nodes/{target}/{action}` takes the new
     verbs; new read routes for planes and units. Web UI: nodes
     grouped by plane and unit.
   - Rules: new sources node power / host state, node V / I / P /
     temperature, plane bus V, unit input I / P; new actions are the
     new verbs. Evaluation stays at 1 s on the Observed snapshot.

8. **Plane firmware updates come from the controller.**
   - The controller's signed OTA bundle carries the plane image
     (signed with the same fleet key, ADR 0002 decision 9); an API
     upload of a plane image alone is also accepted.
   - Policy (config): `manual` (default) or `auto` (after the
     controller itself is confirmed, update every plane whose version
     differs, one at a time, never while a node sequence runs on it).
   - The controller confirms a plane image after reading INFO and
     seeing the units it saw before.

9. **Hardware the firmware assumes** (rev D board, own spec to follow):
   - Plane link on the HP I2C peripheral (hardware master, repeated
     start, clock stretch timeout set) at 100 kHz, 4.7k pull-ups and
     3V3 on the controller side, 5-pin JST PH to the planes' J2
     (3V3, GND, SCL, SDA, INT), EXP_INT on GPIO0.
   - TMP1075 and J9 Qwiic on the LP I2C pins.
   - Four dry-contact inputs straight on MCU GPIOs (U15 is gone),
     non-strapping pins.
   - 12 V in from up to two planes (J4 CTRL_12V), ideal-diode OR into
     the existing buck; VIN sense rescaled to 12 V.
   - Dropped: the 8 node connectors, 16 photoMOS, U14 / U15, the 19 V
     input and its divider. J8 stays only if an add-on need is named.

---

## Architecture Overview

### Component Breakdown

1. **granite-core** (`firmware/granite-core`)
   - `plane`: client, models (Plane, Unit, Node), discovery, poll
     schedule, command jobs, events, config push, update orchestration.
     Uses granite-proto.
   - `msg`, `dispatch`, `rules`, `observed`, `modbus_map`, `api`,
     `config`: extended for the node model above.
   - `hal`: PlaneLink added; NodeSwitches, NodeSense out.
2. **granite-fw** (`firmware/granite-fw`, ESP32-C6)
   - `hw`: PlaneLink on HP I2C + GPIO0, dry inputs on GPIO, probes,
     TMP1075, VIN, LEDs. platform/ unchanged except the OTA bundle
     carrying the plane image.
3. **granite-sim** (`firmware/granite-sim`)
   - PlaneLink over simulated planes running plane-logic (ADR 0002),
     scenario TOML: planes, units, modules, loads, faults.
4. **web UI** (`firmware/web`): plane / unit / node views.

### Data Flow / Interaction

```
 MQTT / HTTP / Modbus                 rules (1 s)
        \         |        /             |
         v        v       v              v
            dispatch(Command) ----> plane jobs (seq, retry)
                  |                        |
              Observed  <---- plane client: models, poll, events
                  ^                        |
                  |                 PlaneLink (HP I2C, EXP_INT)
               probes, TMP1075,            |
               dry inputs, VIN       planes 0x30-0x37
```

---

## Alternatives Considered

### Keep rev C's node model, add planes beside it
- **The idea**: nodes stay "buttons + LED" 1-8; the plane is a power
  board object with its own channels.
- **Optimizes for**: no change for existing clients.
- **Sharpest tradeoff**: a node's rail and its buttons are on the same
  slot but in two objects; every rule and dashboard joins them by
  hand, and 8 nodes per controller stays the limit.
- **Bets on**: one plane per controller. The user decided several.

### Node ids from config only
- **The idea**: the operator maps (plane, slot) to node ids freely.
- **Optimizes for**: matching an existing numbering in the frame.
- **Sharpest tradeoff**: a wrong or stale map controls the wrong node;
  a fresh controller has no names until configured.
- **Bets on**: careful operators. Names cover the human side; the
  id stays derived from the hardware.

### Controller keeps the sequencing (plane as thin I/O)
- Rejected by the user (ADR 0002 context).

---

## Consequences

### Positive
- A node shows its real power state, telemetry and faults; force_off
  ends with the node off even when the board ignores its button.
- Up to 64 nodes per controller with no code change.
- Clients written for rev C verbs keep working.

### Negative
- MQTT state and Modbus change shape: map version 2, HA templates to
  update.
- force_off can take ~5 s longer than a plain cut when the board
  ignores the hold.
- The controller can no longer act on a node without a working plane.

### Risks
- **ESP32-C6 RAM with 64 nodes**: node models, Observed and the state
  payload grow ~8x. Estimate before building: ~200 bytes per node
  model, ~15 KB; the JSON state payload ~20 KB at 64 nodes, built
  streaming, not in one buffer.
- **I2C master behaviour of ESP-IDF with clock stretching through
  the ISO1640**: bench test in the plane-link spike (ADR 0002).

---

## What an Expert Would Ask

**Q: The controller reboots. Does any node change state?**
A: No. The planes hold their state and sequences on their own. The
controller rediscovers the planes, reads their state, and applies
boot policy only to nodes whose policy says so and only on a plane
boot (boot count changed), never on its own boot.

**Q: A plane reboots (watchdog) while the controller watches. What
then?**
A: Its boot count changes; the controller re-reads INFO, re-pushes
config if the hash differs, raises an event, and shows the nodes the
plane reports. A watchdog reset changes nothing: the plane restores
OFF slots from its backup registers (ADR 0002 decision 3). After a
plane power loss the plane flags "intended state lost"; the controller
then re-applies OFF to every node it last commanded OFF (kept in NVS,
so this also works after both rebooted).

The re-apply is driven by the slot, not by the boot count: a core power
loss takes the boot counter with it (the plane comes back at 1) and the
flag stays up until a reset that keeps VDD, so neither the count nor the
flag's edge can say whether a loss has been answered. While the flag is
up, every node the controller last commanded rail-off whose slot reports
itself enabled again is commanded off, at most once per slot report.
That is self-limiting, and a verb that wants the rail up takes the node
out of the set, so an operator who powers a node on is never fought
over.

**Q: Two planes in one frame, and the frame numbers its nodes 1-16
left to right. Plane 0 is the right-hand one.**
A: Ids follow the straps, so strap the planes in frame order. Names
carry the labels a site uses. Free remapping was rejected above.

**Q: An MQTT client sends `on` to a node whose unit hot-swap is
latched off.**
A: The plane refuses (not present / fault result), the ack says so
with the reason. `on` does not reset a unit latch; that is
`unit_reset`, a deliberate act. Since 2026-10-30 the latch is the
plane's own (ADR 0002 decision 7a: the unit stage is an auto-retry
LM5069-2), which changes nothing here - the unit is still held off
until someone sends `unit_reset`.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **Node id from straps and slot (1-64).** Alternative: config map.
  Cost to change later: every client and the Modbus map.
- **force_off = 5 s hold, then a rail cut if the LED stays on**, and
  `off` escalates to it. Decided (user 2026-10-29). Alternatives: a
  plain rail cut, or the hold alone. Cost to change: the escalate
  value.
- **Plane updates manual by default.** Alternative: auto. Cost: a
  config default.

### Known unknowns and how the plan absorbs them
- RAM at 64 nodes: default full models; pivot to SUMMARY-only models
  for nodes not looked at if the heap gets tight.
- Modbus clients in the field on map version 1: none known (rev C
  only at the bench); default break, bump the version.

### The mechanical work
- granite-core: plane module, msg / Observed / rules / modbus_map /
  api extended, rev C paths removed; host tests against golden frames
  and the simulated plane.
- granite-sim: fake planes on plane-logic.
- granite-fw: rev D hw layer once the board exists; until then the
  bench runs on rev C hardware with a plane on the J8 pins (GPIO6 / 7,
  software I2C) as a stopgap.
- web UI and docs (home-assistant.md, modbus_map.md) to the new model.

Review asks (answered 2026-10-29 except 2):
1. Node id derived from plane straps and slot (1-64), names in config:
   yes.
2. `force_off`: 5 s hold, then a rail cut if the host is still on;
   `off` escalates to the same: decided.
3. Plane firmware updates manual by default, auto as an option: yes.
4. Rev D hardware: plane link on the HP I2C, dry inputs on GPIOs, two
   12 V inputs OR-ed: yes, into the rev D hardware spec.

---

## Open Questions

**Architecture-changers**
- [x] Review asks 1 and 4 (user 2026-10-29).
- [x] Review ask 2: force_off = hold then cut, off escalates to it
  (user 2026-10-29).

**Behavior definers**
- [x] OFF slots go back OFF after any reset or glitch (user
  2026-10-29): the plane from its backup registers / RAM, the
  controller after a plane power loss ("intended state lost").
- [x] Review ask 3 (user 2026-10-29).
- [ ] HA: MQTT discovery now that the node set is dynamic, or
  templates as today.

---

## As built (2026-10-29)

Implemented in granite-core, granite-sim and granite-fw. The decisions
above are what was built; this section is the detail they left open and
the few places the implementation chose for them. Everything here is a
local choice, reversible without touching the wire format (granite-proto
is untouched).

### Where things live
- **Config.** The per-node settings are a list keyed by node id in the
  existing `nodes` section, not an array of 64: a node that is not
  present keeps its entry (decision 2) and a config with no entries is
  the default for every node. The plane update policy of decision 8 is
  `nodes.plane_update` (`manual` / `auto`); no new section was added.
  `nodes.vin_min_mv` / `vin_max_mv` default to 10 V / 14 V, because rev D
  is fed 12 V from the planes (decision 9).
- **Targets.** `Target` gained `plane` and `unit`, spelled `planeN` and
  `planeN.unitU` on the wire and in a URL; a node id is still a bare
  number and `all` is still `all`.
- **Acks.** `Reply` gained `job`, the plane job id, and the `accepted`
  event carries it too. `granite_core::jobs` holds the pending jobs, so
  the MQTT ack, the HTTP `?wait=1` result and the per-node "last job" in
  the state payload all come from one place in both the firmware and the
  simulator.
- **Events.** The plane's own events and the client's findings are one
  new event kind, `"kind": "plane"`, keyed by `what` (`plane_found`,
  `slot_on`, `ov_latched`, `intended_off_reapplied`, ...). `node_state`
  carries `power` and `host` instead of rev C's `state` and `led`;
  `soft_off_timeout` is gone, because the plane runs the wait and reports
  the sequence itself.
- **Telemetry rounding.** The state payload carries 0.1 V, 0.1 A, whole
  watts, whole degrees and whole watt-hours. Energy had to be rounded to
  something coarse (Wh) or an on-change publisher would fire every second
  on a node that draws power.

### Modbus map version 2
Sparse, and everything outside it answers exception 2 - the rev C node
registers included, so a version 1 client fails loudly instead of
reading a plausible wrong node. The dry inputs, the link and MQTT bits
and the whole first input-register block (probes, board temperature,
VIN, uptime, firmware, faults, **map version at 14**) keep their
addresses.

| table | block |
|---|---|
| discrete inputs | `100 + id - 1` node rail, `200 + id - 1` node host |
| input registers | `100 + 12 x (id - 1)` node block, `900 + 16 x plane` plane block, `15` planes present, `16` nodes present |
| holding registers | `8` on_all, `9` last result, `100 + id - 1` node command word, `200 + plane` plane command word |
| coils | `100 + id - 1` node rail, read and write |

An absent node reads 0 everywhere with bit 0 of its faults word set. The
per-plane command word (`plane_reboot`, `unit_reset` of unit 0-3) is an
addition: decision 7 names only per-node words, and Modbus was otherwise
the one surface that could not reach a unit latch. `last_result` reports
`absent` where rev C reported `shutdown_pending`, and the fault bitmap's
first two bits are now "a plane stopped answering" and "a node is in
fault" (bit 7 is "no plane at all") where rev C had "expander" and
"sense". `RegisterImage` borrows the snapshot and answers one address at
a time rather than rendering a thousand registers onto a connection
thread's stack.

### HTTP
Same routes, plus `GET /api/v1/planes`, `/api/v1/planes/{n}` and
`/api/v1/planes/{n}/units`. `POST /api/v1/nodes/{target}/{action}` takes
the twelve plane verbs and nothing else, with the new target spellings.

### Boot policy
`on` and `off` are the verbs of decision 4, so a boot policy of `off` on
a node whose host is already dark leaves the rail up: `off` means "shut
the host down", and `power_off` is the verb that means the rail. The
policies run three seconds after a **plane** boot (see the fixes below;
the first implementation ran them three seconds after the dispatcher
started, which is not what this ADR asks for).

### granite-fw: two marked placeholders
Rev D hardware does not exist, so the board firmware compiles against
the new core with:
- `hw::plane_link::StubPlaneLink`: every address answers NAK, which is
  how the client reads absence. No I2C transaction is performed. The real
  driver (HP I2C, 100 kHz, EXP_INT on GPIO0) comes with the board, and
  the ISO1640 clock-stretch question is the bench test this ADR already
  lists as a risk.
- `hw::dry::DryGpio::unwired`: decision 9 says four dry inputs on
  non-strapping GPIOs but leaves the numbers to the hardware spec, so the
  reads answer "not present" (the snapshot says "never read") until
  `DryGpio::wired` gets a pin map.

Also deferred to the board: the TMP1075 stays on the HP I2C bus, where
rev C has it, rather than moving to the LP pins as decision 9 says. The
OTA probation ladder lost its "expanders initialised" step and did not
gain a plane step: a controller whose planes are unplugged still has to
be able to take a firmware update.

### Fixes after the first pass (2026-10-30)
Six places where the implementation did not do what this ADR says. No
decision changed and `granite-proto` is still untouched.

1. **Target `all`.** Only `on_all` accepted it, so the shipped rules 1
   and 2 (`force_off` on `all`) switched nothing off. Every node verb
   takes `all` now and becomes one plane job per present node, each with
   that node's settings; `granite_core::jobs` keeps one entry per
   command, so the group is acked once with the worst outcome of its
   jobs.
2. **`on_all`.** It named every present slot to the plane's stagger
   (0x4B), which switches a slot off before releasing it, so a running
   node lost its rail. Decision 4 calls the verb a staggered `on`: the
   set is now the present nodes whose host is dark, and only those whose
   rail is down are named to 0x4B. 0x4B itself is unchanged - which
   slots may be cut is the caller's decision (ADR 0002 decision 6).
3. **Default rule 3** tripped below 15 V, the figure for rev C's 19 V
   supply, which a 12 V rev D controller is always below. It watches
   `config::DEFAULT_VIN_MIN_MV` (10 V), the same constant
   `nodes.vin_min_mv` defaults to.
4. **Modbus fault bit 0** (`PLANE_LOST`) was never set. The client
   tracks the addresses it had found and lost, the snapshot carries them
   (`Observed::planes_lost`) and `modbus_map::plane_faults` decides bits
   0, 1 and 7 in one host-tested place.
5. **Acks.** The MQTT completion ack had no `job`; it (and the HTTP
   `?wait=1` outcome) carries the job id of the acceptance now. The ack
   and job deadlines are derived from the command and the node's
   settings (`dispatch::ack_timeout_ms`, `plane::job_timeout_ms`)
   instead of fixed 300 s / 600 s constants, which cut off any node
   whose `t_soft_off_s` was more than a few minutes.
6. **Boot policy** ran once per controller start. It runs per plane boot
   now (`granite_core::boot`): the boot count moved, or the plane came
   up reporting a lost intended state with an uptime under a minute -
   the site power loss that took the controller with it. A controller
   restart on its own commands nothing, which is what this ADR's expert
   answer asks for.

### The unit latch moved into the plane firmware (2026-10-30)
The unit hot-swap is an LM5069-2 now and the latch-off is the plane's
(ADR 0005 decision 3, ADR 0002 decision 7a). Nothing on the controller
side changed: the same `unit_reset` verb, the same `UnitHotSwapReset`
CTRL record, the same `latched_off` flag and
`unit_hot_swap_latched_off` event, and `granite-proto` is untouched.
Two behaviours an operator sees are different:
- `unit_reset` no longer pulses U_OFF for 100 ms. It releases the
  plane's hold, so a unit that is *not* latched keeps both its nodes
  running through the command instead of losing the rail for a tenth of
  a second.
- Its result arrives about a second later and says what came of it:
  `done` when the unit carries power again, `failed` when the fault is
  still there and the plane latched it again. The default job timeout
  covers that easily.

### Still open
- The web UI (`firmware/web`) is unchanged and still speaks the rev C
  state payload; `docs/home-assistant.md` and `docs/modbus_map.md` are
  stale (the second is generated: `granite-sim modbus-map`).
- MQTT discovery for Home Assistant now that the node set is dynamic
  (the open question below).
