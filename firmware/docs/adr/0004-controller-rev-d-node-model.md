# Controller rev D: nodes on planes

**Status**: Proposed (all review asks answered 2026-10-29)
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

**Q: Two planes in one frame, and the frame numbers its nodes 1-16
left to right. Plane 0 is the right-hand one.**
A: Ids follow the straps, so strap the planes in frame order. Names
carry the labels a site uses. Free remapping was rejected above.

**Q: An MQTT client sends `on` to a node whose unit hot-swap is
latched off.**
A: The plane refuses (not present / fault result), the ack says so
with the reason. `on` does not reset a unit latch; that is
`unit_reset`, a deliberate act.

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
