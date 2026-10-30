# Granite controller Modbus map

Map version: 2

Generated from `granite-core/src/modbus_map.rs` (`render_markdown`). Do not edit by hand. Unit id and port are configuration (`sec.modbus`); the server is off by default and requires an allow-list.

A node is a slot on a plane: id = 8 x (plane address - 0x30) + slot + 1, so 1-64 (ADR 0004 decision 2). The map is sparse; every address that is not listed answers exception 2, the rev C node registers included. The map version stays in input register 14.

## Discrete inputs

| Address | Name | Meaning | Scale | Access |
|---|---|---|---|---|
| 8 | `dry_in1` | Dry contact 1, 1 = closed | 0/1 | r |
| 9 | `dry_in2` | Dry contact 2, 1 = closed | 0/1 | r |
| 10 | `dry_in3` | Dry contact 3, 1 = closed | 0/1 | r |
| 11 | `dry_in4` | Dry contact 4, 1 = closed | 0/1 | r |
| 12 | `link_up` | Ethernet link, 1 = up | 0/1 | r |
| 13 | `mqtt_connected` | MQTT session, 1 = connected | 0/1 | r |

One address per node 1-64 at 100..163: Node rail, 1 = on (0/1, r).

One address per node 1-64 at 200..263: Node host LED, 1 = on (0/1, r).

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
| 9 | `vin` | Controller input voltage | mV, 0 = never read | r |
| 10 | `uptime_hi` | Seconds since boot, high word | s | r |
| 11 | `uptime_lo` | Seconds since boot, low word | s | r |
| 12 | `fw_version` | Firmware version | major << 8 | minor | r |
| 13 | `fault_flags` | Fault bitmap, see fault_bits | bitmap | r |
| 14 | `map_version` | Version of this register map | integer | r |
| 15 | `planes_present` | Bit n = plane n answers | bitmap | r |
| 16 | `nodes_present` | How many nodes are present | integer | r |

12 registers per node 1-64, first block at 100, address = 100 + 12 x (node - 1) + offset:

| Offset | Name | Meaning | Scale |
|---|---|---|---|
| +0 | `power` | Rail state | 0 unknown, 1 off, 2 on, 3 starting, 4 fault, 5 absent |
| +1 | `host` | Host state | 0 unknown, 1 off, 2 on, 3 sleep |
| +2 | `volts` | Slot voltage | 10 mV |
| +3 | `amps` | Slot current | mA |
| +4 | `watts` | Slot power | 0.1 W |
| +5 | `temp` | Slot temperature | 0.1 C, signed, 0x8000 = missing |
| +6 | `energy_hi` | Energy since the plane's first boot, high word | J |
| +7 | `energy_lo` | Energy since the plane's first boot, low word | J |
| +8 | `energy_boot_hi` | Boot count the energy belongs to, high word | integer |
| +9 | `energy_boot_lo` | Boot count the energy belongs to, low word | integer |
| +10 | `faults` | Fault bitmap, see node_fault_bits | bitmap |
| +11 | `place` | slot << 8 | module class | packed |

16 registers per plane 0-7, first block at 900, address = 900 + 16 x (plane - 0) + offset:

| Offset | Name | Meaning | Scale |
|---|---|---|---|
| +0 | `flags` | Plane flags, see plane_flag_bits | bitmap |
| +1 | `firmware` | Plane firmware version | major << 8 | minor |
| +2 | `bus_volts` | Core input voltage | 10 mV |
| +3 | `core_temp` | Core temperature | 0.1 C, signed |
| +4 | `units_present` | Bit n = unit n is on the chain | bitmap |
| +5 | `nodes_present` | Bit n = slot n holds a module | bitmap |
| +6 | `boot_count_hi` | Boot count, high word | integer |
| +7 | `boot_count_lo` | Boot count, low word | integer |
| +8 | `uptime_hi` | Plane uptime, high word | s |
| +9 | `uptime_lo` | Plane uptime, low word | s |
| +10 | `unit0_watts` | Unit 0 input power | 0.1 W |
| +11 | `unit1_watts` | Unit 1 input power | 0.1 W |
| +12 | `unit2_watts` | Unit 2 input power | 0.1 W |
| +13 | `unit3_watts` | Unit 3 input power | 0.1 W |
| +14 | `units_latched_off` | Bit n = unit n latched its hot-swap off | bitmap |
| +15 | `reserved` | Reserved, reads 0 | integer |

## Holding registers

| Address | Name | Meaning | Scale | Access |
|---|---|---|---|---|
| 8 | `on_all` | Write 1 to run the staggered on_all over every present node | w: 1 trigger; r: 0 | rw |
| 9 | `last_result` | Result of the last command word | 0 idle, 1 ok, 2 accepted, 3 refused, 4 failed, 5 absent | rw |

One address per node 1-64 at 100..163: Node command word; reads the rail state (w: 1 on, 2 off, 3 force_off, 4 reset, 5 cycle, 6 power_on, 7 power_off, 8 clear_latch; r: see the node block's power field, rw).

One address per plane 0-7 at 200..207: Plane command word (w: 1 plane_reboot, 2..5 unit_reset of unit 0..3; r: 0, rw).

## Coils

One address per node 1-64 at 100..163: Node rail: read the state, write 1 = on, 0 = off (0/1, rw).

## Fault flags (input register 13)

| Bit | Meaning |
|---|---|
| 0 | a plane stopped answering |
| 1 | a node is in fault |
| 2 | a configured probe did not answer |
| 3 | controller input voltage outside its window |
| 4 | an OTA image is pending validation |
| 5 | a config section fell back to defaults |
| 6 | broker configured but not connected |
| 7 | no plane answers |

## Node faults (node block offset 10)

| Bit | Meaning |
|---|---|
| 0 | no module, or no unit board |
| 1 | overvoltage latch set |
| 2 | power limit tripped |
| 3 | hot-swap retry limit reached |
| 4 | the unit board latched its hot-swap off |
| 5 | LED / rail cross-check |
| 6 | a sequence is running on the slot |

Writes to a node or plane command word, and to a node coil, become the same `Command` the MQTT and HTTP paths use, so the rules are identical: the verb is submitted as a plane job, the write itself succeeds, and `last_result` says `accepted`. A command for a node, unit or plane that is not there is refused at once and `last_result` says `absent`.
