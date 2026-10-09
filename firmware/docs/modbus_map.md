# Granite controller Modbus map

Map version: 1

Generated from `granite-core/src/modbus_map.rs` (`render_markdown`). Do not edit by hand. Unit id and port are configuration (`sec.modbus`); the server is off by default and requires an allow-list.

## Discrete inputs

| Address | Name | Meaning | Scale | Access |
|---|---|---|---|---|
| 0 | `node1_led` | Node 1 power LED, 1 = on | 0/1 | r |
| 1 | `node2_led` | Node 2 power LED, 1 = on | 0/1 | r |
| 2 | `node3_led` | Node 3 power LED, 1 = on | 0/1 | r |
| 3 | `node4_led` | Node 4 power LED, 1 = on | 0/1 | r |
| 4 | `node5_led` | Node 5 power LED, 1 = on | 0/1 | r |
| 5 | `node6_led` | Node 6 power LED, 1 = on | 0/1 | r |
| 6 | `node7_led` | Node 7 power LED, 1 = on | 0/1 | r |
| 7 | `node8_led` | Node 8 power LED, 1 = on | 0/1 | r |
| 8 | `dry_in1` | Dry contact 1, 1 = closed | 0/1 | r |
| 9 | `dry_in2` | Dry contact 2, 1 = closed | 0/1 | r |
| 10 | `dry_in3` | Dry contact 3, 1 = closed | 0/1 | r |
| 11 | `dry_in4` | Dry contact 4, 1 = closed | 0/1 | r |
| 12 | `link_up` | Ethernet link, 1 = up | 0/1 | r |
| 13 | `mqtt_connected` | MQTT session, 1 = connected | 0/1 | r |

## Input registers

| Address | Name | Meaning | Scale | Access |
|---|---|---|---|---|
| 0 | `probe1_temp` | Probe slot 1 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 1 | `probe2_temp` | Probe slot 2 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 2 | `probe3_temp` | Probe slot 3 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 3 | `probe4_temp` | Probe slot 4 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 4 | `probe5_temp` | Probe slot 5 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 5 | `probe6_temp` | Probe slot 6 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 6 | `probe7_temp` | Probe slot 7 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 7 | `probe8_temp` | Probe slot 8 temperature | 0.01 C, signed, 0x8000 = missing | r |
| 8 | `board_temp` | TMP1075 board temperature | 0.01 C, signed, 0x8000 = missing | r |
| 9 | `vin` | Bus voltage | mV, 0 = never read | r |
| 10 | `uptime_hi` | Seconds since boot, high word | s | r |
| 11 | `uptime_lo` | Seconds since boot, low word | s | r |
| 12 | `fw_version` | Firmware version | major << 8 | minor | r |
| 13 | `fault_flags` | Fault bitmap, see fault_bits | bitmap | r |
| 14 | `map_version` | Version of this register map | integer | r |

## Holding registers

| Address | Name | Meaning | Scale | Access |
|---|---|---|---|---|
| 0 | `node1_cmd` | Node 1: write a command word, read the node state | w: 1 on, 2 off, 3 force_off, 4 reset, 5 cycle; r: 0 unknown, 1 off, 2 on, 3 busy | rw |
| 1 | `node2_cmd` | Node 2: write a command word, read the node state | see node1_cmd | rw |
| 2 | `node3_cmd` | Node 3: write a command word, read the node state | see node1_cmd | rw |
| 3 | `node4_cmd` | Node 4: write a command word, read the node state | see node1_cmd | rw |
| 4 | `node5_cmd` | Node 5: write a command word, read the node state | see node1_cmd | rw |
| 5 | `node6_cmd` | Node 6: write a command word, read the node state | see node1_cmd | rw |
| 6 | `node7_cmd` | Node 7: write a command word, read the node state | see node1_cmd | rw |
| 7 | `node8_cmd` | Node 8: write a command word, read the node state | see node1_cmd | rw |
| 8 | `on_all` | Write 1 to run the staggered on_all | w: 1 trigger; r: 0 | rw |
| 9 | `last_result` | Result of the last command word | 0 idle, 1 ok, 2 accepted, 3 refused, 4 failed, 5 shutdown_pending | rw |

## Coils

| Address | Name | Meaning | Scale | Access |
|---|---|---|---|---|
| 0 | `node1_power` | Node 1: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 1 | `node2_power` | Node 2: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 2 | `node3_power` | Node 3: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 3 | `node4_power` | Node 4: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 4 | `node5_power` | Node 5: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 5 | `node6_power` | Node 6: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 6 | `node7_power` | Node 7: read the state, write 1 = on, 0 = off | 0/1 | rw |
| 7 | `node8_power` | Node 8: read the state, write 1 = on, 0 = off | 0/1 | rw |

## Fault flags (input register 13)

| Bit | Meaning |
|---|---|
| 0 | expander readback mismatch or press deadline |
| 1 | power LEDs unreadable |
| 2 | a configured probe did not answer |
| 3 | bus voltage outside its window |
| 4 | an OTA image is pending validation |
| 5 | a config section fell back to defaults |
| 6 | broker configured but not connected |

Writes to a node command word or a coil become the same `Command` the MQTT and HTTP paths use, so the refusal rules are identical: an action on a node whose state is `unknown` is refused, because Modbus cannot carry `force`.
