# Home Assistant example

The controller publishes retained JSON on MQTT (ADR 0001 component 7),
so Home Assistant's MQTT integration can show and control a frame with
plain YAML, no custom component. Replace `default` (site) and
`granite-37adc7` (device id, from the MAC) with yours; the topic base is
`<topic_root>/<site>/<device>` as configured on the controller's MQTT
page.

Payload shapes the templates below read (firmware/granite-core/src/msg.rs):

- `.../status` (retained): `{"v":1,"online":true,"fw":"0.1.0","ip":"..","uptime_s":N,"boot_reason":".."}`;
  the will message sets `"online":false`.
- `.../state` (retained, on change and every 60 s): `nodes[0..7].state`
  is `unknown|off|on|busy`, `nodes[i].led`, `probes[i].temp_c` (null
  when the probe is missing), `board_temp_c`, `vin_v`, `dry_in[0..3]`
  (true = contact closed), `link_up`, `mqtt_connected`, `uptime_s`.
- `.../cmd`: `{"v":1,"id":"<any>","action":"on|off|force_off|reset|cycle|on_all","target":1..8|"all"}`;
  the reply lands on `.../ack/<id>` when the action completes.

## configuration.yaml

```yaml
mqtt:
  switch:
    - name: "Frame 1 node 1"
      unique_id: granite_37adc7_node_1
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ 'ON' if value_json.nodes[0].state in ['on', 'busy'] else 'OFF' }}"
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_on: '{"v":1,"id":"ha-1-on","action":"on","target":1}'
      payload_off: '{"v":1,"id":"ha-1-off","action":"off","target":1}'
      availability_topic: "granite/default/granite-37adc7/status"
      availability_template: "{{ 'online' if value_json.online else 'offline' }}"
      optimistic: false
      qos: 1
    # repeat for nodes 2..8: nodes[1] .. nodes[7], target 2..8

  button:
    - name: "Frame 1 node 1 reset"
      unique_id: granite_37adc7_node_1_reset
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_press: '{"v":1,"id":"ha-1-reset","action":"reset","target":1}'
      availability_topic: "granite/default/granite-37adc7/status"
      availability_template: "{{ 'online' if value_json.online else 'offline' }}"
    - name: "Frame 1 node 1 force off"
      unique_id: granite_37adc7_node_1_force_off
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_press: '{"v":1,"id":"ha-1-force","action":"force_off","target":1}'
    - name: "Frame 1 all nodes on (staggered)"
      unique_id: granite_37adc7_on_all
      command_topic: "granite/default/granite-37adc7/cmd"
      payload_press: '{"v":1,"id":"ha-on-all","action":"on_all","target":"all"}'

  sensor:
    - name: "Frame 1 inlet"
      unique_id: granite_37adc7_probe_0
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ value_json.probes[0].temp_c }}"
      unit_of_measurement: "\u00b0C"
      device_class: temperature
      state_class: measurement
    - name: "Frame 1 controller board"
      unique_id: granite_37adc7_board_temp
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ value_json.board_temp_c }}"
      unit_of_measurement: "\u00b0C"
      device_class: temperature
      state_class: measurement
    - name: "Frame 1 bus voltage"
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
    - name: "Frame 1 leak float"
      unique_id: granite_37adc7_dry_1
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ 'ON' if value_json.dry_in and value_json.dry_in[0] else 'OFF' }}"
      device_class: moisture
    - name: "Frame 1 node 1 power LED"
      unique_id: granite_37adc7_node_1_led
      state_topic: "granite/default/granite-37adc7/state"
      value_template: "{{ 'ON' if value_json.nodes[0].led else 'OFF' }}"
      device_class: power
      entity_category: diagnostic
```

Notes:

- `id` in a command is what the ack is keyed on. HA payloads are static,
  so acks of the same button share an id; that only matters if you
  correlate acks in an automation.
- An automation can send any command with `mqtt.publish`, for example a
  power cycle: `{"v":1,"id":"ha-cycle-3","action":"cycle","target":3}`.
- `.../event` and `.../log` are JSON lines meant for a log collector,
  not for entities; `mosquitto_sub -v -t 'granite/#'` shows everything.
- The controller's rules (over-temperature, leak) run on the board and
  do not depend on HA; HA sees them as `rule_fired` events and as the
  node state changing.
- MQTT discovery (the controller announcing its entities under
  `homeassistant/...`) is not implemented; the wfi028t controller has it
  (fw/src/mqtt/entity.rs) and the same pattern would fit here if the
  YAML above becomes a nuisance across many frames.
