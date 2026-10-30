# Home Assistant

The controller publishes retained JSON over MQTT, so Home Assistant's
MQTT integration can show and control a frame with plain YAML, no
custom component. Replace the site (`default`) and the device id
(`granite-37adc7`) with yours; the topic base is
`<topic_root>/<site>/<device>` as configured on the controller's MQTT
page.

MQTT discovery (the controller announcing its entities under
`homeassistant/...`) is not implemented. Write the entities by hand, one
block per node you care about.

## What the templates read

- `.../status` (retained): `{"v":1,"online":true,"fw":"..","ip":"..","uptime_s":N,"boot_reason":"..","ota":".."}`;
  the last will sets `"online":false`.
- `.../state` (retained, on change and every `t_state_s`):
  `nodes[]` with one entry per **present** node, each carrying `node`
  (the id, 1..64), `name`, `plane`, `slot`, `unit`, `power`, `host`,
  `v`, `a`, `w`, `temp_c`, `energy_wh`, `faults` and `last_job`;
  `planes[]` with `plane`, `bus_v`, `temp_c` and `units[]`; plus
  `probes[]`, `board_temp_c`, `vin_v`, `dry_in[]`, `link_up`,
  `mqtt_connected`, `uptime_s`.
- `.../cmd`: `{"v":1,"id":"<any>","action":"<verb>","target":<node id>}`;
  the ack lands on `.../ack/<id>` when the plane has finished.

`nodes[]` is not indexed by node id: a node that is not present is left
out, so `nodes[0]` is whichever node comes first. Select by the `node`
field instead, as the templates below do, and let a node that is
missing read as unavailable rather than as some other node.

## configuration.yaml

The example covers node 9 (plane 1, slot 0) and plane 1. Copy the node
blocks per node and change the id in the `selectattr` and in the
`target`.

```yaml
mqtt:
  switch:
    - name: "Frame 1 node 9"
      unique_id: granite_37adc7_node_9
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set n = value_json.nodes | selectattr('node', 'equalto', 9) | list %}
        {{ 'ON' if n and n[0].host in ['on', 'sleep'] else 'OFF' }}
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_on: '{"v":1,"id":"ha-9-on","action":"on","target":9}'
      payload_off: '{"v":1,"id":"ha-9-off","action":"off","target":9}'
      availability_topic: "granite/default/granite-37adc7/status"
      availability_template: "{{ 'online' if value_json.online else 'offline' }}"
      optimistic: false
      qos: 1

  button:
    - name: "Frame 1 node 9 reset"
      unique_id: granite_37adc7_node_9_reset
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_press: '{"v":1,"id":"ha-9-reset","action":"reset","target":9}'
      availability_topic: "granite/default/granite-37adc7/status"
      availability_template: "{{ 'online' if value_json.online else 'offline' }}"
    - name: "Frame 1 node 9 force off"
      unique_id: granite_37adc7_node_9_force_off
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_press: '{"v":1,"id":"ha-9-force","action":"force_off","target":9}'
    - name: "Frame 1 node 9 power cycle"
      unique_id: granite_37adc7_node_9_cycle
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_press: '{"v":1,"id":"ha-9-cycle","action":"cycle","target":9}'

  sensor:
    - name: "Frame 1 node 9 power"
      unique_id: granite_37adc7_node_9_w
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set n = value_json.nodes | selectattr('node', 'equalto', 9) | list %}
        {{ n[0].w if n else none }}
      unit_of_measurement: "W"
      device_class: power
      state_class: measurement
    - name: "Frame 1 node 9 energy"
      unique_id: granite_37adc7_node_9_wh
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set n = value_json.nodes | selectattr('node', 'equalto', 9) | list %}
        {{ n[0].energy_wh if n else none }}
      unit_of_measurement: "Wh"
      device_class: energy
      state_class: total_increasing
    - name: "Frame 1 node 9 host"
      unique_id: granite_37adc7_node_9_host
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set n = value_json.nodes | selectattr('node', 'equalto', 9) | list %}
        {{ n[0].host if n else 'absent' }}
      entity_category: diagnostic
    - name: "Frame 1 node 9 slot temperature"
      unique_id: granite_37adc7_node_9_temp
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set n = value_json.nodes | selectattr('node', 'equalto', 9) | list %}
        {{ n[0].temp_c if n else none }}
      unit_of_measurement: "\u00b0C"
      device_class: temperature
      state_class: measurement
    - name: "Frame 1 plane 1 bus voltage"
      unique_id: granite_37adc7_plane_1_bus_v
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set p = value_json.planes | selectattr('plane', 'equalto', 1) | list %}
        {{ p[0].bus_v if p else none }}
      unit_of_measurement: "V"
      device_class: voltage
      state_class: measurement
    - name: "Frame 1 inlet"
      unique_id: granite_37adc7_probe_1
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ value_json.probes[0].temp_c if value_json.probes else none }}"
      unit_of_measurement: "\u00b0C"
      device_class: temperature
      state_class: measurement
    - name: "Frame 1 controller input voltage"
      unique_id: granite_37adc7_vin
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ value_json.vin_v }}"
      unit_of_measurement: "V"
      device_class: voltage
      state_class: measurement
    - name: "Frame 1 controller firmware"
      unique_id: granite_37adc7_fw
      state_topic: "granite/default/granite-37adc7/status"
      value_template: "{{ value_json.fw }}"
      entity_category: diagnostic

  binary_sensor:
    - name: "Frame 1 node 9 fault"
      unique_id: granite_37adc7_node_9_fault
      state_topic: "granite/default/granite-37adc7/state"
      value_template: >-
        {% set n = value_json.nodes | selectattr('node', 'equalto', 9) | list %}
        {{ 'ON' if n and n[0].power == 'fault' else 'OFF' }}
      device_class: problem
    - name: "Frame 1 leak float"
      unique_id: granite_37adc7_dry_1
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ 'ON' if value_json.dry_in and value_json.dry_in[0] else 'OFF' }}"
      device_class: moisture
```

## Notes

- The switch follows the **host** (`host`, from the power LED), because
  `off` shuts the host down and leaves the standby rail up when the
  host obeyed. A node whose LED is not wired then always reads OFF;
  for such a node template on `power` instead (`on` or `starting`),
  and use `power_on` / `power_off` as the payloads if what you want to
  switch is the rail.
- `id` in a command is what the ack is keyed on. HA payloads are static,
  so acks of the same button share an id; that only matters if you
  correlate acks in an automation.
- An automation can send any command with `mqtt.publish`, for example
  a rail-only power-off of node 12:
  `{"v":1,"id":"ha-12-poff","action":"power_off","target":12}`, or a
  unit hot-swap reset: `{"v":1,"id":"ha-u","action":"unit_reset","target":"plane1.unit0"}`.
- `.../event` and `.../log` are JSON lines meant for a log collector,
  not for entities; `mosquitto_sub -v -t 'granite/#'` shows everything.
- The controller's rules run on the board and do not depend on HA; HA
  sees them as `rule_fired` events and as the node state changing.
- Telemetry in `state` is rounded (0.1 V, 0.1 A, whole watts, degrees
  and watt-hours) so that an "on change" publish does not fire on
  noise. A value such as `1.7999999523` is that rounding seen through a
  32-bit float; round it in the template if it bothers you.
