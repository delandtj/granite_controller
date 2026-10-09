# granite-sim

The Granite controller, on the host. It runs the real `granite-core` -
node state machine, actuator queue, rule engine, dispatcher and the HTTP
route table - against a fake board described by a scenario file, and
serves the real setup page (`firmware/web/`) over HTTPS with a
self-signed certificate. The browser experience and the API contract are
therefore the same ones the firmware serves, without a board.

## Commands

```
granite-sim serve [--port 8443] [--bind 127.0.0.1] [--scenario file.toml]
                  [--no-tls] [--assets ../web]
granite-sim recover [--key ~/.config/granite/fleet-recovery.key] [--port N]
                    [--no-tls] <host>
granite-sim keygen [--key ~/.config/granite/fleet-recovery.key]
granite-sim modbus-map > ../docs/modbus_map.md
```

- `serve` prints the device id, MAC, certificate fingerprint and the
  recovery token, then listens. The certificate is generated per run, so
  the browser warns once; `--assets ../web` serves the page from disk so
  edits need no rebuild.
- `recover` is the fleet recovery tool from ADR 0001 component 13: it
  fetches `/id`, signs `device || nonce || "factory-reset"` with the
  fleet ECDSA P-256 private key and posts `/recover`. TLS is not trusted
  here (the device certificate is self-signed by design); the signature
  is the authentication.
- `keygen` writes the private key with mode 0600 and prints the public
  PEM for the Security page. It refuses to overwrite an existing key.
- `modbus-map` regenerates `firmware/docs/modbus_map.md` from
  `granite_core::modbus_map::render_markdown`.

## First setup with curl

```
B=https://127.0.0.1:8443
curl -sk $B/id
curl -sk -X POST -d '{"password":"correct horse battery"}' $B/api/v1/security/password
curl -sk -c jar -X POST -d '{"password":"correct horse battery"}' $B/api/v1/session
curl -sk -b jar $B/api/v1/status
curl -sk -b jar -X POST -d '{}' $B/api/v1/nodes/3/on
```

## Scenario file

Everything is optional; see `granite-sim/src/scenario.rs` for the full
set and the defaults (8 nodes off, one probe at 40 C, 19 V on the bus).

```toml
device_id = "granite-510000"

[hardware]
vin_mv = 19000
board_temp_c = 32.5
dry_in = [false, false, false, false]
vin_ramp_mv_per_min = 0

[[probes]]
rom = "28ff000000000001"
name = "inlet"
temp_c = 40.0
ramp_c_per_min = 0.0

[[nodes]]
node = 1
powered = false
respond = "normal"     # normal | never | slow
on_delay_ms = 1500
off_delay_ms = 4000
sense_broken = false
```

`respond = "never"` is the hung node: `on` times out, `off` escalates to
`force_off`, `press` still works. A PWR press held 4 s or longer cuts
power in the model, which is what makes the actuator's hardware deadline
observable. `sense_broken` is a broken LED wire: the node reads dark even
while it is powered, which is the failure mode ADR 0001 warns about and
the reason for the per-node `sense = "ignore"` setting. Marking every
node broken makes the whole U15 read fail instead, and then every node is
`unknown` and the state-dependent actions are refused unless the command
carries `"force": true`.

## What is not simulated

- Pull OTA (`{"url":..,"sha256":..}`) is logged and ignored; pushed
  images are checked for the ESP image magic byte only.
- MQTT: the broker summary is static. The topic contract is exercised
  through the `Command`/`Reply` JSON, which is the same code path.
- Modbus TCP: the map and the command translation are in the core and
  unit tested there; the simulator does not open port 502.
