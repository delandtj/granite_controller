# What you need

Collect these before the board goes into the frame. Items marked
**before sealing** cannot be done once the tank is closed.

## From the frame

| Provision | Detail |
|---|---|
| 19 V DC feed | From the frame's redundant 19 V bus, tapped **upstream** of the per-node hot-swap channels and with its **own fuse**, so a node channel that trips cannot take the controller down. Polarity: J3 pin 1 is +19 V, pin 2 is GND. The board draws about 0.4 A at 19 V worst case, and carries a 0.75 A resettable fuse of its own. |
| Eight node cables | One 5-wire cable per node from the motherboard front-panel header to J11..J18. See [Connections](connections.md) for the pinout and what to check on the motherboard side. |
| Temperature probes | DS18B20 probes on 3-wire cables to J4..J7. The four connectors share one bus; the firmware tracks up to eight probes by ROM id. |
| Dry contacts (optional) | Potential-free contacts (leak float, lid switch, door) to J10, closing to GND. |
| Mounting | Four M3 holes, 4 mm in from each corner of the 199 x 50 mm outline. Keep the ESP32 module's antenna edge and the RJ45 opening clear. |

## From the network

| Provision | Detail |
|---|---|
| One switch port | 10/100 Mbit/s, full duplex, auto-MDIX (the W5500 has no auto-MDIX of its own). Any normal switch port qualifies. |
| An IP address | DHCP by default. Reserve the address on the DHCP server by MAC, or configure a static address during commissioning. |
| A way to reach port 443 | The setup page and the API. Port 80 only answers `/id` and redirects. |
| MQTT broker (optional) | Reachable from the controller's subnet, TLS on 8883 recommended, with the broker CA in PEM form at hand. |
| Modbus master (optional) | Its IP address or subnet, because the controller only listens for an explicit allow-list. |
| NTP | From DHCP option 42 or a configured server. Without time the controller runs on its firmware build time, which is enough for its own TLS but not for log timestamps. |

The full port table is in [Network requirements](network.md).

## On the bench, before sealing

| Item | Why |
|---|---|
| A USB-C cable and a host with `espflash` or any serial terminal | The first firmware image is flashed over USB. The USB console is also where the recovery token is printed. |
| A firmware image built for the board (not a devboard build) | See [Building and flashing](../reference/building.md). |
| A label or a sheet to record **MAC address + recovery token** per board | That pair is the whole recovery path for a submerged controller. Record it the moment the console prints it. |
| The admin password you will set | There is no default password; nothing works over the network until one is set. |
| Optionally the fleet recovery public key | One ECDSA P-256 key pair per site lets you factory-reset any board over the network without its token. Generate it once with `granite-sim keygen`. |
| Optionally the firmware signing key | Only if you build with OTA signature verification enabled. Losing it means no more updates for the boards built against it. |

## Cable material

Everything that goes into the fluid has to survive the fluid. The
board follows the OCP immersion compatibility guidelines; the cables
and glands are the integrator's job. See
[Immersion constraints](immersion.md).
