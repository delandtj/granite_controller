# Granite controller firmware

**Status**: Accepted
**Date**: 2026-10-09
**Updated**: 2026-10-10 (user review: off escalates after t_soft_off, fleet recovery key added, Modbus TCP in release 1, signed OTA without eFuses confirmed)

---

## Context

The rev C controller (docs/controller.md) is an ESP32-C6-WROOM-1-N8 with
a W5500 SPI Ethernet MAC/PHY, two MCP23017 expanders on an internal I2C
bus (U14 drives 16 photoMOS relays that press PWR/RST on 8 nodes, U15
reads the 8 node power LEDs and 4 dry contacts), a TMP1075 board sensor,
4 DS18B20 probe connectors on one 1-wire bus, a VIN divider on an ADC pin,
an external I2C bus on J8/J9 for add-on expanders, Qwiic sensors and the
future power board (LTC4282 x8), USB Serial/JTAG on USB-C, and no
buttons.

Constraints that shape everything below:

- The board is sealed and submerged after commissioning. After that the
  only paths in are Ethernet and, via the relays, nothing. A firmware or
  configuration mistake that takes the controller off the network is a
  tank drain. The user notes each board's MAC address before it goes in.
- 8 nodes depend on it. A controller restart, crash or update must never
  change node power state as a side effect, and a stuck relay press must
  be impossible (a held PWR button force-powers a node off after 4 s).
- Radio is useless in the fluid: Wi-Fi and BLE stay off. Ethernet is the
  only network path.
- The user knows ESP-IDF and wants Rust.
- Decided by the user on 2026-10-09: Rust on ESP-IDF; northbound is MQTT
  for logging, monitoring and control, plus an HTTP setup page; Modbus is
  required as well (Modbus TCP on the controller now, Modbus RTU to the
  RS-485 "ducks" of the pod architecture later); security is in scope
  (signed firmware, authenticated API).
- Hardware facts from docs/controller.md that firmware must honour:
  IOCON.ODR = 1 on every expander before enabling interrupts (shared
  open-drain EXP_INT), IOCON.MIRROR = 1 on U15, GPA7/GPB7 never used as
  inputs, external bus at 100 kHz, internal at 400 kHz, console off UART0
  (the 1-wire bus sits on U0TXD/U0RXD), U14 held in reset through GPIO10
  whenever no press is in progress.

Related: docs/controller.md (hardware), docs/power-board.md (LTC4282
provisioning requirements), docs/handoff.md (pod architecture: this
board stays the integrated controller for one-frame sites).

---

## Decision

One firmware image, Rust on ESP-IDF (std, esp-idf-svc / esp-idf-hal),
split into a host-testable core crate and a thin ESP-IDF binary. All
hardware access goes through traits so the core runs on a host simulator
and could move to esp-hal/Embassy later.

Northbound:

1. MQTT over TLS to a configured broker: retained state, events, log
   lines, and a command/ack pair. This is the normal control path.
2. HTTPS setup page and JSON API on the controller: commissioning,
   configuration, status, firmware upload, recovery.
3. Modbus TCP server exposing the same data model as a register map,
   off by default, IP allow-listed. The register map is the contract the
   future RS-485 duck protocol reuses.

Reachability is a first-class property: network configuration changes
are commit-confirmed with automatic revert, OTA images self-validate by
reaching the network and are rolled back otherwise, a factory partition
is the last resort, and a per-device recovery token allows a factory
reset over the network when the admin password is lost. Identity is the
eFuse MAC: hostname, mDNS name, MQTT client id and the default device id
all derive from it, so a noted MAC is enough to find and recover a board.

Node safety: every relay press is bounded by a hardware deadline (a
timer drops the U14 reset line, which releases all 16 relays without
any I2C traffic), node state is never changed on controller boot or
reconnect unless a per-node boot policy says so, and actions are
checked against the sensed power LED before and after.

---

## Architecture Overview

### Crates

```
firmware/
  Cargo.toml              workspace
  granite-core/           no_std + alloc, host-testable, no ESP-IDF
  granite-fw/             ESP-IDF binary for the ESP32-C6
  granite-sim/            host binary: core + fake hardware + the same HTTP
                          page, for UI work and integration tests (optional)
  docs/adr/               this file and successors
  partitions.csv, sdkconfig.defaults, build notes in firmware/README.md
```

### Component Breakdown

1. **Hardware abstraction** (`granite-core/src/hal.rs`)
   - Traits: `NodeSwitches` (assert/release PWR/RST per node, release
     all), `NodeSense` (8 power-LED bits), `DryInputs` (4 bits),
     `Probes` (enumerate DS18B20 ROM ids, read temperatures),
     `BoardTemp`, `BusVoltage`, `StatusLed`, `Clock`, `Persist`
     (namespaced key/value), `Reboot`.
   - Implemented in granite-fw against ESP-IDF; implemented in
     granite-sim with fakes driven from a scenario file.

2. **Expander driver** (`granite-fw/src/mcp23017.rs`)
   - Small in-tree MCP23017 driver over embedded-hal 1.0 `I2c`: bank 0,
     IOCON (ODR, MIRROR, SEQOP), IODIR/GPPU/GPINTEN/DEFVAL/INTCON,
     GPIO/OLAT, INTCAP read-to-clear. Shadows OLAT and IODIR and verifies
     them on every write and on a 1 s readback; a mismatch is an
     `ExpanderFault`.
   - Reset handling: U14 and U15 share EXP_nRESET_INT (GPIO10). At boot
     GPIO10 is driven low first, then both expanders are configured
     after release. Any I2C error on the internal bus reinitialises both
     through a reset pulse and aborts the action in progress.

3. **Node actuator** (`granite-core/src/actuator.rs`, backing in
   `granite-fw/src/relays.rs`)
   - Owns U14 and GPIO10. One action at a time per node, one physical
     press at a time overall (the queue serialises; a stagger is a queue
     of presses with delays).
   - Press primitive: raise GPIO10 (U14 out of reset, all outputs
     low), write the one bit, arm the deadline, wait the duration, clear
     the bit, drop GPIO10 if the queue is empty. Deadline = press
     duration + 500 ms, max 12 s, implemented in granite-fw as an
     esp_timer whose callback only writes GPIO10 low (ISR-safe, no I2C).
     If the deadline fires the actuator logs a fault, reinitialises U14
     and fails the action. The panic handler and the task watchdog
     timeout both also drive GPIO10 low before anything else.
   - Actions (all take a node id, 1-8, or `all`, and return a result
     through the ack channel):

     | Action | Behaviour |
     |---|---|
     | `on` | If LED off: short PWR press. Wait for LED on, timeout `t_on` (default 10 s). If already on: ok, no press. |
     | `off` | If LED on: short PWR press (ACPI soft-off). Wait for LED off up to `t_soft_off` (default 120 s); if still on, escalate to `force_off` (user decision 2026-10-10). Event `soft_off_timeout` is emitted before the escalation. `args.no_escalate: true` keeps the old behaviour and returns `shutdown_pending`. |
     | `force_off` | If LED on: hold PWR until LED off plus 500 ms, max `t_hold` (default 8 s, range 4-10). |
     | `reset` | If LED on: short RST press. If off: rejected. |
     | `cycle` | `off` (which escalates as above) or `force_off` (`args.hard: true`), wait `t_cycle` (default 10 s), `on`. |
     | `press` | Raw press of `pwr` or `rst` for a given duration (100 ms-10 s) regardless of LED state. For odd BIOS behaviour; always logged as an event. |
     | `on_all` | Staggered `on` in the configured node order with `t_stagger` between presses (default 5 s). Skips nodes already on. |

     Short press = `t_short` (default 250 ms, range 100-1000 ms).
   - Node state model (`granite-core/src/node.rs`): per node
     `Unknown | Off | On | Busy(action, started_at)`. `On`/`Off` come
     from the LED with 100 ms debounce. A node with the LED off while a
     press is pending is reported as `Off`; `Hung` is not a state the
     controller can know, so it is not claimed (the reader has `On`
     plus the last reset time and can decide).
   - Boot policy per node: `leave` (default), `on`, `off`. Applied once
     per controller boot after a `t_settle` of 5 s so the LED sense is
     stable, with the stagger. A controller restart therefore never
     changes a node unless the user asked for it.

4. **Sensing** (`granite-fw/src/sense.rs`)
   - U15 via EXP_INT (GPIO0, falling edge) plus a 1 s poll as backstop;
     reads INTCAP then GPIO. Dry inputs are debounced 50 ms in
     firmware on top of the RC.
   - DS18B20 on GPIO16 through the Espressif `onewire_bus` + `ds18b20`
     components (RMT based), pulled in with esp-idf-sys extra
     components; GPIO17 is set to input, no pull, so the UART RX side of
     the same net stays passive. Probes are discovered at boot and on
     request, identified by ROM id, mapped to user names; 12-bit, read
     every `t_probe` (default 10 s). A missing probe is reported, not
     an error.
   - TMP1075 (0x48, internal bus) every 1 s.
   - VIN on ADC1 ch5 with the ESP-IDF curve calibration, 16-sample
     average every 1 s, scaled by the 110/10 divider with a per-board
     trim factor in config.
   - All readings land in a single `Observed` snapshot struct in core
     with a monotonic timestamp per field; publishers and the rule
     engine read that, never the hardware.

5. **Rule engine** (`granite-core/src/rules.rs`)
   - Up to 16 rules, evaluated every 1 s against `Observed`:
     `{ id, enabled, source, op, threshold, hysteresis, hold_s, action,
     target, rearm }`.
     Sources: `probe[n]`, `probe_max`, `board_temp`, `vin`, `dry_in[n]`,
     `node_on[n]`, `mqtt_connected`, `link_up`, `uptime_s`.
     Ops: `>`, `<`, `==` (for booleans), `changed`.
     Actions: any actuator action on `target` (node or all), `event`
     (publish only). `hold_s` = condition must be continuously true
     that long; `rearm` = `auto` (re-fires after the condition cleared
     past the hysteresis) or `manual` (fires once until acknowledged).
   - Examples the defaults ship with, disabled: probe_max > 70 C for 30 s
     -> force_off all; dry_in[1] == closed for 2 s -> force_off all
     (leak float); vin < 15 V for 5 s -> event.
   - Rules run whether or not the broker is reachable; that is the
     standalone behaviour. Firing is logged as an event and acked when
     the broker is back.

6. **Network** (`granite-fw/src/net.rs`)
   - W5500 through esp-idf-svc `EthDriver::new_spi` (ESP-IDF
     `esp_eth_mac_new_w5500`, MAC-raw, lwIP on the C6), SPI at 20 MHz
     to start, 33 MHz max per datasheet, INT on GPIO22, RST on GPIO23.
     MAC address = eFuse base MAC with the Ethernet offset
     (`esp_read_mac(ESP_MAC_ETH)`), never the W5500's own; this is the
     address the user notes.
   - IPv4: DHCP by default; static optional. IPv6 link-local always.
     When DHCP yields nothing within 30 s, IPv4 link-local (AutoIP) is
     added and DHCP keeps retrying. Hostname and mDNS name
     `granite-<last 6 hex of MAC>` (overridable), mDNS advertises
     `_https._tcp` and `_mqtt-dev._tcp` with the device id.
   - SNTP from DHCP option 42 or a configured server; until time is
     known the clock starts at the firmware build time so TLS date
     checks pass and timestamps are monotonic.
   - Commit-confirmed network changes: a change to IP, VLAN, hostname or
     anything that can cut the session is written to a staging slot and
     applied; the client must call `confirm` through the new address
     within `t_confirm` (default 5 min) or the previous config is
     restored and the controller reboots into it. Same for MQTT broker
     changes: confirm = the new broker connects and the client confirms,
     or auto-confirm after a successful connection if the user ticked
     "auto-confirm on connect".
   - Static-config dead-man: if a static IP config has had no gateway
     ARP reply and no accepted TCP connection for `t_deadman` (default
     1 h, 0 = off), DHCP is started alongside as a fallback without
     dropping the static address. Noted in the status page.

7. **MQTT** (`granite-fw/src/mqtt.rs`, messages in
   `granite-core/src/msg.rs`)
   - esp-mqtt via esp-idf-svc, TLS 1.2+ with a configured CA (PEM),
     username/password or client certificate, QoS 1, keepalive 30 s,
     LWT. Reconnect with exponential backoff 1-60 s.
   - Topic root `granite/<site>/<device>` where `<site>` is config
     (default `default`) and `<device>` is the device id (default
     `granite-<mac6>`).

     | Topic | Dir | Retained | Payload |
     |---|---|---|---|
     | `.../status` | out | yes | `{"online":true,"fw":"...","ip":"...","uptime_s":N,"boot_reason":"..."}`; LWT writes `{"online":false}` |
     | `.../state` | out | yes | full snapshot: nodes (state, name, last action), probes, board_temp, vin, dry_in, link; on any change (coalesced 200 ms) and every `t_state` (default 60 s) |
     | `.../event` | out | no | `{"ts":..,"kind":"node_state|action_done|action_failed|rule_fired|fault|ota|config","...":..}` |
     | `.../log` | out | no | one JSON line per log record at or above the configured level (default `warn`) |
     | `.../cmd` | in | - | `{"id":"<client chosen>","action":"on","target":3,"args":{...}}` |
     | `.../ack/<id>` | out | no | `{"id":..,"ok":true,"result":"...","error":null}` published when the action completes (not when accepted); `accepted` event is emitted on receipt for long actions |

   - Commands: every actuator action, `rule_ack`, `probe_scan`,
     `config_get`, `config_set` (same JSON as the HTTP API, same
     commit-confirm rules), `ota` (`{"url":..,"sha256":..}`; the
     controller pulls the image over HTTPS), `reboot`, `factory_reset`
     (needs `"confirm":"<device id>"`).
   - The broker is trusted for control; authentication is the broker's
     TLS + credentials. There is no second signature layer on commands.

8. **HTTP setup page and API** (`granite-fw/src/http.rs`, static assets
   embedded with `include_bytes!`, gzip'd; UI is plain HTML + a small JS
   file, no framework)
   - HTTPS (esp_https_server) with a device certificate: ECDSA P-256
     self-signed generated on first boot (seconds on the C6), SHA-256
     fingerprint shown on the USB console and the status page; the user
     may upload their own cert/key. Port 80 serves only a redirect and
     the unauthenticated identity endpoint below.
   - Auth: one admin password set on first boot (the page refuses
     everything else until it is set; no default password). Stored as
     PBKDF2-HMAC-SHA256, 20k iterations, per-device salt. Session cookie
     (random 32 bytes, 12 h) for the page; `Authorization: Bearer <api
     token>` for scripts, tokens created on the security page. 5 failed
     logins -> 60 s lockout, logged as an event.
   - Pages: Status (nodes with action buttons, sensors, network, MQTT,
     firmware slots), Network, MQTT, Nodes (names, order, boot policy,
     timings), Rules, Security (password, API tokens, device cert, MQTT
     CA/client cert, Modbus allow-list, recovery token display at first
     setup only), Firmware (upload .bin, slot status, rollback to
     previous, "mark current as valid"), Maintenance (reboot, export /
     import config JSON, factory reset, log tail, probe scan).
   - API: `/api/v1/...` JSON, same command vocabulary as MQTT so the two
     share one dispatcher in core (`Command` enum, `Reply` enum).
   - Identity endpoint `/id` (HTTP and HTTPS, unauthenticated, read
     only): `{"device":"granite-xxxxxx","mac":"..","fw":"..","cert_sha256":".."}`.
     This is what the user hits after noting a MAC.

9. **Modbus TCP** (`granite-fw/src/modbus.rs`, map in
   `granite-core/src/modbus_map.rs`)
   - `rmodbus` for frame handling over std `TcpListener`, port 502,
     unit id 1, max 4 connections, off by default, IP/CIDR allow-list
     required to enable (Modbus has no auth).
   - Map (versioned, `modbus_map.md` generated from the core table):
     discrete inputs 0-7 node LED, 8-11 dry inputs, 12 link, 13 mqtt;
     input registers: 0-7 probe temps (0.01 C, signed, 0x8000 =
     missing), 8 board temp, 9 vin (mV), 10 uptime hi, 11 uptime lo,
     12 fw major.minor, 13 fault flags; holding registers: 0-7 per-node
     command word (write 1 on, 2 off, 3 force_off, 4 reset, 5 cycle;
     reads back the node state 0 unknown/1 off/2 on/3 busy), 8 `on_all`
     trigger, 9 last command result; coils 0-7 mirror node state and
     accept 1 = on / 0 = off.
   - The same table defines the register semantics the RS-485 duck
     protocol will use per channel kind, so a frame duck looks like
     this controller seen through a serial port.

10. **Configuration and persistence** (`granite-core/src/config.rs`,
    `granite-fw/src/store.rs`)
    - One `Config` struct, serde, schema version field; stored in NVS as
      a few JSON blobs per section (`net`, `mqtt`, `nodes`, `rules`,
      `sec`, `sys`) so a corrupt section loses only itself. Secrets
      (passwords, tokens, private keys) in a separate NVS namespace
      excluded from export.
    - Defaults are a full valid config: DHCP, MQTT disabled, Modbus
      disabled, boot policy `leave` everywhere, rules present but
      disabled.
    - Staging + confirm for sections that affect reachability (see
      Network). Export/import as one JSON document (secrets omitted) for
      cloning 8 controllers with different MACs.
    - Factory reset erases all namespaces except `factory` (recovery
      token, device cert if the user chose to keep it) and reboots.

11. **Firmware update and boot safety** (`granite-fw/src/ota.rs`)
    - Partition table (8 MB): `nvs` 192 K, `otadata` 8 K, `coredump`
      64 K, `factory` 2.5 M, `ota_0` 2.5 M, `ota_1` 2.5 M. The image
      flashed at the bench goes to `factory` and to `ota_0`; OTA
      alternates `ota_0`/`ota_1`; `factory` is only ever rewritten over
      USB.
    - Images are signed (ESP-IDF app signature scheme, ECDSA P-256,
      `CONFIG_SECURE_SIGNED_APPS_NO_SECURE_BOOT` +
      `CONFIG_SECURE_SIGNED_ON_UPDATE_NO_SECURE_BOOT`): the running app
      refuses an OTA image whose signature does not verify against the
      embedded public key. Secure Boot V2 and flash encryption are not
      enabled by default: both burn eFuses irreversibly and the board is
      physically inaccessible once submerged, which already blocks the
      attacks they stop. They remain a documented production option.
    - Rollback: `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE`. A new image is
      `pending_verify` until the firmware has (a) initialised the
      expanders, (b) got link and an IP, and (c) either connected to the
      configured broker or served an authenticated HTTPS request, within
      `t_validate` (default 10 min). Then it marks itself valid.
      Otherwise it marks itself invalid and reboots into the previous
      slot; if that also fails the bootloader falls through to
      `factory`. The task watchdog (10 s) and a brown-out handler also
      count as failures while pending.
    - OTA sources: HTTPS pull (URL + expected SHA-256, CA = the MQTT CA
      or a separate one) and HTTPS push (multipart upload on the setup
      page). Progress and result go out as events.
    - Core dumps to the `coredump` partition; summary published as an
      event on the next boot and shown on the Maintenance page.
    - Prior art (user, 2026-10-10): ~/Electronics/wfi028t-controller,
      docs/adr/0002-ota-and-console-port.md and fw/src/ota.rs. That
      firmware is bare-metal esp-hal, so its partition writing, ed25519
      header and TCP push port do not carry over (ESP-IDF's esp_ota_ops
      and the app signature block do that here). What does carry over:
      the probation model (image boots pending, confirms once on a
      concrete readiness ladder within a deadline, otherwise resets and
      the rollback bootloader aborts it), every step logged as an event,
      and the host tool shape: `granite-ota push <host> [--wait]` that
      builds, uploads over HTTPS and polls `/id` until the image reports
      valid or a rollback shows up; signing key kept at
      ~/.config/granite/ (mode 0600, never in the repo), key id shown by
      the running image.

12. **Console and logging** (`granite-fw/src/console.rs`)
    - `CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y`; UART0 is left alone (the
      ROM bootloader still prints there at boot, harmless on the 1-wire
      bus). An interactive console on USB with: `id`, `status`,
      `net`, `set-password`, `recovery-token` (print it again),
      `factory-reset`, `ota-mark-valid`, `reboot`. Physical access =
      full access; the console is not a network path.
    - `log` crate -> ESP-IDF log; records are kept in a 16 KB RAM ring
      (last lines on the Maintenance page and `log tail`) and published
      to MQTT at the configured level.

13. **Identity and recovery** (`granite-fw/src/identity.rs`)
    - Device id `granite-<mac6>`; MAC and id printed on the USB console
      at every boot and on the `/id` endpoint.
    - Per-device recovery token: 20 random bytes (base32, grouped),
      generated at first boot and at every factory reset, stored in the
      `factory` namespace, shown once on the first-setup page and on the
      USB console. `POST /recover` on HTTPS with the token (rate-limited
      to one attempt per minute, logged) performs a factory reset without
      the admin password. The user keeps MAC + token together per board.
    - Fleet recovery key (user decision 2026-10-10): an ECDSA P-256
      public key, stored in the `factory` namespace. Set at commissioning
      on the Security page or by the USB console, and optionally baked
      into the image at build time (`FLEET_RECOVERY_PUBKEY` env, PEM) so
      a freshly flashed board already trusts it. `POST /recover` also
      accepts `{"device":"granite-xxxxxx","nonce":"..","sig":".."}` where
      `nonce` comes from `/id` (fresh per request, valid 5 min, one use)
      and `sig` is the fleet private key's signature over
      `device || nonce || "factory-reset"`. Same rate limit and logging.
      The private key never touches a board; a small host tool
      (`granite-sim recover`) produces the request. Replacing the fleet
      key requires the admin password or the old fleet key.
    - LED: 1 Hz heartbeat when healthy; 4 Hz while an OTA image is
      pending validation; 0.25 Hz when no link; solid on during a press.

14. **Power board hook** (`granite-fw/src/ltc4282.rs`, later)
    - External bus on LP I2C (hardware, port `LP_I2C_NUM_0`, GPIO6/7,
      100 kHz) through the standard ESP-IDF i2c_master driver, which
      resolves docs/controller.md open item 3; bit-banged I2C is the
      fallback if the esp-idf-hal binding cannot select the LP port.
    - Bus scan at boot and on request, published in `state`.
    - LTC4282 EEPROM provisioning per docs/power-board.md "EEPROM
      provisioning": compare, write only on mismatch, hard-coded
      FET_ON = 1, verify, alarm on failure. Fault counting and
      auto-disable after `n_retry` faults in `t_window`. Not in the
      first release; the trait boundary (`PowerChannels`) is.

### Tasks and data flow

```
 [EXP_INT isr] -> sense task --+
 [1-wire/ADC/TMP timer] -------+--> Observed (RwLock) --> rule engine (1 s)
                                                   |           |
 MQTT rx ---+                                      |           v
 HTTP api --+--> Command channel --> dispatcher --> actuator queue --> U14/GPIO10
 Modbus ----+                           |              |   ^
                                        v              v   | deadline timer
                                   Reply/ack      events   (GPIO10 low)
                                        |              |
                                        +--> MQTT tx <-+--> log ring
```

FreeRTOS tasks: `net` (ESP-IDF event loop), `mqtt`, `http`, `modbus`,
`actuator`, `sense`, `rules` (merged with sense on one tick), `console`.
The actuator, sense and rules tasks are subscribed to the task watchdog.
Core logic is synchronous and single-threaded per component; cross-task
traffic uses `std::sync::mpsc` and `RwLock<Observed>`.

---

## Alternatives Considered

### Bare-metal esp-hal + Embassy
- **The idea**: no ESP-IDF, async Rust, embassy-net with the W5500
  driver (embassy-net-wiznet), own OTA.
- **Optimizes for**: smallest firmware, pure Rust, cleanest async model.
- **Sharpest tradeoff**: OTA with rollback, app signing, TLS server,
  MQTT-over-TLS and NVS all have to be assembled from younger crates;
  a sealed board needs the boring, proven update path on day one.
- **Bets on**: the esp-hal ecosystem reaching ESP-IDF parity for OTA and
  TLS before this firmware needs a second major revision. The trait
  boundary in granite-core keeps this door open.

### C with ESP-IDF directly
- **The idea**: the vendor's native language and examples.
- **Optimizes for**: zero binding friction, every ESP-IDF example applies.
- **Sharpest tradeoff**: the state machines, rule engine and config
  model are where the bugs will be, and that logic is far easier to make
  correct and host-test in Rust.
- **Bets on**: binding gaps in esp-idf-svc being rare. They are not zero
  (W5500 config structs changed shape in the 2.0 component), so the
  binary crate keeps raw esp-idf-sys calls acceptable where a binding is
  missing.

### HTTP/JSON only, no MQTT
- **The idea**: the management server polls each controller.
- **Optimizes for**: simplicity, no broker to run.
- **Sharpest tradeoff**: no push for events, no central log, every
  controller needs an inbound route; a pod with dozens of ducks and
  controllers wants a bus.
- **Bets on**: the fleet staying small. Rejected by the user.

### OpenRPC (Hero style) as the API
- **The idea**: the user's other services speak OpenRPC.
- **Optimizes for**: uniform tooling with the Hero stack.
- **Sharpest tradeoff**: no embedded tooling, and a bespoke RPC on a
  sealed device is one more thing to get right. Keeping the command
  vocabulary as one `Command` enum means an OpenRPC transport can be
  added as a thin adapter later.
- **Bets on**: the management server being the only client. Not the
  case: browsers and PLC-style tools (Modbus) are clients too.

### Modbus TCP as the primary control path
- **The idea**: PLC-grade protocol everyone has tooling for.
- **Optimizes for**: integration with building/industrial monitoring.
- **Sharpest tradeoff**: no authentication, no events, no rich payloads.
- **Bets on**: a trusted management VLAN. Kept as a secondary, allow-listed
  interface for exactly those tools.

### Secure Boot V2 + flash encryption from day one
- **The idea**: full chain of trust in eFuses.
- **Optimizes for**: resistance to physical flash replacement.
- **Sharpest tradeoff**: irreversible; a signing-key mistake bricks
  boards that are about to be sealed in oil; development friction.
- **Bets on**: physical access being a real threat. The tank is the
  physical security. App-signature verification on OTA gives the
  remote-attack protection without eFuses; the eFuse option stays
  documented for a production batch.

---

## Consequences

### Positive
- Node power state cannot be changed by a controller crash, reboot, OTA
  or a firmware hang (reset-held expander, hardware-bounded presses).
- Every reachability change is reversible by time (commit-confirm, OTA
  self-validation, factory partition, recovery token).
- One command vocabulary serves MQTT, HTTP and (reduced) Modbus; the
  core crate and its tests run on the host.
- The data model and register map are reusable for the ducks.

### Negative
- ESP-IDF build toolchain (espup, ldproxy, embuild) is heavier than a
  pure-Rust build and pins the ESP-IDF version.
- TLS server + TLS MQTT client + lwIP + HTTP assets on 512 KB SRAM is
  comfortable but not generous; large JSON responses must stream.
- Modbus without auth means the allow-list is the only guard; it is off
  by default.

### Risks
- esp-idf-svc binding gaps (W5500 config, LP I2C port selection,
  onewire component): mitigated by allowing raw esp-idf-sys calls in
  granite-fw and by the bit-bang I2C fallback.
- A wrong `t_hold` or BIOS behaviour that differs from ACPI defaults:
  all timings are per-controller config; `press` exists for the odd
  board; actions verify through the LED.
- The PLED sense was never tested against the real motherboard
  (docs/controller.md open item 4): the actuator treats `Unknown` LED
  state as "do not infer", and `press` still works, so control is not
  lost if sensing is wrong.
- Signing key management: the public key is baked into the image; losing
  the private key means no more OTA for that batch (USB reflash only,
  which is impossible submerged). Mitigation: the key lives in the
  user's secret store, and the Firmware page shows the key id of the
  running image.

---

## What an Expert Would Ask

**Q: What exactly happens to the relays if the firmware deadlocks with a
press in progress and the task watchdog does not fire because the
watchdog task itself is blocked?**
A: The deadline timer is an esp_timer in the high-priority timer task,
independent of the actuator; its callback writes GPIO10 low, which
resets U14 and opens all 16 photoMOS outputs. If the timer task is also
dead, the task watchdog (hardware-backed, interrupt-driven) fires and
its handler drives GPIO10 low before the panic path. If interrupts are
globally off for more than the press duration, the RTC watchdog resets
the chip and GPIO10 goes high-Z; the pull-down holds U14 in reset. There
is no software state in which a relay stays closed for longer than the
longest watchdog period plus the press duration (about 22 s worst case).
That is longer than a motherboard's 4 s force-off threshold, so the
worst-case outcome of a triple failure is a node powered off, not a
damaged one.

**Q: An operator fat-fingers a static IP on a submerged board. Walk
through it.**
A: The change is staged, applied, and a 5 min confirm timer starts. The
page tells the operator to reconnect at the new address and press
Confirm. If nothing confirms, the previous config is restored and the
controller reboots. Meanwhile mDNS still answers on the link-local
address, and the dead-man adds DHCP after an hour if nobody ever talks
to it. If the operator also loses the password, `/recover` with the
recovery token resets to DHCP. The only unrecoverable case is a VLAN
misconfiguration on the switch side, which is not the firmware's.

**Q: How do you avoid pressing PWR on a node that is on but whose LED
you cannot see (broken LED wire, unknown motherboard drive level)?**
A: Actions that depend on state (`on`, `off`, `reset`, `cycle`) refuse
when the node state is `Unknown` (sense faulted) unless the command
carries `"force":true`; `press` never looks at the LED. Sense is
`Unknown` only when U15 is unreadable; an unlit LED on a running node
reads as `Off`, and that failure mode is exactly why the first thing to
check at bring-up is the PLED level on the real motherboard. The config
has a per-node `sense: enabled|ignore` flag for boards whose LED cannot
be read: `ignore` makes every action behave like `press` with the
standard durations.

**Q: Why trust the broker for control without end-to-end signing of
commands?**
A: The broker is on the management network, speaks TLS with a pinned
CA, and authenticates the controller; the controller's ACL on the
broker side limits who can publish to `cmd`. Signing commands would
need key distribution to every client, which for a browser-based
operator is the same trust problem moved one step. If the deployment
later needs it, `Command` has a reserved `sig` field and the dispatcher
has one verification hook.

**Q: Two OTA slots of 2.5 MB each plus factory: is 2.5 MB enough for
ESP-IDF + TLS server + MQTT + HTTP assets in Rust?**
A: Comparable esp-idf-svc firmware with HTTPS, MQTT-TLS and lwIP lands
around 1.3-1.8 MB in release with LTO and `opt-level = "s"`. 2.5 MB
leaves margin for the HTTP assets (gzip'd, under 100 KB) and growth. If
it does not fit, `factory` shrinks first (it only needs to be
recoverable, so a reduced build with HTTPS + OTA and no Modbus/rules is
acceptable there).

**Q: What does the controller do on the 200th boot when NVS is worn or
corrupted?**
A: NVS wear at the write rates here (config changes, OTA state) is
negligible, but a corrupt section deserialises to defaults for that
section only, logged as an event; `sec` corrupt means no admin password,
which forces first-setup mode: reachable, but only the recovery token
or USB can set a new password. The device never boots into an
unreachable state because the network section's default is DHCP.

---

## Implementation Plan

### Decisions you will probably want to tweak

- **Command vocabulary and JSON shape** (one `Command` enum shared by
  MQTT, HTTP, Modbus).
  Alternative: per-transport schemas. Cost to change later: every
  client; keep a `v` field and the `/api/v1` prefix from day one.
- **Topic layout** `granite/<site>/<device>/{status,state,event,log,cmd,ack/<id>}`.
  Alternative: Homie or Home Assistant discovery conventions. Cost: the
  broker ACLs and the management server's subscriptions; cheap while
  there is one consumer.
- **Node action semantics** (what `off` does when soft-off fails,
  defaults for `t_hold`, `t_stagger`).
  Alternative: `off` escalating to `force_off` automatically. Cost:
  behaviour only, config-driven; cheap.
- **Modbus register map**. Alternative: a map that mirrors the future
  duck registers one-to-one. Cost: PLC configs; freeze it with a map
  version register before the first external integration.
- **Config storage as JSON blobs per section in NVS**. Alternative: one
  blob, or typed NVS keys. Cost: a migration function per schema bump;
  the schema version field makes that mechanical.
- **Recovery**: per-device token plus a fleet ECDSA public key.
  Alternative: fleet-wide shared secret (simpler, but a secret on every
  board). Cost to change: the `/recover` handler and the host tool only.
- **PBKDF2 for the admin password**. Alternative: Argon2id (memory
  cost is the problem on 512 KB). Cost: a rehash on next login.

### Known unknowns and how the plan absorbs them

- **LP I2C from the HP core through esp-idf-hal**: default is the
  hardware port; signal to pivot: the hal cannot address `LP_I2C_NUM_0`
  in the pinned ESP-IDF version, then raw esp-idf-sys or bit-bang on
  GPIO6/7.
- **Motherboard PLED drive and switch returns**: default assumes the
  sense works as designed; signal: LED reads `Off` on a running node at
  bring-up, then `sense: ignore` per node and a hardware note.
- **W5500 binding shape in esp-idf-svc for the pinned ESP-IDF**:
  default `EthDriver::new_spi`; signal: compile failure or the 2.0
  component's nested config, then raw `esp_eth_mac_new_w5500`.
- **Broker policy** (who runs it, ACL conventions, site naming): default
  `site = default`; signal: the management server design.
- **Memory headroom with HTTPS server + MQTT TLS at once**: default
  both on; signal: heap below 60 KB free after connect, then stream
  responses, reduce TLS fragment size, or serve the setup page over
  HTTP with a warning on untrusted networks.
- **Time without SNTP** for TLS date checks: default build-time clock;
  signal: a broker cert that is not yet valid at build time, then
  `skip time check` option in the MQTT section.

### The mechanical work

Component specs, each independently testable:

- `granite-core`: `hal` traits; `node` state machine with a table-driven
  test per action x initial state x LED response; `actuator` queue with
  a fake clock; `rules` with scenario tests; `config` with
  serde round-trip and default tests; `msg` (Command/Reply/Event) with
  JSON fixtures; `modbus_map` table plus a generator for
  `modbus_map.md`.
- `granite-fw`: ESP-IDF glue per component above; `sdkconfig.defaults`
  (USB console, no Wi-Fi/BT, rollback, signed-on-update, task WDT 10 s,
  mbedTLS ECDSA, lwIP AutoIP, mDNS); `partitions.csv`; `build.rs`
  embedding the gzip'd assets; signing key handling in `firmware/README.md`
  (`espsecure.py` key generation, CI signing, key id in the image).
- `granite-sim`: the same HTTP server (axum) over the core with fakes,
  so the setup page and the MQTT topic contract can be exercised with a
  local broker before boards arrive.
- Bring-up order on hardware: USB console + identity -> expanders held
  in reset, readback -> Ethernet link, DHCP, `/id` -> HTTPS + first setup
  -> sense (LED, dry, probes, TMP, VIN) -> actuator with the deadline
  proven by a deliberately hung task -> MQTT -> OTA with rollback
  proven by a deliberately broken image -> Modbus -> rules.

### Review outcome (2026-10-10)

1. `off` escalates to `force_off` after `t_soft_off`. Done above.
2. Fleet-wide recovery key in addition to the per-device token. Done
   above.
3. Modbus TCP server ships in release 1, off by default (the question
   was whether to defer the server; it is not deferred).
4. App-signature verification on OTA without eFuses for the first batch;
   Secure Boot V2 stays a later production step.

---

## Open Questions

**Architecture-changers**
- [ ] "Modbus maybe in some way over that": does this mean Modbus TCP on
      the controller (as specified), Modbus frames tunnelled over MQTT,
      or only that the ducks will speak Modbus RTU? The register map
      serves all three; only the tunnel would add a component.
- [ ] Does the management server exist yet, and will it be the only
      MQTT publisher to `cmd`? Decides broker ACL conventions and the
      `site` level in the topic tree.

**Behavior definers**
- [ ] Network-loss policy beyond rules: should "broker unreachable for
      N minutes" have its own default action (spec: none, rules can
      express it)?
- [ ] `on_all` order and stagger at controller boot when policies say
      `on`: node order 1-8 at 5 s (spec default) or something slower for
      the 19 V bus inrush?
- [ ] Should `/id` on plain HTTP be allowed (it leaks MAC and firmware
      version to the LAN)? Spec: yes, it is the recovery path.
- [ ] Probe names: by ROM id mapping in config (spec) or by connector
      position? The bus cannot tell position; mapping needs a one-time
      "touch a probe" identification step on the page.

---

## What was built differently (stage 1, 2026-10-10)

Reversible deviations taken during implementation, folded in here so the
ADR keeps describing the built thing.

granite-core (commit 49fb6fe):
- The actuator calls `release_all` after every press, so GPIO10 is low
  between the presses of a stagger instead of staying high for the whole
  queue. One expander reconfiguration per press, strictly safer.
- Validation ranges added: t_on 1-300 s, t_soft_off 5-900 s, t_cycle
  1-300 s, t_stagger and t_settle 0-60 s.
- Rule thresholds are in native units (centi-degrees, mV, 0/1, s).
- Messages: an `accepted` event, a `data` field on Reply (config_get),
  and a `v` field on status, state and event payloads.
- Modbus: input register 14 = map version. Modbus cannot carry `force`,
  so actions on a node in state Unknown are refused there.
- `cycle` skips t_cycle when the node is already off. With
  `sense: ignore`, `force_off` presses PWR for the full t_hold.

granite-fw skeleton (commit b38c30e, firmware/README.md has the detail):
- esp-idf-svc 0.53 / esp-idf-hal 0.47 / ESP-IDF v5.5.5, nightly +
  build-std; the crate sits outside the firmware/ workspace because of
  its target config.
- The partition table is applied by espflash (`--partition-table`), not
  by the ESP-IDF build: esp-idf-sys cannot point the build at a custom
  CSV. The bench flash also passes `--bootloader` with the ESP-IDF-built
  bootloader, because espflash's own bootloader has rollback disabled.
- App signing lives in `sdkconfig.defaults.signing`, opt-in via
  `ESP_IDF_SDKCONFIG_DEFAULTS`, since the build fails without a key.
- `CONFIG_ESP_WIFI_ENABLED` is hidden in v5.5 and cannot be set to n;
  the radio is simply never initialised. BT is off.
- Dropping the SPI driver after a failed W5500 init panics inside
  esp-idf-hal; the driver is leaked on purpose (main.rs comment).

granite-fw hardware layer (commit 62ee599):
- GPIO10 (EXP_nRESET_INT) resets U14 AND U15, so "GPIO10 low whenever no
  press is in progress" would also hold the sense expander in reset:
  no EXP_INT, no LED read-back right after a press, dry contacts shorter
  than the poll missed. Decision: the line stays high while idle
  (`expanders::IDLE_IN_RESET = false`). Idle safety rests on U14 OLAT = 0
  verified every second against the shadow, and on the press deadline,
  which still resets both expanders mid-press. Boot, panic hook and
  the deadline are the three paths that drive GPIO10 low.
- Both I2C buses use the ESP-IDF v5 i2c_master driver; the external bus
  is the hardware LP I2C (LP_I2C_NUM_0, GPIO6/7, 100 kHz), which closes
  docs/controller.md open item 3. `CONFIG_I2C_SKIP_LEGACY_CONFLICT_CHECK`
  is required because esp-idf-hal links the legacy driver.
- 1-wire through the espressif/onewire_bus component (RMT) with DS18B20
  commands in-tree; one CONVERT_T broadcast per 2 s cycle shared by all
  probes.

granite-fw platform layer (commit 0017b32):
- Device certificate: mbedTLS x509write through esp-idf-sys, generated on
  first boot in about 40 ms, stored in the `secrets` namespace.
- SNTP uses `esp_netif_sntp` directly (DHCP option 42 support); AutoIP
  is lwIP's own (CONFIG_LWIP_AUTOIP_TRIES = 4 gives the 30 s).
- `lwIP must be initialised before the HTTPS server even when Ethernet
  failed` (NetifStack::initialize early), otherwise the server aborts.
- The dispatcher keeps its own Config copy and only uses try_read /
  try_write: the API context holds the config write guard across a
  dispatch, so a blocking lock would deadlock. The MQTT section has its
  own mutex for the same reason.
- OTA writer is on raw esp_ota_* calls (EspOtaUpdate's borrow cannot
  live inside an Arc<Mutex<dyn OtaSink>>).
- Not built yet, marked TODO(ADR 0001 6) in the code: VLAN tagging, the
  dead-man keeping the static address while DHCP runs alongside, live
  apply of staged mqtt/sec sections.
