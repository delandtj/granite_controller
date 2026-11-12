# Granite controller firmware

Rust on ESP-IDF for the rev C controller board: an ESP32-C6-WROOM-1-N8
(RISC-V, 8 MB flash) with a WIZnet W5500 on SPI2, two MCP23017 expanders
that press PWR/RST on 8 nodes and read their power LEDs, DS18B20 probes,
a TMP1075, a VIN divider and an external I2C bus on J8/J9.

Read `docs/adr/0001-firmware-architecture.md` for the architecture and
the reasoning, `../docs/controller.md` for the hardware and the pin map,
`docs/modbus_map.md` for the register map.

What is in the image today: the hardware layer (expanders with a
hardware press deadline, sense, probes, board temperature, VIN, LED, both
I2C buses), the platform layer (NVS config, identity and recovery,
W5500 networking with commit-confirm, SNTP, mDNS, USB console, OTA with
probation, a 16 KB log ring), the node state machine, actuator queue and
rule engine from `granite-core`, and all three northbound interfaces:
HTTPS setup page plus `/api/v1`, an MQTT client, and a Modbus TCP
server. MQTT and Modbus are off in the default configuration.

Nothing above the hardware layer has been exercised on a rev C board
yet: the boards are on order. See "Testing on a devboard".

## Crate layout

```
firmware/
  Cargo.toml          workspace: granite-core + granite-sim
  granite-core/       no ESP-IDF, host-tested: hal traits, node state
                      machine, actuator, rules, config, messages,
                      dispatcher, the HTTP API handlers, the Modbus
                      frame handler and the register map, the block
                      switch's model, parsers, forms and planner
  granite-fw/         the ESP-IDF binary for the board. A standalone
                      cargo crate, deliberately NOT a workspace member:
                      it has its own target, toolchain and linker
    src/hw/           hardware layer (MCP23017, relays, sense, probes,
                      TMP1075, VIN, LED, I2C)
    src/platform/     NVS store, identity, net, console, OTA, log ring,
                      the API trait implementations, wifi_dev
    src/http.rs       HTTPS server and the API glue
    src/mqtt.rs       the broker session
    src/modbus.rs     the Modbus TCP listener
    src/switch/       the block switch (ADR 0006): the HTTP client and
                      the worker thread. The model, the page parsers,
                      the form builders and the reconcile planner are
                      host-tested in granite-core/src/switch
    src/main.rs       boot order and the integration of all of it
  granite-sim/        host binary: granite-core over fake hardware with
                      the real setup page, for UI work and integration
                      tests
  web/                the setup page (plain HTML/CSS/JS, gzipped into
                      the image by build.sh)
  docs/adr/           architecture decisions
  docs/modbus_map.md  generated from granite-core
```

Every component in `granite-core` is host-tested
(`cargo test` in `firmware/`); `granite-fw` is glue and is tested on
hardware.

## Versions

| Component | Version |
|---|---|
| esp-idf-svc | 0.53.0 |
| esp-idf-hal | 0.47.0 |
| esp-idf-sys | 0.38.1 |
| embuild (build dep) | 0.33.5 |
| ESP-IDF | v5.5.5 (the default of esp-idf-sys 0.38.1, pinned in `granite-fw/.cargo/config.toml`) |
| Rust | nightly with `build-std` (see `granite-fw/rust-toolchain.toml`) |
| Target | `riscv32imac-esp-espidf` |

The ESP32-C6 is RISC-V, so the build uses upstream **nightly** plus
`build-std`, not the Xtensa-only `esp` toolchain from espup.

Two ESP-IDF managed components are pulled in from
`granite-fw/Cargo.toml`: `espressif/onewire_bus` (the RMT-based 1-wire
driver the DS18B20 code sits on) and `espressif/mdns`.

## Toolchain install

```sh
# Rust nightly with the standard library sources (build-std).
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly

# Linker wrapper and flash tool.
cargo install ldproxy espflash     # or use your distribution's packages

# Native build tools the ESP-IDF CMake build needs.
#   cmake, ninja, python3, git
# Arch: pacman -S cmake ninja python git
```

The ESP-IDF checkout and its toolchains are downloaded on the first
build by `esp-idf-sys` into `~/.espressif` (sources under
`~/.espressif/esp-idf-<hash>`), with per-crate build state in
`granite-fw/.embuild/`. Nothing of that lands in the repo; `.embuild/`
and `target/` are gitignored. The first build takes a while (ESP-IDF
download plus a full ESP-IDF compile); later builds are quick.

## Build

```sh
cd firmware/granite-fw
cargo build --release                 # the FD image: link-canfd
cargo clippy --release --all-targets -- -D warnings
```

### One link per image

ADR 0006 (architecture component 7) gives each image exactly one plane
link, picked by a cargo feature. Exactly one of the two must be set and
`src/lib.rs` refuses the build with a `compile_error!` otherwise - both
set or neither set is a mistake, never a default.

| feature | board | link |
|---|---|---|
| `link-canfd` (**default**) | controller-fd | MCP2518FD on the W5500's SPI bus, CAN FD transport of `docs/adr/0005` |
| `link-i2c` | rev D | isolated I2C on GPIO2 / GPIO3, EXP_INT GPIO0; still the NAK stub |

```sh
cargo build --release                                            # canfd
cargo build --release --no-default-features --features link-i2c  # rev D
cargo build --release --no-default-features \
    --features link-i2c,devboard                                 # rev D on a devkit
```

The feature decides what three pins are, because the two boards wire
them differently (`src/hw/mod.rs`, "The two links, and the pins they
take"):

| pin | `link-i2c` | `link-canfd` |
|---|---|---|
| GPIO0 | EXP_INT (planes' wired-OR interrupt) | TCAN1044V STB, driven low for normal mode |
| GPIO2 | plane link SDA (HP I2C) | MCP2518FD nCS |
| GPIO3 | plane link SCL (HP I2C) | MCP2518FD nINT |
| GPIO6 / 7 | local I2C (LP): J9 | local I2C (LP): J9 **and the TMP1075** |

controller-fd has no HP I2C bus at all - its TMP1075 sits on the local
bus with J9 - so the FD build never opens `I2C_NUM_0` and reads the
board temperature over LP I2C. The MCP2518FD is a second device on the
W5500's bus (`platform::net::spi_bus` builds the bus before either
device): its own chip select, its own 10 MHz clock, below the ~17 MHz
erratum ceiling at a 40 MHz SYSCLK; the W5500 keeps its 20 MHz.

The link-related features in full:

- `link-canfd`, `link-i2c`: above, exactly one.
- `link-bench`: a link that does not come up is logged instead of
  keeping the image from confirming itself (see "probation" below). For
  the rev C bench board, which has no MCP2518FD on it. In `devboard`.
  **Never in a shipped image.**

`link` on the console prints the link kind and, on an FD build, the
transport's counters: frames in and out, blocked sends, timeouts,
EVENTs, reassembly drops by reason, TEC / REC and the bus-off count
(ADR 0005 decision 8). `status` carries one summary line.

Nothing of the CAN link has run on hardware: no controller-fd board
exists. The transport itself is host-tested
(`granite-core/src/plane/can_link.rs`, `cargo test -p granite-core`).

`MCU=esp32c6`, `ESP_IDF_VERSION=v5.5.5` and `CARGO_WORKSPACE_DIR` come
from `.cargo/config.toml`, so no environment setup is needed. The last
one matters: `esp-idf-sys` looks for `sdkconfig.defaults` relative to
the cargo workspace directory, which embuild derives from the target
directory - with a `CARGO_TARGET_DIR` pointing outside the crate that
guess lands in the wrong place and the configuration is silently
ignored.

The build output is `target/riscv32imac-esp-espidf/release/granite-fw`
(an ELF; espflash turns it into an image while flashing). If you have
`CARGO_TARGET_DIR` set in your environment, the binary is under that
directory instead - the flash commands below assume the default, so
substitute the path.

Image size, release, default features (`link-canfd`, 2026-11-12):
1,868,800 bytes of the 3,997,696-byte slot, 46.7 % - the block switch
(ADR 0006: the scanner, the page parsers, the form builders, the
planner, the client and the worker, plus the `switch` config section
and its serde) is +159,856 bytes on the 1,706,432 of 2026-11-06, which
is more than the 30-80 KB the ADR guessed at, and `config_confirm` /
`config_revert` are the 2,512 bytes after that. Before all of it, the CAN
transport, `mcp251xfd` and the link marker were +51,616 bytes on the
1,654,816 of 2026-11-04. `cargo build --release` keeps
`opt-level = "s"` and `lto = true`; do not switch the release profile to
`3` without checking that number again. The number comes from

```sh
espflash save-image --chip esp32c6 --flash-size 8mb \
    --partition-table partitions.csv \
    target/riscv32imac-esp-espidf/release/granite-fw /tmp/granite-fw.bin
```

which prints `App/part. size` and the percentage.

The host crates build and test normally:

```sh
cd firmware
cargo test                     # granite-core and granite-sim
```

## Flash and monitor

The USB-C connector is the ESP32-C6's **USB Serial/JTAG** peripheral,
and the console is on it (`CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y`);
UART0 is left to the 1-wire probe bus. It usually shows up as
`/dev/ttyACM0`.

```sh
cd firmware/granite-fw
espflash flash --monitor --port /dev/ttyACM0 \
    --bootloader target/riscv32imac-esp-espidf/release/build/bootloader.bin \
    --partition-table partitions.csv \
    --target-app-partition ota_0 \
    --erase-parts otadata \
    target/riscv32imac-esp-espidf/release/granite-fw
# monitor only (the --elf gives symbol names in a panic backtrace):
espflash monitor --port /dev/ttyACM0 \
    --elf target/riscv32imac-esp-espidf/release/granite-fw
```

That is the whole bench flash: one invocation, no second pass. All four
extra flags matter.

- Without `--bootloader`, espflash flashes the bootloader it bundles
  itself (an ESP-IDF v6.1-beta build at espflash 4.6.0) instead of the
  one the ESP-IDF build produced from `sdkconfig.defaults`, so
  `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` and the signature check would
  not be the ones in the bootloader on the chip.
- Without `--partition-table`, the default single-app table is flashed
  and there are no OTA slots.
- `--target-app-partition ota_0` is now required: there is no `factory`
  partition, so with no label espflash has no default app slot to write.
- `--erase-parts otadata` makes the bootloader pick `ota_0` on the next
  boot. On a board with OTA history (old layout or new), `otadata`
  holds a sequence number that can select `ota_1`, so the bootloader
  would otherwise boot that stale slot, not the image just flashed.

`nvs` is never written by `espflash flash` and it sits at the same
offset in the old and the new table, so the configuration, the device
key and certificate and the recovery token survive a reflash - including
the reflash that moves an existing bench board from the old layout to
this one. That is the same wired flash as above; nothing else is needed.
Erase NVS on purpose with `espflash erase-parts nvs` or with the
console's `factory-reset CONFIRM` (which keeps the `factory` namespace:
the recovery token, the fleet key and the device key and certificate -
an NVS namespace that happens to share the name of the dropped
partition and has nothing to do with it).

If the board does not enter download mode by itself, pull the BOOT test
pad (GPIO9) to GND while powering up.

## Partition layout

`granite-fw/partitions.csv`, 8 MB flash, table at 0x8000:

| Name | Type | SubType | Offset | Size |
|---|---|---|---|---|
| nvs | data | nvs | 0x9000 | 192 K |
| otadata | data | ota | 0x39000 | 8 K |
| coredump | data | coredump | 0x3B000 | 64 K |
| ota_0 | app | ota_0 | 0x50000 | 0x3D0000 (3,997,696 B) |
| ota_1 | app | ota_1 | 0x420000 | 0x3D0000 (3,997,696 B) |

Last byte used is 0x7F0000, so 64 K of the 8 MB stays free. App
partitions need 64 K alignment, which leaves a 20 K hole between
`coredump` (ends 0x4B000) and `ota_0` (starts 0x50000).

There is no `factory` partition (user decision, 2026-11-04): its 2.5 MB
went to the two OTA slots instead. With bootloader app rollback an
update never overwrites the slot it is running from, so one OTA slot
always holds the last image that passed self-validation and two bad
updates in a row cannot brick the board. `factory` only covered both
OTA images being invalid at the same time, and it did not help with an
image that validates but is subtly broken. The cost: if both OTA slots
are invalid the board needs USB.

The bench image goes to `ota_0` (see the flash command above); OTA then
alternates `ota_0`/`ota_1`.

`esp-idf-sys` cannot hand the ESP-IDF build a partition CSV that lives
outside its own generated project directory (its README, "Known
limitations", says not to set `CONFIG_PARTITION_TABLE_CUSTOM` at all),
so the table is applied **at flash time**: always pass
`--partition-table partitions.csv` to `espflash`. `sdkconfig.defaults`
selects the large single-app table instead, purely so the build-time
image size check has 1.5 MB of headroom; that choice never reaches the
chip.

## Boot order

`main.rs` fixes it and the comments there say why no line may move:

1. `link_patches`, then the log ring in front of the ESP-IDF logger, so
   the first boot lines are already in `log tail`.
2. **GPIO10 (EXP_nRESET_INT) and GPIO4 low before any other GPIO.**
   While GPIO10 is low every photoMOS output is open, whatever the
   firmware does next. The pin has a pull-down, so a chip reset does the
   same thing.
3. Platform layer: NVS, config, identity (recovery token and device
   certificate on a first boot), OTA state and probation, the network
   thread.
4. Hardware layer. A layer that does not come up is logged, not fatal:
   the console and the network must stay reachable on a board whose I2C
   bus is dead, and the dispatcher then runs on a `NodeSwitches` that
   refuses every press honestly.
5. Console, dispatcher thread, sense thread, LED thread.
6. HTTPS and `/api/v1`, the MQTT worker, the Modbus supervisor, the
   config watcher.
7. One `boot complete: free heap ...` line, then the 10 s heartbeat.

## First boot

On a board with an empty `nvs` the firmware

1. writes the default configuration (DHCP, MQTT off, Modbus off, boot
   policy `leave`, the example rules present and disabled),
2. generates the per-device recovery token (20 random bytes, base32,
   grouped) into the `factory` namespace and prints it **once**,
3. generates the HTTPS device certificate (ECDSA P-256, self-signed,
   CN = the device id, `notBefore` = the firmware build time, 10 years)
   with mbedTLS and prints its SHA-256 fingerprint.

Both are reloaded on every later boot; the token is printed again by the
`recovery-token` console command, the fingerprint by `id`. The device
key and certificate live in the `factory` namespace next to the recovery
token, so a factory reset keeps the fingerprint and anything pinning it
keeps working. A board written by a firmware older than 2026-10-10 has
them in `secrets`; the next boot moves them over and logs one line.
A factory reset does clear the "recovery token already shown" flag, so
the first setup after a reset shows the token again (it is the same
token).

The identity is the eFuse MAC: device id, hostname, mDNS name and the
default MQTT client id are all `granite-<last 6 hex of the Ethernet
MAC>`. **Write the MAC and the recovery token down together before the
board is sealed** - that pair is the whole recovery path for a submerged
controller.

Then, from a browser on the same network:

1. `http://granite-xxxxxx.local/id` or `https://<ip>/id` (both
   unauthenticated, read only) confirms which board you are talking to
   and prints the certificate fingerprint to compare against the console.
2. The page refuses everything until the admin password is set - there
   is no default password. Set it on the first-setup page, or over USB
   with `set-password <pw>`.
3. The Security page shows the recovery token once. The same page takes
   the fleet recovery public key.

An optional fleet recovery public key (ECDSA P-256, PEM) can be baked
into the image:

```sh
FLEET_RECOVERY_PUBKEY="$(cat fleet-recovery.pub)" cargo build --release
```

A freshly flashed board then already trusts it; otherwise it is set at
commissioning. The private key never touches a board; `granite-sim
recover` is the host tool that uses it.

## Console

The USB-C connector carries an interactive console
(`CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y`, driven through the
`usb_serial_jtag` driver rather than through `stdin`). Physical access
is full access; the console asks for no password and is not a network
path.

```
granite> help
  id                    device id, mac, firmware, cert fingerprint
  status                net, ota slot and state, uptime, free heap
  net                   network detail
  set-password <pw>     set the admin password (first setup or reset)
  recovery-token        print the per-device recovery token again
  fleet-key             fleet recovery key fingerprint
  factory-reset CONFIRM erase config and secrets, keep factory, reboot
  ota-mark-valid        confirm the running image
  reboot                restart through the planned-reboot path
  log [n]               last n lines of the log ring (default 40)
  stacks                per-task stack headroom, worst first
  help                  this list
```

`stacks` lists every FreeRTOS task with its stack high-water mark in
bytes (ESP-IDF reports bytes, not the words upstream FreeRTOS
documents), least headroom first, plus the size and the percentage used
where the size is known - the threads the firmware spawns through
`platform::spawn` and the ESP-IDF main task. The same block is logged
once at info level 120 s after boot. It needs
`CONFIG_FREERTOS_USE_TRACE_FACILITY=y`, and it is how to check a stack
budget under `lto = true`: LTO inlines large constructors into caller
frames, which is what overflowed the dispatcher's old 12 KB stack.

`espflash monitor` does not forward keystrokes in non-interactive mode;
use it interactively, or any serial terminal (`picocom /dev/ttyACM0`).

## HTTPS page and API

HTTPS on 443 with the device certificate; plain HTTP on 80 serves only
`/id` and a redirect. One admin password (PBKDF2-HMAC-SHA256, 20k
iterations, per-device salt), a session cookie for the page and
`Authorization: Bearer <token>` for scripts, 5 failed logins then a 60 s
lockout. The salt is minted once and then kept: API tokens are hashed
with it too, so changing the password - over the API or with the
console's `set-password` - leaves existing tokens working. Tokens are
32 random bytes and stored as `s256:` + SHA-256(salt || token): a slow
KDF adds nothing for them and cost ~1.7 s per request per token. A
token stored by older firmware (plain PBKDF2 hex) still works and is
rehashed on first use. The password hash pauses one tick every 500
rounds (`api::set_kdf_pause`), so queued logins cannot starve the idle
task into a task-watchdog reset.

`GET`, `HEAD`, `POST`, `PUT` and `DELETE` are registered; `HEAD` answers
every `GET` route, for monitoring tools that probe with it.

`/api/v1/...` carries the same command vocabulary as MQTT, because both
go through one dispatcher in `granite-core`: status, config sections
(with staging and confirm for anything that can cut the session), node
actions, rules, firmware upload and rollback, log tail, export/import,
factory reset, recovery, 1-wire probe re-scan.

A node action answers `accepted` at queue time and the outcome lands in
`state.nodes[i].last_action` and on the MQTT `ack` topic.
`POST /api/v1/nodes/<n>/<action>?wait=1` holds the HTTP reply until that
command id finishes and answers with the outcome instead (400 with the
error when it failed). Two caveats, both in `http.rs`
(`ChannelDispatcher::wait_for`, `WAIT_CAP`): the wait is capped at 30 s
(`api::ACTION_WAIT_MS`) and then falls back to the `accepted` reply, and
ESP-IDF's HTTP server runs one task here, so a waiting request answers
nothing else - the setup page included - until the action is done. It is
a convenience for scripts, not the way the page works. The wait itself
holds no lock: `wait_for` records the command id and returns, the
handler drops every platform guard (the configuration write guard
included, so the MQTT worker and the sense task keep reading it), and
only then reads the same event bus the MQTT ack is built from.

The page itself is plain
HTML/CSS/JS from `firmware/web/`, gzipped into the image by
`web/build.sh`; `granite-sim serve --assets ../web` serves it from disk
for editing.

## MQTT

Operator guide (topics, commands, config sections, recipes): [`docs/mqtt.md`](docs/mqtt.md).

Off by default. Configured on the MQTT page (or by `config_set`): host,
port (8883), TLS with a CA in PEM, username plus password or a client
certificate, QoS, keepalive, `t_state_s`, `site`.

Topic root `<topic_root>/<site>/<device>`, default
`granite/default/granite-xxxxxx`:

| Topic | Dir | Retained | Payload |
|---|---|---|---|
| `.../status` | out | yes | online, fw, ip, uptime, boot reason; the LWT writes `{"online":false}` |
| `.../state` | out | yes | full snapshot, on change (coalesced 200 ms) and every `t_state_s` |
| `.../event` | out | no | node_state, action_done, action_failed, rule_fired, fault, ota, config |
| `.../log` | out | no | one JSON line per log record at or above `sys.log_level` |
| `.../cmd` | in | - | `{"id":..,"action":..,"target":..,"args":{..}}` |
| `.../ack/<id>` | out | no | published when the action *completes* |
| `.../switch/counters` | out | no | per-port totals plus pps and bit/s, every `switch.poll.counters_s` (ADR 0006) |

A commit-confirm cycle can be finished over MQTT: `config_set` stages a
section that affects reachability, `config_confirm` promotes every
staged section (the same promote the setup page's confirm does, acked
with the section list in `data`) and `config_revert` puts them back
without waiting out `net.t_confirm_s`. Both answer `nothing is staged`
when nothing is.

ADR 0001 component 7 has the authoritative table and the command list;
`granite-core/src/msg.rs` is the implementation and its tests are the
payload fixtures. The worker reconnects on an exponential ladder (1 s to
60 s) and keeps publishing `mqtt_connected` into the snapshot the rule
engine reads, so rules that watch the broker work.

The worker thread runs whatever `mqtt.enabled` says: with no broker to
talk to it idles on the configuration, and it rebuilds its client as
soon as the live `mqtt` section stops matching the one it connected
with. Changing or enabling a broker therefore needs no reboot, and a
staged section gets a real connection attempt inside its confirm window
(ADR component 6). `mqtt.auto_confirm_on_connect` promotes the staged
section the moment that attempt succeeds; otherwise the config-watch
thread puts the stored section back when the window passes.

## Modbus TCP

Off by default, and it refuses to listen without an IP/CIDR allow-list:
Modbus has no authentication. Port, unit id, maximum connections and the
allow-list are the `sec.modbus` section; enabling it (or editing the
allow-list) takes effect within about 5 s, no reboot.

The register map is `docs/modbus_map.md`, generated from
`granite-core/src/modbus_map.rs` with `cargo run -p granite-sim --
modbus-map`. It is versioned (input register 14) because the RS-485 duck
protocol will reuse the same semantics.

## The block switch

Operator guide, with every key, the confirm cycle, recipes and
troubleshooting: [`docs/switch.md`](docs/switch.md).

Off by default, and **read-only** when it is on. The `switch` section
(ADR 0006) says where the switch is, whether the controller may write
to it and what it should look like:

```json
{"enabled": true, "manage": false, "host": "10.0.202.5",
 "mac": "1c:2a:a3:24:26:7e", "user": "admin", "uplink_port": 9,
 "enforce": false,
 "poll": {"stats_s": 5, "info_s": 60, "mac_s": 120, "drift_s": 600,
          "counters_s": 60},
 "desired": {"vlans": [{"id": 300, "name": "storage"}],
             "ports": [{"port": 7, "mode": "access", "vlan": 300,
                        "node": 7}],
             "jumbo": 9216, "loop": "loop",
             "ip": {"mode": "static", "addr": "10.0.202.5/24",
                    "gw": "10.0.202.1"}}}
```

The switch is a Sodola SL902-SWTGW218AS with a CGI web UI and nothing
else; `Granite/infra` has the protocol notes (`docs/sodola.md`) and a
reference client (`tools/sodola.py`). One worker thread owns every byte
that goes to it. With `manage` false it polls and publishes and the
only POSTs it sends are the login and the MAC table's paging; with
`manage` true `desired` is applied through the same commit-confirm path
as `net` and `mqtt`:

- a `config_set` of `switch` is **staged**: the worker reads the
  switch, plans the ops (create VLANs, port VLANs, delete VLANs,
  jumbo/storm/loop, the address last) and applies them **without
  saving**, keeping a snapshot of what it read,
- a confirm (`POST /api/v1/config/confirm`, or the `config_confirm`
  command) is the only thing that calls `save.cgi`, so the switch's
  flash always holds the last confirmed configuration,
- no confirm inside `net.t_confirm_s` - or an explicit
  `config_revert` - and the worker plans from the switch's current
  state back to its snapshot and applies that; if the switch is gone
  or the undo does not verify, it reboots the switch, which drops
  everything unsaved (about 3 s of downtime for every port).

The uplink port and the port the controller's own MAC is seen on are
protected: a plan that would re-VLAN or disable one is refused unless
the command says `"force": true`, and while the controller cannot find
its own MAC in the table *every* port counts as protected. Drift from
`desired` is reported as a `switch_drift` event and only put back when
`enforce` is true.

Commands: `switch_port` (`{"port":N}` or `{"node":N}` plus
`"enabled":bool`, optional `"force":true`), `switch_reconcile`,
`switch_save`, `switch_reboot` and `switch_factory_reset` (both need
`"confirm":"<device id>"`). Events are `kind:"switch"` with a `what`:
`switch_port`, `switch_reachable`, `switch_unreachable`,
`switch_reboot`, `switch_drift`, `switch_reconciled`,
`switch_reverted`, `switch_refused`, `switch_mismatch`,
`switch_fw_unknown`, `switch_saved`. `state.switch` carries the
summary; the counters have their own topic.

Parsers are written against firmware V200.1.8 and tested against pages
captured from a real unit
(`granite-core/tests/fixtures/sodola-v200.1.8/`). Another version is
monitored while its pages still parse, and never written
(`switch_fw_unknown`); a switch whose MAC is not `switch.mac` is not
talked to beyond `info.cgi` (`switch_mismatch`). Both set Modbus fault
bit 9; three failed polls in a row set bit 8 and
`state.switch.reachable` to false.

## OTA, signing and probation

Two OTA slots of 0x3D0000 bytes each and no `factory` partition,
ESP-IDF `esp_ota_*`, rollback enabled in the bootloader.

- **Push**: the Firmware page uploads a `.bin` over HTTPS
  (`POST /api/v1/firmware/upload`, authenticated, streamed into the slot
  in 2 KB chunks so the whole image never sits in RAM).
- **Pull**: the `ota` command over MQTT (or the API) takes
  `{"url":..,"sha256":..}` and the controller fetches it over HTTPS
  itself, using the MQTT CA.
- **Progress**: both paths publish `ota` events on the event topic:
  `started` (progress 0), `downloading` every 10 % of a known
  Content-Length or every 256 KiB without one, then `verified`, and
  after the reboot `pending_verify` and `valid` (or `failed` /
  `rolled_back`).

### Making the `.bin`

The cargo build produces an ELF, not an image: the application `.bin`
the two paths above want is made from it with espflash. The ESP-IDF
build under the target directory does **not** contain one - it only
produces `build/bootloader.bin` and `build/partition-table.bin`,
because the app itself is linked by cargo, not by the ESP-IDF project.
There is a `libespidf.bin` under
`build/esp-idf-sys-*/out/build/`; it is the esp-idf-sys stub project
(about 150 KB, no Rust code in it) and must never be pushed as an
image.

```sh
cd firmware/granite-fw
cargo build --release
espflash save-image --chip esp32c6 \
    target/riscv32imac-esp-espidf/release/granite-fw granite-fw.bin
sha256sum granite-fw.bin      # the sha256 the pull command needs
```

Verified with espflash 4.6.0 on 2026-10-10: 1,719,488 bytes for the
default release build, and `esptool.py --chip esp32c6 image_info
granite-fw.bin` reports a valid checksum and validation hash. Do not
pass `--merge`: that bundles the bootloader and the partition table and
pads to the flash size, which is a full-flash image, not an OTA one.

For a build with signing on (below), the image needs the signature
block appended afterwards, because espflash writes the plain image and
the signing happens in the ESP-IDF build, which never sees this app:

```sh
espsecure.py sign_data --version 2 --keyfile signing_key.pem \
    --output granite-fw-signed.bin granite-fw.bin
espsecure.py verify_signature --version 2 --keyfile signing_key.pem \
    granite-fw-signed.bin
sha256sum granite-fw-signed.bin
```

`sign_data` pads the image to the next 4 KB boundary itself ("Padding
data contents by N bytes so signature sector aligns at sector
boundary") and appends a 4 KB signature sector, so no `--secure-pad-v2`
step is needed. Verified on 2026-10-10 with esptool/espsecure 5.3.1
against an unsigned release image: sign, then `verify_signature`
reports "Signature block 0 verification successful", and `image_info`
still reports a valid image. ESP-IDF v5.5.5 ships its own
`espsecure.py` in `.embuild/espressif/python_env/*/bin/`, which takes
the same `sign_data` / `verify_signature` subcommands; standalone
esptool 5.x prefers the dashed spelling (`sign-data`) and warns about
the underscore one.

**Unverified**: that a board built with `sdkconfig.defaults.signing`
accepts such an image over OTA, and whether the bootloader the signing
build produces is itself signed. That needs a signing build (a full
ESP-IDF reconfigure) and a bench board; nothing here has been pushed to
a board yet.
- **Link kind**: the image carries which plane link it drives, and an
  upload of an image for the other one is refused with HTTP 400 and an
  `ota failed` event, like a hash mismatch (ADR 0006 component 7). The
  marker is a 24-byte tagged struct in the image's read-only data
  (`granite-core/src/link_kind.rs`: a 16-byte magic, a format version,
  the kind, its complement, a CRC-32), and the upload path searches the
  stream it is writing for it. **An image with no marker is refused
  too** - an image from before the links split is exactly the case the
  guard exists for. That means the first FD image cannot be installed
  over an older build *from the network*: the board takes it over USB
  once, and every image after that over OTA. `esp_app_desc_t` was the
  alternative and is not usable: its `version` is pinned to the crate
  version by `build.rs` and comes from a file the ESP-IDF build reads
  before cargo features exist, and `project_name` has no sdkconfig
  symbol at all.
- **Probation**: a new image boots `pending_verify` and must, within
  `sys.t_validate_s` (default 10 min), have (a) brought its plane link
  hardware up, (b) got link and an IP, and (c) either connected to the
  configured broker or served an authenticated HTTPS request. Then it
  marks itself valid; otherwise it marks itself invalid and reboots, and
  the bootloader falls back to the other slot - which, with no
  `factory` partition, is the last resort: if it is invalid too, the
  board needs USB.
  A step nobody can satisfy is not required: with MQTT disabled the
  broker step is not armed, so an authenticated HTTPS request is what
  confirms such an image. The LED blinks at 4 Hz while an image is on
  probation, and `ota-mark-valid` on the console confirms it by hand.
  The hardware backs this up: the bootloader's RTC watchdog stays on
  into the app (`CONFIG_BOOTLOADER_WDT_DISABLE_IN_USER_CODE`). A
  confirmed image switches it off at the top of `main`; an image on
  probation keeps it at 60 s for early boot, then `t_validate_s` + 60 s,
  and switches it off when it confirms. An image that hangs before it
  can roll itself back is reset by the watchdog, and the bootloader
  rolls it back. Any other binary built with this sdkconfig must call
  `platform::ota::rtc_watchdog_at_boot()` first (the hwtest example
  does) or it resets after 9 s.

App signing (ECDSA P-256, OTA signature verification, no Secure Boot and
no eFuse burning) is an **opt-in**: the options live in
`granite-fw/sdkconfig.defaults.signing`, not in `sdkconfig.defaults`,
because the ESP-IDF build fails when signing is on and no private key
exists. Generate a key outside the repo or in the crate root (`*.pem` is
gitignored, and the key belongs in your secret store, not in git):

```sh
# espsecure.py ships with ESP-IDF (~/.espressif/python_env/.../bin)
espsecure.py generate_signing_key --version 2 --scheme ecdsa256 \
    firmware/granite-fw/signing_key.pem
```

Then build with signing enabled:

```sh
cd firmware/granite-fw
env ESP_IDF_SDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.defaults.signing" \
    cargo build --release
```

`CONFIG_SECURE_BOOT_SIGNING_KEY` in that file points at
`signing_key.pem` relative to the crate root. Losing the private key
means no more OTA for boards built with its public key, and a submerged
board cannot be reflashed over USB - keep it backed up.

How signing surfaces at runtime: the console's `status` and the status
page print a `signing` line. On a signed build it reads `signed images
required, key id <8 hex>`, where the key id is the first digits of the
running app's ELF SHA-256 (`esp_app_get_elf_sha256`) - the same value
`espflash` prints as `ELF file SHA256` at boot, so an operator can match
a running image against the one that was pushed. On an unsigned build it
says so plainly. There is no eFuse involved either way: Secure Boot V2
and flash encryption stay off (ADR 0001 component 11).

## Network behaviour

- **AutoIP** is lwIP's, not the firmware's: `CONFIG_LWIP_AUTOIP=y` plus
  `CONFIG_LWIP_AUTOIP_TRIES=4` makes lwIP add an IPv4 link-local address
  after four failed DHCP DISCOVERs while DHCP keeps retrying. lwIP
  doubles the DISCOVER timeout, so four tries is 2 + 4 + 8 + 16 = 30 s,
  the window ADR 0001 component 6 asks for. The firmware reports the
  result (`mode autoip` in `net`) by recognising 169.254/16.
- **SNTP** uses `esp_netif_sntp_*` rather than esp-idf-svc's `EspSntp`,
  because only that API can take the server from DHCP option 42
  (`CONFIG_LWIP_DHCP_GET_NTP_SRV=y`). Until the first sync the clock is
  set to the firmware build time, so TLS not-before checks pass.
- **mDNS** comes from the external `espressif/mdns` component. It
  advertises `_https._tcp` and `_granite._tcp` on 443 with
  `id=<device id>`.
- **Commit-confirm**: a staged `net` section is applied live while the
  old value stays in `cfg`, so a reboot reverts by doing nothing. If the
  confirm does not arrive within `t_confirm_s` the firmware reboots into
  the stored configuration.
- **Dead-man**: a static address with no gateway answer and no accepted
  connection for `t_deadman_s` (default 1 h, 0 = off) starts DHCP.

## Testing on a devboard

A bare ESP32-C6 devboard has no W5500 and no expanders. Three features
make it useful anyway, and `devboard` is the two network-and-LED ones
together; none of them belongs in an image that goes on a board.

**`hwtest`** - the hardware layer's bring-up exercise (ADR 0001,
"Bring-up order on hardware"), as an example binary rather than wired
into `main`:

```sh
cd firmware/granite-fw
cargo build --release --features hwtest --example hwtest
espflash flash --monitor --port /dev/ttyACM0 \
    target/riscv32imac-esp-espidf/release/examples/hwtest
```

It proves that `hw::init` on a board with nothing on I2C logs and
returns instead of panicking, that the press deadline really drops
GPIO10, that the probe bus enumerates to an empty list, and that VIN,
the board temperature, the external bus scan and the LED patterns all
answer.

**`wifi-dev`** - Wi-Fi station instead of the W5500, so HTTPS, MQTT,
Modbus and OTA can be exercised before boards arrive. The network thread
brings up the `sta_default` netif and everything above it (NetStatus,
hostname, mDNS, SNTP, commit-confirm, the dead-man) runs on it
unchanged. Credentials are build-time, not configuration: they belong to
the bench, not in a board's NVS, and a missing one has to break the
build rather than produce an image that cannot reach anything.

```sh
cd firmware/granite-fw
GRANITE_WIFI_SSID=bench GRANITE_WIFI_PASS=hunter2 \
    cargo build --release --features wifi-dev
```

Without the two variables the build stops with a message naming them.
Known limits, all deliberate: `esp_wifi` gets no NVS partition (the
store owns the one default partition handle), so RF calibration is
redone every boot; a dropped association is retried every 10 s with no
backoff and no roaming; WPA2-PSK or open only.

The default build has **no radio code in it**: `platform::wifi_dev` is
not compiled, the W5500 is the only network path, and ADR 0001's
"Wi-Fi and BLE stay off" still describes the shipped image.

**`rgb-led`** - the status LED patterns mirrored in colour on the
addressable WS2812 that an Espressif C6 devkit carries on GPIO8
(DevKitC-1 and DevKitM-1), *in addition to* the plain GPIO1 LED. On a
bench it is the only LED that says which pattern is running without
counting blinks:

| pattern | colour | rate |
|---|---|---|
| heartbeat, healthy | green, tinted towards cyan while the broker session is up | breathing, 1 Hz |
| no link | amber | 0.25 Hz |
| OTA image pending validation | blue | 4 Hz |
| press in progress | white | solid |
| expander or sense fault | red | 2 Hz |

```sh
cd firmware/granite-fw
cargo build --release --features rgb-led
GRANITE_RGB_GPIO=2 cargo build --release --features rgb-led   # other devkit
```

Brightness is capped at 40/255 on purpose. `GRANITE_RGB_GPIO` moves the
data line (the pin differs between devkit revisions); the default is 8,
and a value that is not a decimal pin number fails the build. The bit
stream is RMT, the same peripheral the DS18B20 probes use through the
Espressif `onewire_bus` component: both sit on the ESP-IDF v5 RMT
driver, so the IDF allocates the channels (the C6 has two TX channels
and 1-wire takes one TX and one RX) and the firmware still boots with
the probe bus up. Each pattern transition is logged once, as
`rgb: heartbeat (green, breathing 1 Hz)`.

The fault pattern is the one thing this feature adds above the LED: ADR
0001 fixes four meanings for the plain status LED and "a fault is
present" is not one of them, so `LedPattern::Fault` is only ever
selected in a build that has `rgb-led`. The signal is the fault word
Modbus input register 13 already reports (expander, sense).

Which means a **bare** devkit sits on red: it has no MCP23017s, so the
expander and sense bits are set a second after boot and stay set. That
is the honest reading, not a bug; green breathing needs expanders that
answer. The monitor says which pattern is on the LED either way.

**`devboard`** - `wifi-dev`, `rgb-led` and `link-bench`, i.e.
everything a bare devkit wants. `link-bench` is in there because a
devkit has no MCP2518FD, and without it every image would fail the
link-hardware step of its probation and roll back:

```sh
cd firmware/granite-fw
GRANITE_WIFI_SSID=bench GRANITE_WIFI_PASS=hunter2 \
    cargo build --release --features devboard
```

For reference, release image sizes on 2026-10-10 were 1,719,488 bytes
default, 1,730,400 bytes with `--features rgb-led`, 2,101,824 bytes
with `--features wifi-dev` and 2,112,528 bytes with
`--features devboard` (the two Wi-Fi figures with the `bench`/`hunter2`
credentials above; the strings are in the image).

## The simulator

`granite-sim` runs the real `granite-core` - node state machine,
actuator queue, rule engine, dispatcher, the HTTP route table - against
a fake board from a scenario file, and serves the real setup page over
HTTPS with a per-run certificate:

```sh
cd firmware
cargo run -p granite-sim -- serve                 # https://127.0.0.1:8443
cargo run -p granite-sim -- serve --assets ../web # page from disk
cargo run -p granite-sim -- modbus-map > docs/modbus_map.md
cargo run -p granite-sim -- keygen                # fleet recovery key
cargo run -p granite-sim -- recover <host>        # fleet recovery request
```

The browser experience and the API contract are the ones the firmware
serves, so page and client work needs no board. `granite-sim/README.md`
has the scenario file format and a curl walkthrough of first setup.
