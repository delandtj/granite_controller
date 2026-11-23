# What you need

Collect these before the boards go into the frame. Items marked
**before sealing** cannot be done once the tank is closed.

## From the frame

| Provision | Detail |
|---|---|
| 48 V DC bus | 48-60 V nominal, SELV. Every part on the planes is rated 100 V; the slots switch on above about 38 V and off above about 65 V. |
| Breaker or fuse outside the fluid | One per block or per branch of the bus, **outside the fluid** (one 63 A DC breaker per block is the option drawn; it is not decided). There are no fuses on the planes and none may be added in the tank: a fuse cooled by the fluid blows late or not at all, can arc in it, and cannot be replaced there. Overcurrent inside the tank is handled by the hot-swap limiters on every unit and every slot. |
| Power cables per plane | One cable to the core's XT30 (J1) and one to each unit's XT60 (J201), from a distribution block in the fluid. Both connectors are keyed; pin 1 is `-`, pin 2 is `+`. The unit input is rated for about 14 A at 38 V (two slots of about 265 W); 2 x 4 mm2 per unit is drawn, whether 4 mm2 fits the XT60H-F cup is to be checked (12 AWG otherwise). The core draws under 1 A (its 3.3 V supply and the isolated 12 V for the link and the controller); 2 x 0.75 mm2 is drawn, and such a thin lead needs its own small breaker outside the fluid. The bus does not pass from board to board. |
| Mounting bar | One bar per plane that holds the four units in a row, each board screwed down to it, 4 mm apart. The units plug into each other (a 2x7 chain and a 2x3 link, pin header into socket), so there are no cables between units; the bar holds the boards, the connectors carry no load. The core sits off the row, in the frame's infrastructure section, on its own four M3 holes. See [Assembling and cabling planes](../commissioning/planes.md). |
| Core ribbons | Two per plane, core to unit 1, about 0.15 m, pin n to pin n: W_CHAIN (14-way, core J301 to unit 1 J301) and W_CORELINK (6-way, core J3 to unit 1 J303). Male IDC plug at the core, IDC socket at the unit. **Open**: no male IDC plug for 1.27 mm cable that fits the core's sockets has been found yet, and the two plugs sit side by side (7.4 mm between their end pins), so the fit is still to be tried. |
| Link ribbon | One per controller, W_LINK: 6-way 2x3 IDC ribbon, about 0.35 m (1 m at most), socket at both ends, pin n to pin n, from controller J15 to the core's J2. It carries the CAN FD link and the controller's only supply. |
| Termination | Two standard 2.54 mm jumper caps on JP301 of the last unit of the row (1-2 and 3-4). None anywhere else. |
| Node cables | Per node two cables, both made by the installer: a 14 AWG pair from the slot's screw terminal to the node's 19 V DC jack (barrel plug at the node end), and a 4-wire front-panel cable, about 0.6 m (JST GH at the plane), to the motherboard's power switch, reset switch and power LED pins. **Open**: the barrel plug is not chosen (the jack is not measured, and no plug rated for the node's 8.7 A has been found), and the motherboard front-panel pinout is not confirmed. See [Connections](connections.md). |
| Block switch | One 8-port switch per block for the nodes, with an SFP+ uplink, fed 12 V by its own converter off the 48 V bus (a CUI DQD50-48-S12-T, screw terminals). It has no port for the controller. |
| Port modules | One PM-19V-250W per populated slot, with four M2 x 5 screws and split lock washers. |
| Temperature probes | DS18B20 probes on 3-wire cables to controller J4..J7. The four connectors share one bus; the firmware tracks up to eight probes by ROM id. |
| Dry contacts (optional) | Potential-free contacts (leak float, lid switch, door) to controller J10, closing to GND. |
| Mounting | Controller: four M3 holes, 4 mm in from each corner of the 105 x 50 mm outline, within the reach of its 0.35 m link ribbon to the core. Plane: every board on its own four M3 holes, units in a row on a common bar, 4 mm apart when mated, the core nearby within its ribbons' reach. |

## From the network

| Provision | Detail |
|---|---|
| One switch port | For the controller, on a management switch outside the block (the block switch has no port to spare). 10/100 Mbit/s, full duplex, auto-MDIX (the W5500 has no auto-MDIX of its own), straight patch cable. Any normal switch port qualifies. |
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
| A controller firmware image built for the board: the `link-canfd` image (not a devboard build, not a `link-i2c` image) | See [Building and flashing](../reference/building.md). |
| An SWD probe with an ARM Cortex 10-pin (1.27 mm) cable, or a Tag-Connect TC2030-NL cable | The plane core firmware (`plane-fd`) is flashed over SWD on J6 (or J5). |
| An address plan for the planes | Each plane on a link gets a unique address 0x30-0x37 on its DIP switch, and the address fixes its node numbers. Decide it before the planes go in. |
| A label or a sheet to record **MAC address + recovery token** per controller | That pair is the whole recovery path for a submerged controller. Record it the moment the console prints it. |
| The admin password you will set | There is no default password; nothing works over the network until one is set. |
| Optionally the fleet recovery public key | One ECDSA P-256 key pair per site lets you factory-reset any board over the network without its token. Generate it once with `granite-sim keygen`. |
| Optionally the firmware signing key | Only if you build with OTA signature verification enabled. Losing it means no more updates for the boards built against it. |

## Cable material

Everything that goes into the fluid has to survive the fluid. The
boards follow the OCP immersion compatibility guidelines; the cables
and glands are the integrator's job. See
[Immersion constraints](immersion.md).
