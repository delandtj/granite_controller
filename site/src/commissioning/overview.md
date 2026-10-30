# Overview and order of work

Commissioning follows the system's own bring-up order, and it is split
in two by one event: sealing the tank. Everything that needs a USB
port, an SWD probe or a DIP switch happens before; everything after is
done over the network.

## The order

| Step | Where | Chapter |
|---|---|---|
| 1. Assemble the planes, set their addresses, connect power, nodes and the controller cables | frame, bus off | [Assembling and cabling planes](planes.md) |
| 2. Flash the controller firmware, record MAC and recovery token | bench, USB | [Flash and first boot](first-boot.md) |
| 3. Connect Ethernet and power, find the controller, set the admin password | bench or frame, network | [First setup](first-setup.md) |
| 4. Fix the network configuration (DHCP reservation or static) | network | [Network configuration](network.md) |
| 5. Check every plane is found, verify every node's power and LED sense, set names and boot policy, cable and name the probes | frame, network | [Nodes and boot policy](nodes.md) |
| 6. Connect the broker | network | [MQTT](mqtt.md) |
| 7. Enable Modbus if a master needs it | network | [Modbus TCP](modbus.md) |
| 8. Enable the rules you want to run without the network | network | [Rules](rules.md) |
| 9. Verify, export the configuration, seal | frame | [Checklist before sealing](before-sealing.md) |

Steps 1 and 2 are independent and can run in parallel. Steps 4 to 8
can be done in any order and repeated; the configuration lives in flash
on the controller and survives reboots and firmware updates. The plane
addresses live on the planes' DIP switches and nowhere else.

## What protects you from mistakes

- **Every slot defaults ON in hardware.** A plane core that is
  unpowered, in reset or crashed, a missing controller, a pulled link
  cable or a pulled chain cable never cuts a node. The overvoltage cut,
  the slot and unit current limits and the module's own limit all work
  without any firmware.
- **Nothing moves a node on its own.** A controller reboot, a crash or
  a firmware update never changes node power: OFF is held by the plane,
  not the controller, and no boot policy runs for a controller restart.
  The only exceptions are a rule you enabled and the per-node boot
  policy on a *plane* boot.
- **Presses end on their own.** A plane can press a node's button only
  while its core keeps the press enable line alive; a core that hangs
  or resets releases every button within about a second.
- **Network changes are commit-confirmed.** A new address is applied
  live but not saved until you confirm it through the new path within
  the confirm window (default 5 minutes). No confirm, and the board
  reboots into the old settings. MQTT and security changes are staged
  the same way and are also applied live, but reverting them needs no
  reboot: the stored section simply comes back. A new device
  certificate is the exception - the HTTPS server only picks one up at
  start-up.
- **Controller firmware updates roll back.** A new image has 10 minutes
  to get an IP and reach you (broker connected or an authenticated
  HTTPS request). If it cannot, the bootloader returns to the previous
  image, and finally to the factory image flashed over USB. A
  controller whose planes are unplugged can still take an update.
- **Factory reset works without the password.** The per-device recovery
  token, or a fleet key you hold, resets a controller over the network.
  Both only work if you recorded the token or installed the key before
  sealing.

## Tools you will use

- A browser for the setup page at `https://granite-xxxxxx.local/` or
  `https://<ip>/`.
- `curl` or any HTTP client for the same API, with a bearer token.
- `mosquitto_sub` and `mosquitto_pub` or your broker's tools for MQTT.
- `espflash` and a serial terminal (`picocom`, `screen`, or
  `espflash monitor`) on the bench.
- An SWD probe with an ARM Cortex 10-pin 1.27 mm cable for the plane
  cores' first firmware.
- A multimeter, crimp tools for JST PH (2.0 mm) and GH (1.25 mm), and
  a screwdriver for the M2 module screws and the node terminals.
- `granite-sim` from the firmware tree: the fleet key generator, the
  recovery tool and a full simulator of the controller with simulated
  planes, for trying the page and the API without hardware.
