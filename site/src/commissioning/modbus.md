# Modbus TCP

For building-management systems and PLCs that speak Modbus rather than
MQTT. The server exposes the state, telemetry and commands of up to 64
nodes, a block per plane, probe temperatures, the controller's input
voltage, dry contacts and a fault word in a fixed, versioned register
map (version 2). It is off by default.

Modbus has no authentication, so the controller **refuses to listen
until an allow-list of client addresses exists**, and it answers only
those clients. Keep the Modbus master and the controllers on the same
management subnet.

## Enable

The settings live under Security, as the `sec.modbus` object, and are
staged like other security changes: apply, then confirm.

| Field | Default | Notes |
|---|---|---|
| Enabled | off | |
| Port | 502 | |
| Unit id | 1 | 1..247 |
| Max connections | 4 | 1..8 |
| Allowed peers | empty | Addresses or CIDRs, for example `10.20.0.5` or `10.20.0.0/24`. Required when enabled. |

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT \
  -d '{"enabled":true,"port":502,"unit_id":1,"max_conn":4,"allow":["10.20.0.5"]}' \
  https://granite-37adc7.local/api/v1/security/modbus-allowlist
curl -sk -H "Authorization: Bearer $T" -X POST https://granite-37adc7.local/api/v1/config/confirm
```

A configuration watcher turns the saved section into a running
listener within about 5 s, no reboot needed; confirm the staged `sec`
section so the setting survives the next reboot. Editing the
allow-list is picked up the same way.

## Check it

With any Modbus client, for example `mbpoll`:

```sh
mbpoll -m tcp -a 1 -t 3 -r 1 -c 17 granite-37adc7.local     # input registers 0..16
mbpoll -m tcp -a 1 -t 3 -r 101 -c 12 granite-37adc7.local   # node 1 block, 100..111
mbpoll -m tcp -a 1 -t 3 -r 901 -c 16 granite-37adc7.local   # plane 0 block, 900..915
mbpoll -m tcp -a 1 -t 1 -r 9 -c 6 granite-37adc7.local      # discrete inputs 8..13
mbpoll -m tcp -a 1 -t 4 -r 9 -c 2 granite-37adc7.local      # holding registers 8..9
```

Input register 14 is the map version: it must read **2**. A client
written for version 1 (the rev C controller with eight fixed nodes)
gets exception 2 on the old node addresses instead of plausible wrong
values; it has to be rewritten against the new map. Input register 13
is the fault word, 15 has a bit per plane that answers and 16 counts
the nodes that are present.

Each node has a block of 12 input registers at `100 + 12 x (node - 1)`:
rail state, host state, volts, amps, watts, temperature, energy, faults
and its place on the plane. A node that is not present reads 0
everywhere with bit 0 of its faults register set.

To command a node, write its command word, holding register
`100 + node - 1`: `1` on, `2` off, `3` force_off, `4` reset, `5` cycle,
`6` power_on, `7` power_off, `8` clear_latch. Holding register
`200 + plane` takes `1` for plane_reboot and `2`..`5` for unit_reset of
unit 0..3; holding register 8 takes `1` for on_all. Coils `100 + node - 1`
are the simple view: read the rail, write 1 for `on` and 0 for `off`.

A command over Modbus is the same command as over MQTT, with no
arguments, so it uses the node's own settings. The write itself
succeeds and holding register 9 (`last_result`) says what happened:
`2` accepted while the plane runs it, then `1` ok, `4` failed or `5`
absent once the plane reports. A command for a node, unit or plane
that is not there reads `5` at once, and anything else the controller
refuses before it reaches a plane reads `3`. A value the map does not know is exception 3, and
nothing is written.

The full map is in [Modbus register map](../reference/modbus-map.md).
