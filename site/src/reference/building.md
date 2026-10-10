# Building and flashing

A condensed version of the firmware README (`firmware/README.md` in the
repository), which stays authoritative for toolchain versions and the
devboard features.

## Layout

| Path | What |
|---|---|
| `firmware/granite-core` | Hardware-independent core: node state machine, actuator, rules, config, messages, HTTP handlers, Modbus map. Host-tested with `cargo test`. |
| `firmware/granite-fw` | The ESP-IDF binary for the board. A standalone crate, not a workspace member. |
| `firmware/granite-sim` | Host simulator: the real core and the real setup page over a fake board. Also `keygen`, `recover`, `modbus-map`. |
| `firmware/web` | The setup page (plain HTML/CSS/JS), gzipped into the image by `build.sh`. |
| `firmware/docs` | The architecture decision record, the generated Modbus map, the Home Assistant example. |

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
`devboard` and `hwtest` are for bare devkits only and must not be in
an image that goes on a board.

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
ELF=target/riscv32imac-esp-espidf/release/granite-fw
espflash flash --monitor --port /dev/ttyACM0 \
    --bootloader target/riscv32imac-esp-espidf/release/build/bootloader.bin \
    --partition-table partitions.csv $ELF
espflash flash --port /dev/ttyACM0 --partition-table partitions.csv \
    --target-app-partition ota_0 $ELF
```

- `--bootloader` is required so the chip gets the ESP-IDF-built
  bootloader with rollback enabled, not espflash's bundled one.
- `--partition-table` is required because the table is applied at
  flash time; without it there are no OTA slots.
- `nvs` is never written by a flash; `espflash erase-parts nvs` wipes
  configuration and secrets on purpose.

Partitions (8 MB): `nvs` 192 K, `otadata` 8 K, `coredump` 64 K,
`factory` 2.5 M, `ota_0` 2.5 M, `ota_1` 2.5 M.

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
cargo test                                   # granite-core and granite-sim
cargo run -p granite-sim -- serve            # https://127.0.0.1:8443
cargo run -p granite-sim -- serve --assets ../web
cargo run -p granite-sim -- modbus-map > docs/modbus_map.md
```

## This site

```sh
cargo install mdbook
cd site
mdbook serve --open        # live preview
mdbook build               # static output in site/book/
```
