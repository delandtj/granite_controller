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
| GET | `/api/v1/nodes` | yes | `{settings: <nodes section>, nodes: [...]}` |
| POST | `/api/v1/nodes/<1-8 or all>/<action>` | yes | Queue an action; body is the `args` object (optional). 200 with `accepted` when queued, 400 when refused. `all` only for `on_all` |
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
{"v":1,"id":"api-12","ok":true,"result":"ok","error":null}
{"v":1,"id":"api-13","ok":false,"result":null,"error":"refused: unknown_state"}
```

`result` is `accepted` for anything queued (every node action, OTA,
reboot), `ok`, `staged` or `saved` for configuration, or a
`refused: ...` string. The outcome of a queued action is not in the
HTTP reply: read `nodes[i].last_action` in `/api/v1/state`, or the
MQTT `event` and `ack` topics. See
[Controlling nodes](../operation/nodes.md) for the reasons.

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
curl -sk -H "Authorization: Bearer $T" -X POST $B/api/v1/nodes/3/on
curl -sk -H "Authorization: Bearer $T" $B/api/v1/config/export -o granite-37adc7-config.json
```

The same sequence runs against the simulator
(`cargo run -p granite-sim -- serve`, `https://127.0.0.1:8443`) with no
hardware.
