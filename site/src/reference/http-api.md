# HTTP API

HTTPS on port 443 with the device certificate. Plain HTTP on port 80
serves only `/id` and a redirect. All bodies are JSON unless noted;
request bodies are limited to 64 KiB except the firmware upload.

## Authentication

| Class | Routes | How |
|---|---|---|
| Public | `/id`, `/recover`, `/api/v1/session`, static assets | none |
| Password setup | `POST /api/v1/security/password` | public until a password exists, then authenticated |
| Authenticated | everything else under `/api/v1` | session cookie `granite_session` or `Authorization: Bearer <token>` |

- Before a password exists every authenticated route returns 403 and
  login returns 409.
- A missing or bad credential returns 401 with
  `WWW-Authenticate: Bearer realm="granite"`.
- Login lockout: after `sec.max_login_fails` (5) failures, 429 for
  `sec.lockout_s` (60 s).
- Session cookie: 32 random bytes, `HttpOnly; Secure; SameSite=Strict`,
  lifetime `sec.session_hours` (12 h), kept in RAM (a reboot logs
  everyone out).
- API tokens: 32 random bytes shown once, stored hashed. Preferred
  over the cookie when both are sent.

## Routes

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/id` | none | Identity: `device`, `mac`, `fw`, `cert_sha256`, `nonce`, `password_set`, `uptime_s`, `ota{running,state,pending_verify}`, `modbus_map_version`, `v` |
| POST | `/recover` | none | Factory reset with `{"token":..}` or `{"device","nonce","sig"}`; 1 attempt per minute |
| POST | `/api/v1/session` | none | Login `{"password":..}` -> `{ok, session_hours}` + cookie |
| DELETE | `/api/v1/session` | none | Logout |
| GET | `/api/v1/session` | none | `{ok, authenticated, password_set, locked_out, lockout_left_s}` |
| GET | `/api/v1/status` | yes | Status page data: identity, `boot_reason`, `uptime_s`, `free_heap`, `state`, `net`, `mqtt`, `ota`, `staged`, `modbus` |
| GET | `/api/v1/state` | yes | The state snapshot (same as the MQTT `state` payload) |
| GET | `/api/v1/nodes` | yes | `{ok, settings: <nodes section>, nodes: [...], verbs: [...]}`; `nodes` is the `nodes` array of the state payload, `verbs` the twelve plane verbs |
| POST | `/api/v1/nodes/<target>/<verb>` | yes | Submit a plane verb. `<target>` is a node id `1`..`64`, `all`, `planeN` or `planeN.unitU`; `<verb>` is one of `on`, `off`, `force_off`, `reset`, `cycle`, `press`, `on_all`, `power_on`, `power_off`, `clear_latch`, `unit_reset`, `plane_reboot`. The body is the `args` object, bare or as `{"args":{..}}` (optional). 200 with `accepted` and the `job` id, 400 when refused. Add `?wait=1` to hold the reply until the plane reports (30 s cap, see below). An unknown verb or a malformed target is a 404 |
| GET | `/api/v1/planes` | yes | `{ok, planes: [...], update_policy}`; `planes` is the `planes` array of the state payload |
| GET | `/api/v1/planes/<0-7>` | yes | `{ok, plane: {..}, nodes: [..]}`: the controller's full model of one plane and its present nodes, in raw plane units (see below); 404 if the plane does not answer |
| GET | `/api/v1/planes/<0-7>/units` | yes | `{ok, plane, units: [..]}`: the unit boards of one plane, raw units; 404 if the plane does not answer |
| POST | `/api/v1/cmd` | yes | A raw command, identical to the MQTT `cmd` payload |
| GET | `/api/v1/config` | yes | Full export (no secrets) |
| PUT | `/api/v1/config` | yes | Import a full document |
| GET | `/api/v1/config/<section>` | yes | One of `net`, `mqtt`, `nodes`, `rules`, `sec`, `sys` |
| PUT | `/api/v1/config/<section>` | yes | Validate, then stage (`net`, `mqtt`, `sec`) or save; `{ok, section, staged, confirm_s}` |
| POST | `/api/v1/config/confirm` | yes | Make staged sections permanent; 409 if nothing is staged |
| POST | `/api/v1/config/revert` | yes | Drop staged sections; 409 if nothing is staged |
| GET | `/api/v1/config/export` | yes | Same as GET config, as a download |
| POST | `/api/v1/config/import` | yes | Import; `{ok, saved:[..], staged:[..], confirm_s}` |
| GET | `/api/v1/rules` | yes | The rules section |
| PUT | `/api/v1/rules` | yes | `{"rules":[..]}` or a bare array |
| POST | `/api/v1/rules/<id>/ack` | yes | Acknowledge a manual rule |
| POST | `/api/v1/probes/scan` | yes | Request a 1-wire re-scan (accepted; not yet wired to the bus) |
| GET | `/api/v1/firmware` | yes | `{running, state, pending_verify, validate_left_s, slots[{label,state,version,size}], key_id, rollback_available}` |
| POST | `/api/v1/firmware/upload` | yes | Raw `.bin` body -> `{ok, slot, bytes, reboot_required}` |
| POST | `/api/v1/firmware/rollback` | yes | Mark the running image invalid and reboot into the other slot |
| POST | `/api/v1/firmware/mark-valid` | yes | End probation early |
| POST | `/api/v1/reboot` | yes | Planned reboot; node power untouched |
| POST | `/api/v1/factory-reset` | yes | `{"confirm":"<device id>"}` |
| POST | `/api/v1/security/password` | setup | `{"password":..}` (min 8); first call returns `{ok, first_boot:true, recovery_token}`; later calls take `current` |
| GET | `/api/v1/security/tokens` | yes | `{tokens:[{name, created_s}]}` |
| POST | `/api/v1/security/tokens` | yes | `{"name":..}` -> `{ok, name, token}` (shown once; 409 on a duplicate name) |
| DELETE | `/api/v1/security/tokens/<name>` | yes | Revoke |
| PUT | `/api/v1/security/fleet-key` | yes | `{"pem":..}`; saved directly |
| PUT | `/api/v1/security/cert` | yes | `{"cert_pem","key_pem"}` -> `{ok, cert_sha256}`; loaded by the HTTPS server at the next reboot |
| GET | `/api/v1/security/modbus-allowlist` | yes | The `sec.modbus` object |
| PUT | `/api/v1/security/modbus-allowlist` | yes | Same object; staged as part of `sec` |
| PUT | `/api/v1/security/mqtt-credentials` | yes | Any of `username`, `password`, `client_cert_pem`, `client_key_pem`; an empty string clears; 204 |
| GET | `/api/v1/log/tail?lines=N` | yes | text/plain, 1..500 lines, default 100 |
| GET | anything else | none | Static asset of the setup page |

An unknown path under `/api` returns 404; a wrong method on a known
path returns 405.

## Reply shape

Node actions, `/api/v1/cmd`, `/recover`, reboot and factory reset
return the same reply object MQTT publishes as an ack:

```json
{"v":1,"id":"api-9","ok":true,"result":"accepted","error":null,"job":1}
{"v":1,"id":"api-7","ok":false,"result":null,"error":"node 40 is on plane 4 which is not present"}
{"v":1,"id":"api-4","ok":false,"result":null,"error":"plane_reboot needs a plane target"}
```

`result` is `accepted` for anything queued (every plane verb, OTA,
reboot), with `job` set for a plane verb; `ok`, `staged` or `saved` for
configuration. A node verb on target `all` becomes one job per present
node, and `job` is the first of the group. A refusal has `ok: false`
and the reason in `error`. The outcome of a plane job is not in the
HTTP reply: read `nodes[i].last_job` in `/api/v1/state`, or the MQTT
`event` and `ack` topics. See
[Controlling nodes](../operation/nodes.md) for the outcomes and the
reasons.

### `?wait=1` on a node action

`POST /api/v1/nodes/<target>/<verb>?wait=1` (also `wait=true`,
`wait=yes`) holds the reply until the plane reports the job, and
answers with the outcome instead of the queue ack: 200 with
`"result":"done"`, or 400 with `"error":"failed: conflict"`,
`"absent: not_present"`, `"link_lost"` and so on, `job` set in both. It
is meant for scripts that would otherwise poll `/api/v1/state`.

Two limits:

- The wait is capped at 30 seconds. A longer job - a soft `off` that
  waits the node's `t_soft_off_s` for its host, or a verb on `all` that
  waits for every node of the group - keeps running and the reply falls
  back to `accepted`, so `?wait=1` never replaces reading
  `nodes[i].last_job`.
- The controller runs one HTTP server task, so a waiting request
  answers nothing else while it waits: the setup page stalls for as
  long as the action takes. Use it from scripts, not from a browser
  tab, and not in parallel.

An escalation inside `off` runs on the plane as part of the same job,
so the reply is the job's one final outcome, which is also what the
MQTT ack reports.

## Plane routes

`/api/v1/planes/<n>` and `/api/v1/planes/<n>/units` return the
controller's model of the plane as it reads it from the link, not the
rounded state payload: `volts` in 10 mV, `amps` in mA, `watts` in 0.1 W,
`temperature` and `core_temperature` in 0.1 C, `energy_joules` with the
`energy_boot_count` it belongs to, plus bookkeeping such as `ts_ms`,
`config_hash`, `serial`, `bootloader`, `segment_errors` and
`times_lost`. For dashboards use `/api/v1/state` or `/api/v1/planes`,
which carry the same data in volts, amperes, watts and degrees.

## Status payload fields

| Object | Fields |
|---|---|
| `net` | `link_up`, `ip_mode` (`dhcp`, `static`, `autoip`), `ip`, `netmask`, `gateway`, `dns[]`, `hostname`, `dhcp_fallback`, `sntp_synced` |
| `mqtt` | `enabled`, `connected`, `broker` (`host:port`), `last_error` |
| `ota` | as `/api/v1/firmware` |
| `staged` | `sections[]`, `seconds_left` |
| `modbus` | the `sec.modbus` object |

## Walkthrough

```sh
B=https://granite-37adc7.local
curl -sk $B/id
curl -sk -X POST -d '{"password":"correct horse battery"}' $B/api/v1/security/password
curl -sk -c jar -X POST -d '{"password":"correct horse battery"}' $B/api/v1/session
curl -sk -b jar -X POST -d '{"name":"ops"}' $B/api/v1/security/tokens
T=<token from the previous reply>
curl -sk -H "Authorization: Bearer $T" $B/api/v1/status
curl -sk -H "Authorization: Bearer $T" $B/api/v1/planes
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/3/on
curl -sk -H "Authorization: Bearer $T" $B/api/v1/config/export -o granite-37adc7-config.json
```

The same sequence runs against the simulator
(`cargo run -p granite-sim -- serve --scenario
granite-sim/scenarios/two-planes.toml`, `https://127.0.0.1:8443`) with
no hardware: the scenario puts two simulated planes on the link.
