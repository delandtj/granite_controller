<div class="board-hero">

# Granite controller

<p class="board-legend"><span>Rev D</span><span>105 x 50 mm</span><span>4 layers</span><span>ESP32-C6 + W5500</span><span>runs submerged</span></p>

</div>

Granite is the electronics of an immersion-cooled frame of motherboards
(nodes). It is two kinds of board:

- **Power planes.** A plane takes the frame's 48 V bus and turns it into
  one switched, protected and metered 19 V rail per node, through a port
  module (PM-19V-250W) screwed onto each slot. The same plane presses
  each node's power and reset buttons and reads its power LED. A plane
  is one **core board** (microcontroller, isolated link, isolated 12 V
  aux output) and one to four identical **unit boards** of two slots
  each, chained with short cables: 2, 4, 6 or 8 nodes per plane.
- **The controller.** One controller serves up to eight planes over an
  isolated I2C link. It measures fluid temperatures with DS18B20 probes,
  watches up to four dry contacts (leak float, lid switch), and reports
  all of it, nodes included, over Ethernet. It is powered from the
  planes' isolated 12 V aux outputs, never from the 48 V bus.

From the outside the frame needs the 48 V bus (with its breaker outside
the fluid) and one Ethernet cable. Northbound the controller offers
three interfaces, all served by the same command core so they behave
identically:

- an HTTPS setup page and JSON API on port 443,
- an MQTT client that publishes state and accepts commands (off by
  default),
- a Modbus TCP server with a fixed register map (off by default).

All boards are designed to run fully submerged in dielectric fluid.
Once the tank is closed there is no USB port, no switch and no LED you
can rely on, so everything about commissioning is organised around one
rule: **configure, verify and record before you seal**, and keep the
network path recoverable afterwards.

This manual describes controller rev D with power planes. The rev C
controller (relays and LED sense on the controller, eight node
connectors, 19 V input) is kept as demoware; its manual is in the
repository at tag `rev-c-demo`.

## Who this site is for

- **Installers** planning a frame: what to cable, which network and
  power provisions to make, how to assemble and address the planes, and
  in what order to bring everything up. Start with
  [What you need](planning/what-you-need.md).
- **Operators** running frames: controlling nodes, reading state,
  integrating with MQTT, Home Assistant or a Modbus master, updating
  firmware and recovering a board. Start with
  [Controlling nodes](operation/nodes.md).
- **Developers** building the firmware: the
  [Reference](reference/building.md) section, then the firmware README
  and the architecture decision records in the repository.

## Hardware at a glance

| Item | Controller rev D |
|---|---|
| MCU | ESP32-C6-WROOM-1 (RISC-V), radio off |
| Network | 10/100 Ethernet, WIZnet W5500, RJ45 magjack |
| Power | 12 V from up to two planes' isolated aux outputs (2 x JST PH 2p, diode OR-ed) |
| Plane link | 2 x JST PH 5p, one isolated I2C bus plus a shared interrupt line, up to 8 planes (addresses 0x30-0x37) |
| Probes | 4 x JST PH 3p DS18B20 connectors on one 1-wire bus |
| Inputs | 4 dry contacts (JST PH 5p), 12 V input sense, board temperature |
| Sensor port | J9, JST PH 4p in Qwiic order (I2C) |
| Service | USB-C: console, flashing, bench power; used out of the fluid only |
| Board | 105 x 50 mm, 4 layers, four M3 mounting holes |

| Item | Power plane |
|---|---|
| Bus | 48-60 V DC nominal (SELV), slots on above about 38 V, off above about 65 V |
| Boards | 1 core board + 1-4 unit boards, 2 slots per unit, chained with JST PH 14-pin cables |
| Core | STM32G070 MCU, I2C mux, isolated link to the controller, isolated 12 V aux out, XT30 48 V input, 3-way DIP switch for the plane address |
| Unit | XT60 48 V input with its own hot-swap (auto-retry, about 18 A; the plane core latches a faulted unit off), two slots |
| Slot | hot-swap front end with overvoltage latch and V / I / P monitor, port module PM-19V-250W (19 V, 250 W), node power screw terminal, front-panel JST GH 4p (PWR, RST, power LED) |
| Default | every slot ON in hardware: a dead or missing core or controller never cuts a node |

## Status of the hardware, the firmware and this site

At the time of writing (October 2026) no rev D controller and no plane
board has been built. The controller rev D board is routed; the plane
core board is routed and the unit board is still being laid out. Board
sizes and some part choices may still change; where a page depends on
something still open, it says so.

The controller firmware has been moved to the rev D node model (nodes
on planes) and is tested against simulated planes on the host. The
plane core firmware is specified (it runs on the STM32G070 and owns the
node sequencing) and its hardware-independent logic exists as a tested
library, but the firmware image for the core is **not written yet**.
Until it is, a plane runs on its hardware defaults: every slot on, no
presses, nothing reported to the controller.

Should a page describe behaviour the firmware stores but does not act
on yet, it says so in a **Current firmware** note; treat such notes as
the to-do list for the first bring-up.

## Conventions

A node is a slot on a plane. Its number is fixed by the plane's address
(set with the DIP switch SW1 on the core board) and the slot's position
along the chain:

    node = 8 x (plane address - 0x30) + slot + 1

Slots are numbered 0 to 7 from the core outwards: unit 1 (the one next
to the core) holds slots 0 and 1, unit 4 holds slots 6 and 7, and on
every unit slot A (the core side) comes before slot B. So the plane at
0x30 carries nodes 1 to 8, the plane at 0x31 nodes 9 to 16, up to 57
to 64 at 0x37. Node numbers are the same everywhere a human reads them:
in the setup page, in MQTT targets and in Modbus register names. See
[Assembling and cabling planes](commissioning/planes.md).

The device id is `granite-` followed by the last six hex digits of the
Ethernet MAC address, for example `granite-37adc7`. It is the hostname,
the mDNS name, the default MQTT client id and the certificate name.
