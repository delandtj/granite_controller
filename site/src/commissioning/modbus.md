# Modbus TCP

For building-management systems and PLCs that speak Modbus rather than
MQTT. The server exposes node states and commands, probe temperatures,
bus voltage, dry contacts and a fault word in a fixed, versioned
register map. It is off by default.

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
mbpoll -m tcp -a 1 -t 3 -r 1 -c 15 granite-37adc7.local     # input registers 0..14
mbpoll -m tcp -a 1 -t 1 -r 1 -c 14 granite-37adc7.local     # discrete inputs 0..13
mbpoll -m tcp -a 1 -t 4 -r 1 -c 10 granite-37adc7.local     # holding registers 0..9
```

Input register 14 is the map version (currently 1); input register 13
is the fault word. Writing `1` to holding register 0 powers node 1 on,
`2` powers it off, `3` forces it off, `4` resets, `5` power-cycles.
Holding register 9 reports the result of the last write. Coils 0..7
are the simple on/off view of the same thing.

Because a Modbus write cannot carry a `force` flag, writes to a node in
the `unknown` state are refused (result 3). Fix the LED sense rather
than working around it.

The full map is in [Modbus register map](../reference/modbus-map.md).
