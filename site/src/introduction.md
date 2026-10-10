# Granite controller

The Granite controller is a small board that sits in an immersion-cooled
frame of eight motherboards (nodes) and gives you remote hands on them:
it presses each node's power and reset buttons, reads each node's power
LED to know whether it is on, measures fluid temperatures with DS18B20
probes, watches the frame's 19 V bus and up to four dry contacts (leak
float, lid switch), and reports all of it over Ethernet.

One Ethernet cable and one 19 V feed are all it needs from the outside.
Northbound it offers three interfaces, all served by the same command
core so they behave identically:

- an HTTPS setup page and JSON API on port 443,
- an MQTT client that publishes state and accepts commands (off by
  default),
- a Modbus TCP server with a fixed register map (off by default).

The board is designed to run fully submerged in dielectric fluid. Once
the tank is closed there is no USB port, no button and no LED you can
rely on, so everything about commissioning is organised around one
rule: **configure, verify and record before you seal**, and keep the
network path recoverable afterwards.

## Who this site is for

- **Installers** planning a frame: what to cable, which network and
  power provisions to make, and in what order to bring the board up.
  Start with [What you need](planning/what-you-need.md).
- **Operators** running frames: controlling nodes, reading state,
  integrating with MQTT, Home Assistant or a Modbus master, updating
  firmware and recovering a board. Start with
  [Controlling nodes](operation/nodes.md).
- **Developers** building the firmware: the
  [Reference](reference/building.md) section, then the firmware README
  and the architecture decision record in the repository.

## Hardware at a glance

| Item | Rev C |
|---|---|
| MCU | ESP32-C6-WROOM-1-N8 (RISC-V, 8 MB flash), radio off |
| Network | 10/100 Ethernet, WIZnet W5500, RJ45 magjack |
| Power | 19 V DC from the frame bus (JST PH 2p), about 0.4 A worst case |
| Nodes | 8 x JST PH 5p: power switch, reset switch, common, power LED + / - |
| Switching | 16 photoMOS relays (power and reset per node), fail-open |
| Probes | 4 x JST PH 3p DS18B20 connectors on one 1-wire bus |
| Inputs | 4 dry contacts (JST PH 5p), bus voltage sense, board temperature |
| Expansion | J9 Qwiic-order I2C sensor port, J8 2x6 expansion header (unpopulated) |
| Service | USB-C: console, flashing, bench power; used out of the fluid only |
| Board | 199 x 50 mm, 4 layers, four M3 mounting holes |

## Status of the firmware and of this site

The firmware is complete and its hardware-independent core is tested
on the host, but at the time of writing (October 2026) no rev C board
has been powered yet. The parts that need the real board are untested:
the W5500 Ethernet path (link, DHCP, AutoIP, commit-confirm, dead-man),
the relay presses and the press deadline, the power LED sense, the
probes, and the over-the-air probation ladder end to end. The HTTPS
page, the API, MQTT and Modbus have been exercised on a bare ESP32-C6
devkit. Where the current firmware stores a setting but does not act on
it yet, the page says so in a **Current firmware** note. Treat those
notes as the to-do list for the first bring-up.

## Conventions

Nodes are numbered 1 to 8 everywhere a human reads them: on the
silkscreen (NODE 1 to NODE 8), in the setup page, in MQTT targets and
in Modbus register names. JSON arrays in the state payload are zero
based, so node 1 is `nodes[0]`.

The device id is `granite-` followed by the last six hex digits of the
Ethernet MAC address, for example `granite-37adc7`. It is the hostname,
the mDNS name, the default MQTT client id and the certificate name.
