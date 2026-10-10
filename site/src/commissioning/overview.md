# Overview and order of work

Commissioning follows the firmware's own bring-up order, and it is
split in two by one event: sealing the tank. Everything that needs the
USB port happens before; everything after is done over the network.

## The order

| Step | Where | Chapter |
|---|---|---|
| 1. Flash the firmware, record MAC and recovery token | bench, USB | [Flash and first boot](first-boot.md) |
| 2. Connect Ethernet and power, find the board, set the admin password | bench or frame, network | [First setup](first-setup.md) |
| 3. Fix the network configuration (DHCP reservation or static) | network | [Network configuration](network.md) |
| 4. Cable the nodes and probes, verify every LED sense, set names and boot policy | frame, network | [Nodes and boot policy](nodes.md) |
| 5. Connect the broker | network | [MQTT](mqtt.md) |
| 6. Enable Modbus if a master needs it | network | [Modbus TCP](modbus.md) |
| 7. Enable the rules you want to run without the network | network | [Rules](rules.md) |
| 8. Verify, export the configuration, seal | frame | [Checklist before sealing](before-sealing.md) |

Steps 3 to 7 can be done in any order and repeated; the configuration
lives in flash on the board and survives reboots and firmware updates.

## What protects you from mistakes

- **Nothing moves a node on its own.** A controller reboot, a crash or
  a firmware update never changes node power. The only exception is a
  rule you enabled (and, once the firmware applies it, the per-node
  boot policy).
- **Network changes are commit-confirmed.** A new address is applied
  live but not saved until you confirm it through the new path within
  the confirm window (default 5 minutes). No confirm, and the board
  reboots into the old settings. MQTT and security changes are staged
  the same way but take effect only after confirm **and a reboot**.
- **Firmware updates roll back.** A new image has 10 minutes to prove
  it can drive the expanders, get an IP and reach you. If it cannot,
  the bootloader returns to the previous image, and finally to the
  factory image flashed over USB.
- **Factory reset works without the password.** The per-device recovery
  token, or a fleet key you hold, resets a board over the network.
  Both only work if you recorded the token or installed the key before
  sealing.

## Tools you will use

- A browser for the setup page at `https://granite-xxxxxx.local/` or
  `https://<ip>/`.
- `curl` or any HTTP client for the same API, with a bearer token.
- `mosquitto_sub` and `mosquitto_pub` or your broker's tools for MQTT.
- `espflash` and a serial terminal (`picocom`, `screen`, or
  `espflash monitor`) on the bench.
- `granite-sim` from the firmware tree: the fleet key generator, the
  recovery tool and a full simulator of the board for trying the page
  and the API without hardware.
