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
                      frame handler and the register map
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
cargo build --release
cargo clippy --release --all-targets -- -D warnings
```

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

Image size, release, default features (2026-10-10): 1,719,360 bytes of
the 2,621,440-byte slot, 66 %. `cargo build --release` keeps
`opt-level = "s"`; do not switch the release profile to `3` without
checking that number again.

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
    target/riscv32imac-esp-espidf/release/granite-fw
# monitor only (the --elf gives symbol names in a panic backtrace):
espflash monitor --port /dev/ttyACM0 \
    --elf target/riscv32imac-esp-espidf/release/granite-fw
```

Both extra flags matter. Without `--bootloader`, espflash flashes the
bootloader it bundles itself (an ESP-IDF v6.1-beta build at espflash
4.6.0) instead of the one the ESP-IDF build produced from
`sdkconfig.defaults`, so `CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` and
the signature check would not be the ones in the bootloader on the chip.
Without `--partition-table`, the default single-app table is flashed and
there are no OTA slots.

With no `--target-app-partition`, espflash writes the app to `factory`,
which is what the bootloader runs when `otadata` is empty:

```sh
# bench image to factory (what the bootloader runs):
espflash flash --port /dev/ttyACM0 --bootloader ... --partition-table partitions.csv <elf>
# and the same image to ota_0, so the first OTA replaces ota_1:
espflash flash --port /dev/ttyACM0 --partition-table partitions.csv \
    --target-app-partition ota_0 <elf>
```

`nvs` is never written by `espflash flash`, so the device certificate,
the recovery token and the configuration survive a reflash. Erase them
on purpose with `espflash erase-parts nvs` or with the console's
`factory-reset CONFIRM` (which keeps the `factory` namespace: the
recovery token and the fleet key).

If the board does not enter download mode by itself, pull the BOOT test
pad (GPIO9) to GND while powering up.

## Partition layout

`granite-fw/partitions.csv`, 8 MB flash, table at 0x8000:

| Name | Type | SubType | Offset | Size |
|---|---|---|---|---|
| nvs | data | nvs | 0x9000 | 192 K |
| otadata | data | ota | 0x39000 | 8 K |
| coredump | data | coredump | 0x3B000 | 64 K |
| factory | app | factory | 0x50000 | 2.5 M |
| ota_0 | app | ota_0 | 0x2D0000 | 2.5 M |
| ota_1 | app | ota_1 | 0x550000 | 2.5 M |

Last byte used is 0x7D0000, so 192 K of the 8 MB stays free. App
partitions need 64 K alignment, which leaves a 20 K hole between
`coredump` (ends 0x4B000) and `factory` (starts 0x50000).

The bench image goes to `factory` and `ota_0`; OTA alternates
`ota_0`/`ota_1`; `factory` is only ever rewritten over USB.

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
`recovery-token` console command, the fingerprint by `id`.

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
  help                  this list
```

`espflash monitor` does not forward keystrokes in non-interactive mode;
use it interactively, or any serial terminal (`picocom /dev/ttyACM0`).

## HTTPS page and API

HTTPS on 443 with the device certificate; plain HTTP on 80 serves only
`/id` and a redirect. One admin password (PBKDF2-HMAC-SHA256, 20k
iterations, per-device salt), a session cookie for the page and
`Authorization: Bearer <token>` for scripts, 5 failed logins then a 60 s
lockout.

`/api/v1/...` carries the same command vocabulary as MQTT, because both
go through one dispatcher in `granite-core`: status, config sections
(with staging and confirm for anything that can cut the session), node
actions, rules, firmware upload and rollback, log tail, export/import,
factory reset, recovery. (`probe_scan` is accepted and still logs a
TODO: the 1-wire re-scan is not wired to the hardware layer yet.) The page itself is plain
HTML/CSS/JS from `firmware/web/`, gzipped into the image by
`web/build.sh`; `granite-sim serve --assets ../web` serves it from disk
for editing.

## MQTT

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

ADR 0001 component 7 has the authoritative table and the command list;
`granite-core/src/msg.rs` is the implementation and its tests are the
payload fixtures. The worker reconnects on an exponential ladder (1 s to
60 s) and keeps publishing `mqtt_connected` into the snapshot the rule
engine reads, so rules that watch the broker work.

## Modbus TCP

Off by default, and it refuses to listen without an IP/CIDR allow-list:
Modbus has no authentication. Port, unit id, maximum connections and the
allow-list are the `sec.modbus` section; enabling it (or editing the
allow-list) takes effect within about 5 s, no reboot.

The register map is `docs/modbus_map.md`, generated from
`granite-core/src/modbus_map.rs` with `cargo run -p granite-sim --
modbus-map`. It is versioned (input register 14) because the RS-485 duck
protocol will reuse the same semantics.

## OTA, signing and probation

Two OTA slots plus `factory`, ESP-IDF `esp_ota_*`, rollback enabled in
the bootloader.

- **Push**: the Firmware page uploads a `.bin` over HTTPS
  (`POST /api/v1/firmware/upload`, authenticated, streamed into the slot
  in 2 KB chunks so a 2.5 MB image never sits in RAM).
- **Pull**: the `ota` command over MQTT (or the API) takes
  `{"url":..,"sha256":..}` and the controller fetches it over HTTPS
  itself, using the MQTT CA.
- **Probation**: a new image boots `pending_verify` and must, within
  `sys.t_validate_s` (default 10 min), have (a) initialised the
  expanders, (b) got link and an IP, and (c) either connected to the
  configured broker or served an authenticated HTTPS request. Then it
  marks itself valid; otherwise it marks itself invalid and reboots, and
  the bootloader falls back to the other slot (and then to `factory`).
  A step nobody can satisfy is not required: with MQTT disabled the
  broker step is not armed, so an authenticated HTTPS request is what
  confirms such an image. The LED blinks at 4 Hz while an image is on
  probation, and `ota-mark-valid` on the console confirms it by hand.

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

A bare ESP32-C6 devboard has no W5500 and no expanders. Two features
make it useful anyway; neither belongs in an image that goes on a
board.

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
"Wi-Fi and BLE stay off" still describes the shipped image. For
reference, release image sizes on 2026-10-10 were 1,719,360 bytes
default and 2,101,680 bytes with `--features wifi-dev`.

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
