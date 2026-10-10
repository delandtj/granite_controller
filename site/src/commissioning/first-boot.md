# Flash and first boot over USB

Done on the bench with the board out of the fluid. This is the only
step that needs the USB-C port.

## 1. Flash the image

Build or obtain a release image of `granite-fw` for the board (not a
devboard build with Wi-Fi or RGB features). Connect USB-C; the board
powers from it. Then flash the bootloader, the partition table and the
application to the `factory` slot, and the same application to `ota_0`:

```sh
cd firmware/granite-fw
ELF=target/riscv32imac-esp-espidf/release/granite-fw
espflash flash --port /dev/ttyACM0 \
    --bootloader target/riscv32imac-esp-espidf/release/build/bootloader.bin \
    --partition-table partitions.csv $ELF
espflash flash --port /dev/ttyACM0 --partition-table partitions.csv \
    --target-app-partition ota_0 $ELF
```

Both the `--bootloader` and the `--partition-table` flags are required:
without them the chip gets espflash's own bootloader without rollback
support and a partition table without OTA slots. The details are in
[Building and flashing](../reference/building.md).

If the chip does not enter download mode by itself, hold the BOOT test
pad (GPIO9) to GND while applying power, then release it.

The `nvs` partition is never touched by a reflash, so a board that was
already commissioned keeps its password, certificate, recovery token
and configuration across firmware reflashes over USB.

## 2. Watch the first boot

Open the console:

```sh
espflash monitor --port /dev/ttyACM0 --elf $ELF
# or any serial terminal, 115200 baud is irrelevant on USB CDC:
picocom /dev/ttyACM0
```

On a board with an empty `nvs` the firmware prints, once:

- the device id and MAC (`granite-xxxxxx`),
- the **recovery token**: 32 base32 characters in eight groups of
  four, `XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`,
- the SHA-256 fingerprint of the freshly generated HTTPS certificate,
- a `boot complete: free heap ...` line, then a heartbeat every 10 s.

**Write down the MAC and the recovery token together, now.** The token
is printed again by the `recovery-token` console command for as long as
you have USB access, and shown once on the setup page when the password
is set there (not if you set it from the console, and not after a
factory reset). After sealing, that token or a fleet key is the only
way to recover a board whose password or network is lost.

Expect log lines about the expanders and sensors: on a bare board with
nothing cabled, probes enumerate to an empty list and the dry contacts
read open. An expander that does not answer is logged as a fault, not a
crash, and the console and network stay up.

## 3. Console checks

```
granite> id
granite> status
granite> net
```

`id` shows the device id, MAC, firmware version, build time and the
certificate fingerprint. `status` shows the link, the OTA slot that is
running and its state, uptime and heap. With no Ethernet cable the link
reads down and the status LED blinks slowly (0.25 Hz).

Optionally set the admin password here instead of in the browser:

```
granite> set-password <at least 8 characters>
```

Setting the password from the console also invalidates every API
token, because it mints a new device salt; the page's password change
does not. On a fresh board there are no tokens yet, so it does not
matter here.

The full command list is in [Console commands](../reference/console.md).

## 4. Mark the bench image valid

Probation is for images written over the air, so an image flashed over
USB normally boots straight into the valid state. If `status` or the
Firmware page nevertheless shows `pending_verify`, confirm it by hand:

```
granite> ota-mark-valid
```

The Firmware page later shows `factory` as the running slot and `ota_0`
holding the same version.

## Devboard builds

A bare ESP32-C6 devkit without the W5500 and expanders can run the
firmware with the `wifi-dev` and `rgb-led` features for exercising the
network side. Those images must never go on a controller board: the
Wi-Fi credentials are baked in and the radio is on. See the firmware
README for the feature set.
