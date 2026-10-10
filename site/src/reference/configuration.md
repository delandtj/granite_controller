# Configuration

The configuration is one JSON document with a `schema_version` (1) and
six sections. `GET /api/v1/config` returns it, `PUT /api/v1/config` or
`POST /api/v1/config/import` loads it, and each section is also
addressable on its own at `/api/v1/config/<section>`. Secrets (admin
password hash, API tokens, MQTT password and client key, device key,
recovery token) are stored separately and never exported.

Fields missing from a document or a section take their defaults. A
section that fails to parse at boot falls back to its defaults alone
and logs a `config`/`fallback` event; the other sections are
unaffected.

Sections `net`, `mqtt` and `sec` can cut you off, so writing them
**stages** the change until `POST /api/v1/config/confirm`. Only `net`
is applied live during the confirm window; `mqtt` and `sec` take effect
after confirm and a reboot (except `sec.modbus`, which a watcher picks
up within seconds). Sections `nodes`, `rules` and `sys` are saved
directly.

## `net`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `ip_mode` | `dhcp` or `static` | `dhcp` | AutoIP is added after 30 s without DHCP. |
| `address` | CIDR string | `""` | Required for static. |
| `gateway` | string | `""` | |
| `dns` | string[] | `[]` | |
| `hostname` | string | `""` | Empty = `granite-<mac6>`. |
| `mdns` | bool | `true` | |
| `vlan` | 1..4094 or null | null | Not implemented in the current firmware. |
| `sntp` | string | `""` | Empty = DHCP option 42. |
| `t_confirm_s` | seconds | 300 | Confirm window for staged changes (UI: 30..3600). |
| `t_deadman_s` | seconds | 3600 | Static dead-man (gateway ping only); 0 = off. |

## `mqtt`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `false` | `host` required when true. |
| `host` | string | `""` | |
| `port` | u16 | 8883 | |
| `tls` | bool | `true` | |
| `ca_pem` | string | `""` | Broker CA. |
| `username` | string | `""` | Password via `/api/v1/security/mqtt-credentials`. |
| `client_id` | string | `""` | Empty = `granite-<mac6>`. |
| `site` | string | `default` | Second topic level. |
| `topic_root` | string | `granite` | First topic level. |
| `qos` | 0..2 | 1 | |
| `keepalive_s` | seconds | 30 | UI: 5..600. |
| `t_state_s` | seconds | 60 | Retained state republish interval. |
| `auto_confirm_on_connect` | bool | `false` | Not yet wired. |
| `skip_time_check` | bool | `false` | Ignored by the current firmware. |

Topic base: `<topic_root>/<site>/<device_id>`.

## `nodes`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `nodes` | 8 x NodeSettings | | Index 0 is node 1. |
| `order` | node ids | `[1,2,3,4,5,6,7,8]` | Sequence for `on_all` and the boot policy. |
| `timings` | Timings | | See below. |
| `probes` | `{rom, name}`[] | `[]` | Up to 8; `rom` is 16 hex digits. |
| `t_probe_s` | seconds | 10 | Probe read period (read at boot). |
| `vin_trim` | float | 1.0 | Multiplier on the measured bus voltage (read at boot). |

NodeSettings:

| Field | Values | Default |
|---|---|---|
| `name` | string | `node<N>` |
| `boot_policy` | `leave`, `on`, `off` | `leave` (stored; not applied by the current firmware) |
| `sense` | `enabled`, `ignore` | `enabled` |

Timings (milliseconds; out-of-range values are clamped to the bound
when saved):

| Field | Default | Min | Max |
|---|---|---|---|
| `t_short_ms` | 250 | 100 | 1000 |
| `t_hold_ms` | 8000 | 4000 | 10000 |
| `t_on_ms` | 10000 | 1000 | 300000 |
| `t_soft_off_ms` | 120000 | 5000 | 900000 |
| `t_cycle_ms` | 10000 | 1000 | 300000 |
| `t_stagger_ms` | 5000 | 0 | 60000 |
| `t_settle_ms` | 5000 | 0 | 60000 |

## `rules`

`rules`: array of up to 16 rule objects, unique ids 1..255. The rule
fields are described in [Rules](../commissioning/rules.md). Default:
the three disabled example rules.

## `sec`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `admin_password_set` | bool | `false` | False forces first-setup mode. |
| `session_hours` | hours | 12 | |
| `max_login_fails` | count | 5 | 0 disables the lockout. |
| `lockout_s` | seconds | 60 | |
| `modbus` | object | | See below. |
| `id_on_plain_http` | bool | `true` | Serve `/id` on port 80. |
| `fleet_recovery_pubkey_pem` | string | `""` | Public key only. |
| `device_cert_pem` | string | `""` | The HTTPS certificate; its key is a secret. |

`sec.modbus`:

| Field | Type | Default | Meaning |
|---|---|---|---|
| `enabled` | bool | `false` | |
| `port` | u16 | 502 | |
| `unit_id` | 1..247 | 1 | |
| `max_conn` | 1..8 | 4 | |
| `allow` | string[] | `[]` | Addresses or CIDRs; required when enabled. |

## `sys`

| Field | Type | Default | Meaning |
|---|---|---|---|
| `device_id` | string | `""` | Empty = `granite-<mac6>`. |
| `log_level` | `error`, `warn`, `info`, `debug`, `trace` | `warn` | Level published on the `log` topic. |
| `t_validate_s` | seconds | 600 | OTA probation window. |
| `coredump_report` | bool | `true` | Reserved for a core-dump summary event; not emitted by the current firmware. |
| `timezone` | string | `UTC` | Display only; wire timestamps are UTC. |

## Validation

A document is rejected as a whole (HTTP 400) when: more than 8 probes
or a bad ROM id, more than 16 rules,
duplicate rule ids or id 0, static mode without an address, MQTT
enabled without a host, Modbus enabled without an allow-list, or a
`schema_version` newer than 1. Timings outside their range and a QoS
above 2 are clamped rather than rejected. IP and CIDR syntax is not
validated by the core; a typo there is what the commit-confirm step is
for.

## Example export

```json
{
  "schema_version": 1,
  "net": {"ip_mode": "dhcp", "address": "", "gateway": "", "dns": [], "hostname": "",
          "mdns": true, "vlan": null, "sntp": "", "t_confirm_s": 300, "t_deadman_s": 3600},
  "mqtt": {"enabled": true, "host": "broker.site.example", "port": 8883, "tls": true,
           "ca_pem": "-----BEGIN CERTIFICATE-----\n...", "username": "granite-37adc7",
           "client_id": "", "site": "pod1", "topic_root": "granite", "qos": 1,
           "keepalive_s": 30, "t_state_s": 60, "auto_confirm_on_connect": false,
           "skip_time_check": false},
  "nodes": {"nodes": [{"name": "node1", "boot_policy": "on", "sense": "enabled"}, "..."],
            "order": [1, 2, 3, 4, 5, 6, 7, 8],
            "timings": {"t_short_ms": 250, "t_hold_ms": 8000, "t_on_ms": 10000,
                        "t_soft_off_ms": 120000, "t_cycle_ms": 10000,
                        "t_stagger_ms": 5000, "t_settle_ms": 5000},
            "probes": [{"rom": "28ff000000000001", "name": "inlet"}],
            "t_probe_s": 10, "vin_trim": 1.0},
  "rules": {"rules": ["..."]},
  "sec": {"admin_password_set": true, "session_hours": 12, "max_login_fails": 5,
          "lockout_s": 60,
          "modbus": {"enabled": false, "port": 502, "unit_id": 1, "max_conn": 4, "allow": []},
          "id_on_plain_http": true, "fleet_recovery_pubkey_pem": "", "device_cert_pem": ""},
  "sys": {"device_id": "", "log_level": "warn", "t_validate_s": 600,
          "coredump_report": true, "timezone": "UTC"}
}
```
