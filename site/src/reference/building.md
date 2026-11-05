# Building and flashing

A condensed version of the firmware README (`firmware/README.md` in the
repository), which stays authoritative for toolchain versions and the
devboard features.

## Layout

`firmware/Cargo.toml` is a workspace with four members; `granite-fw`
is excluded on purpose and built on its own, because it is an ESP-IDF
crate with its own target, linker and `.cargo/config.toml`.

| Path | What |
|---|---|
| `firmware/granite-core` | Hardware-independent controller core: the plane client (discovery, polling, plane jobs, events, config push, plane firmware updates), node settings, rules, config, messages, HTTP handlers, Modbus map. Host-tested with `cargo test`. Workspace member. |
| `firmware/granite-proto` | The plane link wire format (frames, records, CRCs), `no_std`, shared by the controller and the plane. Workspace member. |
| `firmware/plane-logic` | The hardware-free core of the plane firmware: plane model, sequencer, unit discovery and polling, link handler, update state machine, drivers for the unit board chips. `no_std`; builds for `thumbv6m-none-eabi`, and with the `sim` feature runs on the host. Workspace member. |
| `firmware/granite-sim` | Host simulator: the real core and the real setup page over a fake controller, with fake planes that run the real `plane-logic`. Also `keygen`, `recover`, `modbus-map`. Workspace member. |
| `firmware/granite-fw` | The ESP-IDF binary for the controller. Not a workspace member. |
| `firmware/web` | The setup page (plain HTML/CSS/JS), gzipped into the image by `build.sh`. |
| `firmware/docs` | The architecture decision records and the generated Modbus map. |

The plane core firmware for the STM32G070 (`plane-fw`, and its
bootloader `plane-boot`) does not exist yet; `plane-logic` is the part
of it that is written. Until then there is no plane image to build or
flash.

## Toolchain

Rust nightly with `rust-src` (the ESP32-C6 is RISC-V, so upstream
nightly plus `build-std`, not the Xtensa `esp` toolchain), `ldproxy`,
`espflash`, and cmake, ninja, python3 and git for the ESP-IDF build.
The first build downloads ESP-IDF v5.5.5 into `~/.espressif`.

```sh
rustup toolchain install nightly
rustup component add rust-src --toolchain nightly
cargo install ldproxy espflash
```

## Build

```sh
cd firmware/granite-fw
cargo build --release
```

Output: `target/riscv32imac-esp-espidf/release/granite-fw` (an ELF;
espflash converts it while flashing). Features `wifi-dev`, `rgb-led`,
`devboard` and `hwtest` are for bare devkits only, and the
`*-test` features are bench fault injections; none of them may be in
an image that goes on a controller.

The current image runs the rev D node model with a placeholder plane
link driver that answers every address with NAK, so it finds no
planes; the real driver comes with the rev D board.

Optional at build time:

- `FLEET_RECOVERY_PUBKEY="$(cat fleet-recovery.pub)"` bakes the fleet
  recovery public key into the image.
- `ESP_IDF_SDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.defaults.signing"`
  enables OTA signature verification; it needs `signing_key.pem` in the
  crate root, generated with `espsecure.py generate_signing_key
  --version 2 --scheme ecdsa256`. Keep that key out of git and backed
  up.

## Flash

```sh
cd firmware/granite-fw
espflash flash --monitor --port /dev/ttyACM0 \
    --bootloader target/riscv32imac-esp-espidf/release/build/bootloader.bin \
    --partition-table partitions.csv \
    --target-app-partition ota_0 \
    --erase-parts otadata \
    target/riscv32imac-esp-espidf/release/granite-fw
```

- `--bootloader` is required so the chip gets the ESP-IDF-built
  bootloader with rollback enabled, not espflash's bundled one.
- `--partition-table` is required because the table is applied at
  flash time; without it there are no OTA slots.
- `--target-app-partition ota_0` names the slot to write: there is no
  `factory` partition, so espflash has no default app slot.
- `--erase-parts otadata` makes the bootloader run the image just
  flashed rather than the slot a board with OTA history last booted.
  It is also what moves a board off the older layout (`factory` plus
  2.5 M slots) onto the current one.
- `nvs` is never written by a flash and keeps the same offset in both
  layouts, so configuration, the device certificate and the recovery
  token survive; `espflash erase-parts nvs` wipes them on purpose.

Partitions (8 MB): `nvs` 192 K at 0x9000, `otadata` 8 K at 0x39000,
`coredump` 64 K at 0x3B000, `ota_0` 0x3D0000 (3,997,696 B) at 0x50000,
`ota_1` 0x3D0000 at 0x420000. No `factory` partition: its space went to
the two OTA slots, because rollback already guarantees that one slot
holds the last image that validated.

## Producing an OTA image

The Firmware page and the `ota` command take an application image
(`.bin`), not the ELF. The ESP-IDF build under the target directory has
no app image either: it produces only `build/bootloader.bin` and
`build/partition-table.bin`, because cargo links the app. (The
`libespidf.bin` under `build/esp-idf-sys-*/out/build/` is the
esp-idf-sys stub project, about 150 KB with no firmware in it - never
push that.) Make the image with espflash:

```sh
cd firmware/granite-fw
ELF=target/riscv32imac-esp-espidf/release/granite-fw
espflash save-image --chip esp32c6 $ELF granite-fw-<version>.bin
sha256sum granite-fw-<version>.bin     # what the pull command needs
```

Do not pass `--merge`: that produces a full-flash image with the
bootloader and the partition table in it, not an OTA image. Check the
result with `esptool.py --chip esp32c6 image_info <the .bin>`; the
checksum and the validation hash must both say valid.

For a build with signature verification enabled, append the signature
block afterwards - espflash writes the plain image, and the ESP-IDF
build that does the signing never sees this app:

```sh
espsecure.py sign_data --version 2 --keyfile signing_key.pem \
    --output granite-fw-<version>-signed.bin granite-fw-<version>.bin
espsecure.py verify_signature --version 2 --keyfile signing_key.pem \
    granite-fw-<version>-signed.bin
sha256sum granite-fw-<version>-signed.bin
```

`sign_data` pads the image to the next 4 KB boundary itself and appends
a 4 KB signature sector, so there is no separate padding step. The
commands above were verified on 2026-10-10 with espflash 4.6.0 and
esptool/espsecure 5.3.1 on a default release build.

**Unverified:** that a board built with `sdkconfig.defaults.signing`
accepts such an image over OTA. No image has been pushed to a board
yet; try the first one on a bench board.

## Host tests and the simulator

```sh
cd firmware
cargo test                                   # every workspace member
cargo test -p plane-logic --features sim     # the plane logic on its own
cargo build -p plane-logic --target thumbv6m-none-eabi   # it still builds for the MCU
cargo run -p granite-sim -- serve            # https://127.0.0.1:8443
cargo run -p granite-sim -- serve --scenario granite-sim/scenarios/two-planes.toml
cargo run -p granite-sim -- serve --assets web
cargo run -p granite-sim -- modbus-map > docs/modbus_map.md
```

The `two-planes.toml` scenario puts two simulated planes on the link,
one with all four unit boards, with faults that arrive while it runs;
`granite-sim/README.md` has the scenario format.

## This site

```sh
cargo install mdbook
cd site
mdbook serve --open        # live preview
mdbook build               # static output in site/book/
```
