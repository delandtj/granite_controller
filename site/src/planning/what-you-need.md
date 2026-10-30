# What you need

Collect these before the boards go into the frame. Items marked
**before sealing** cannot be done once the tank is closed.

## From the frame

| Provision | Detail |
|---|---|
| 48 V DC bus | 48-60 V nominal, SELV. Every part on the planes is rated 100 V; the slots switch on above about 38 V and off above about 65 V. |
| Breaker or fuse outside the fluid | One per box or per branch of the bus, **outside the fluid**. There are no fuses on the planes and none may be added in the tank: a fuse cooled by the fluid blows late or not at all, can arc in it, and cannot be replaced there. Overcurrent inside the tank is handled by the hot-swap limiters on every unit and every slot. |
| Power cables per plane | One cable to the core's XT30 (J1) and one to each unit's XT60 (J201). Both connectors are keyed; pin 1 is `-`, pin 2 is `+`. The unit input is rated for about 14 A at 38 V (two slots of about 265 W); the core draws little (its 3.3 V supply, the 12 V aux, the link). The bus does not pass from board to board. |
| Chain cables | One per unit: a short flat JST PH 14-pin cable (about 50-100 mm), core to unit 1, unit 1 to unit 2 and so on. Made by the installer, see [Assembling and cabling planes](../commissioning/planes.md). |
| Link cables | JST PH 5-pin, pin 1 to pin 1: controller J13 or J14 to the first plane's J2, then plane J3 to the next plane's J2. |
| 12 V aux cables | JST PH 2-pin, pin 1 to pin 1: a plane core's J4 to controller J11, and a second plane's J4 to J12 when there is more than one plane. |
| Node cables | Per node two cables, both made by the installer: a 14 AWG pair from the slot's screw terminal to the node's 19 V DC-ATX converter, and a 4-wire front-panel cable (JST GH) to the motherboard's power switch, reset switch and power LED pins. See [Connections](connections.md). |
| Port modules | One PM-19V-250W per populated slot, with four M2 x 5 screws and split lock washers. |
| Temperature probes | DS18B20 probes on 3-wire cables to controller J4..J7. The four connectors share one bus; the firmware tracks up to eight probes by ROM id. |
| Dry contacts (optional) | Potential-free contacts (leak float, lid switch, door) to controller J10, closing to GND. |
| Mounting | Controller: four M3 holes, 4 mm in from each corner of the 105 x 50 mm outline. Plane: every board on its own four M3 holes, boards in a row 2-5 mm apart. Where the controller sits relative to the planes (in the fluid or above it) is still open; it sets the cable lengths. |

## From the network

| Provision | Detail |
|---|---|
| One switch port | 10/100 Mbit/s, full duplex, auto-MDIX (the W5500 has no auto-MDIX of its own). Any normal switch port qualifies. |
| An IP address | DHCP by default. Reserve the address on the DHCP server by MAC, or configure a static address during commissioning. |
| A way to reach port 443 | The setup page and the API. Port 80 only answers `/id` and redirects. |
| MQTT broker (optional) | Reachable from the controller's subnet, TLS on 8883 recommended, with the broker CA in PEM form at hand. |
| Modbus master (optional) | Its IP address or subnet, because the controller only listens for an explicit allow-list. |
| NTP | From DHCP option 42 or a configured server. Without time the controller runs on its firmware build time, which is enough for its own TLS but not for log timestamps. |

The planes are not on the network; they are reached only through the
controller. The full port table is in [Network requirements](network.md).

## On the bench, before sealing

| Item | Why |
|---|---|
| A USB-C cable and a host with `espflash` or any serial terminal | The controller's first firmware image is flashed over USB. The USB console is also where the recovery token is printed. |
| A controller firmware image built for the board (not a devboard build) | See [Building and flashing](../reference/building.md). |
| An SWD probe with an ARM Cortex 10-pin (1.27 mm) cable, or a Tag-Connect TC2030-NL cable | The first plane core firmware is flashed over SWD on J6 (or J5). The plane core firmware is not written yet at the time of writing. |
| An address plan for the planes | Each plane gets a unique address 0x30-0x37 on its DIP switch, and the address fixes its node numbers. Decide it before the planes go in. |
| A label or a sheet to record **MAC address + recovery token** per controller | That pair is the whole recovery path for a submerged controller. Record it the moment the console prints it. |
| The admin password you will set | There is no default password; nothing works over the network until one is set. |
| Optionally the fleet recovery public key | One ECDSA P-256 key pair per site lets you factory-reset any board over the network without its token. Generate it once with `granite-sim keygen`. |
| Optionally the firmware signing key | Only if you build with OTA signature verification enabled. Losing it means no more updates for the boards built against it. |

## Cable material

Everything that goes into the fluid has to survive the fluid. The
boards follow the OCP immersion compatibility guidelines; the cables
and glands are the integrator's job. See
[Immersion constraints](immersion.md).
