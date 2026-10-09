# Granite controller firmware

Rust on ESP-IDF for the rev C controller board: an ESP32-C6-WROOM-1-N8
(RISC-V, 8 MB flash) with a WIZnet W5500 on SPI2. See
`docs/adr/0001-firmware-architecture.md` for the architecture and
`../docs/controller.md` for the hardware and the pin map.

Crates in this directory:

- `granite-fw/` - the ESP-IDF binary for the board (standalone cargo
  crate; it is **not** a member of the `firmware/` workspace, because it
  needs its own target, toolchain and linker).
- `granite-core/` - host-testable logic, a member of the `firmware/`
  workspace.

`granite-fw` at this stage is the bring-up skeleton: expander resets held
low, status LED blinking, device id and MAC on the console, W5500 up with
DHCP, heartbeat.

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

The ESP-IDF checkout and its toolchains are downloaded on the first build
by `esp-idf-sys` into `~/.espressif` (sources under
`~/.espressif/esp-idf-<hash>`), with per-crate build state in
`granite-fw/.embuild/`. Nothing of that lands in the repo;
`.embuild/` and `target/` are gitignored. The first build takes a while
(ESP-IDF download plus a full ESP-IDF compile); later builds are quick.

## Build

```sh
cd firmware/granite-fw
cargo build --release
```

`MCU=esp32c6`, `ESP_IDF_VERSION=v5.5.5` and `CARGO_WORKSPACE_DIR` come
from `.cargo/config.toml`, so no environment setup is needed. The last
one matters: `esp-idf-sys` looks for `sdkconfig.defaults` relative to the
cargo workspace directory, which embuild derives from the target
directory - with a `CARGO_TARGET_DIR` pointing outside the crate that
guess lands in the wrong place and the configuration is silently ignored.

The build output is `target/riscv32imac-esp-espidf/release/granite-fw`
(an ELF; espflash turns it into an image while flashing). If you have
`CARGO_TARGET_DIR` set in your environment, the binary is under that
directory instead.

## Flash and monitor

The USB-C connector is the ESP32-C6's **USB Serial/JTAG** peripheral, and
the console is on it (`CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y`); UART0 is
left to the 1-wire probe bus. It usually shows up as `/dev/ttyACM0`.

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
later the signature check would not be the ones in the bootloader on the
chip. Without `--partition-table`, the default single-app table is
flashed and there are no OTA slots.

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
limitations", says not to set `CONFIG_PARTITION_TABLE_CUSTOM` at all), so
the table is applied **at flash time**: always pass
`--partition-table partitions.csv` to `espflash`. `sdkconfig.defaults`
selects the large single-app table instead, purely so the build-time
image size check has 1.5 MB of headroom; that choice never reaches the
chip.

## Signing key

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
