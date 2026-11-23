# Connectors and pinouts

Hardware reference for controller-fd and the chained CAN FD power
plane (one core-fd board, 1-4 unit-fd boards), as in the first batch.
Pin 1 is first in every table and is marked on the silkscreen. No board
of the batch has been measured yet; the pinouts below are those of the
schematics the boards were ordered from.

## Controller (controller-fd)

### Connector list

| Ref | Function | Part | Mate |
|---|---|---|---|
| J1 | Ethernet 10/100 | HanRun HR911105A magjack | RJ45 plug |
| J2 | USB-C service port | HRO TYPE-C-31-M-12 | USB-C plug |
| J4, J5, J6, J7 | DS18B20 probes | JST B3B-PH-K-S | PHR-3 housing |
| J9 | I2C sensor port | JST B4B-PH-K-S | PHR-4 housing |
| J10 | Dry-contact inputs | JST B5B-PH-K-S | PHR-5 housing |
| J15 | Plane link and 12 V supply | BOOMELE 2.54-2*3P bent header, keyed 2x3 2.54 mm box header, right angle, B side (LCSC C34945) | 2x3 IDC socket on 6-way 1.27 mm flat cable |

Crimp contact for the PH housings: SPH-002T-P0.5S (AWG 24..30). There
is no J3, J8 or J11..J14 on controller-fd.

### J15 - plane link and power

| Pin | Signal |
|---|---|
| 1 | CANH |
| 2 | CANL |
| 3 | LINK_GND (the controller's ground) |
| 4 | LINK_GND |
| 5 | AUX_12V, about 12 V from the plane cores |
| 6 | AUX_12V |

The same pinout as the core's J2 / J3 and the units' J303 / J304; a
straight ribbon maps pin n to pin n. J15 is keyed, the plane-side
plugs are not.

CAN side: MCP2518FD CAN FD controller (40 MHz crystal) on the W5500's
SPI bus, TCAN1044V transceiver (5 V supply, 3.3 V logic), not
isolated: the controller's ground is LINK_GND. Fixed split termination
(2 x 60.4 ohm with a capacitor to LINK_GND) at this end of the bus.
PESD2CANFD24V ESD protection on CANH / CANL at J15. 500 kbit/s
arbitration, 2 Mbit/s data phase.

Power side: AUX_12V -> SS34 Schottky (reverse polarity) -> SMAJ15A TVS
(15 V standoff, 24.4 V clamp) -> LMR51430 buck (4.5-36 V in) -> 5 V,
then the 3.3 V buck. No fuse or PTC. The input is measured through a
68 kohm / 10 kohm divider on GPIO5 (ADC). USB VBUS is OR-ed into the
5 V rail through a second Schottky for the bench.

### J4 .. J7 - temperature probes

| Pin | Legend | Signal |
|---|---|---|
| 1 | `3V` | +3.3 V |
| 2 | `DQ` | 1-wire data, 4.7 kohm pull-up to 3.3 V |
| 3 | `G` | GND |

All four connectors are one bus on GPIO16/17 (UART0 pins, open-drain
TX plus RX on the same node). Up to eight probes are tracked by ROM id.

### J9 - I2C sensor port (Qwiic order)

| Pin | Legend | Signal |
|---|---|---|
| 1 | `G` | GND |
| 2 | `3V` | +3.3 V through a PTC |
| 3 | `DA` | Local I2C SDA, 330 ohm series, ESD |
| 4 | `CL` | Local I2C SCL, 330 ohm series, ESD |

Local bus: 100 kHz, 4.7 kohm pull-ups, GPIO6 (SDA) / GPIO7 (SCL),
shared with the on-board TMP1075 at 0x48.

### J10 - dry-contact inputs

| Pin | Legend | Signal |
|---|---|---|
| 1 | `1` | DRY_IN1 (GPIO10) |
| 2 | `2` | DRY_IN2 (GPIO11) |
| 3 | `3` | DRY_IN3 (GPIO4) |
| 4 | `4` | DRY_IN4 (GPIO15) |
| 5 | `G` | GND |

Each input: 10 kohm pull-up to 3.3 V, 1 kohm + 100 nF filter, ESD
array (TPD4E05U06). Closed = pulled to GND. GPIO4 and GPIO15 are
strapping pins whose straps are not used on this board; a closed
contact at boot is harmless.

### J2 - USB-C

USB 2.0 only: D+ / D- to the ESP32-C6 USB Serial/JTAG (GPIO13 / 12),
5.1 kohm CC pull-downs (UFP), ESD protection, VBUS OR-ed into the 5 V
rail through a Schottky diode so the board runs from USB on the bench.

### MCU pin map

| Function | GPIO |
|---|---|
| USB D- / D+ | 12 / 13 |
| BOOT strap (test pad) | 9 |
| SPI SCLK / MOSI / MISO (W5500 and MCP2518FD) | 19 / 20 / 21 |
| W5500 SCSn / INTn / RSTn | 18 / 22 / 23 |
| MCP2518FD nCS (10k pull-up) | 2 |
| MCP2518FD nINT (10k pull-up) | 3 |
| TCAN1044V STB (low = normal mode) | 0 |
| Local I2C (TMP1075, J9) SDA / SCL | 6 / 7 |
| 1-wire (J4..J7) | 16 / 17 |
| Dry inputs 1-4 | 10 / 11 / 4 / 15 |
| 12 V sense (ADC1) | 5 |
| Status LED | 1 |
| Spare | 8 (strapping pin, 10k pull-up) |

### I2C bus

| Bus | Devices | Addresses | Speed |
|---|---|---|---|
| Local (on-board and J9) | U5 TMP1075, sensors on J9 | 0x48, sensors | 100 kHz |

This is the controller's only I2C bus. The plane link is CAN FD on its
own SPI device, so a fault on J9 cannot block the plane link.

### Mechanical

- Outline 105 x 50 mm, 1.6 mm 4-layer FR-4.
- Four M3 holes 4 mm in from each corner.
- RJ45, USB-C and the probe connectors on the top long edge; J10 and
  J15 on the right-hand short edge, J15 on the B side with its shroud
  past the edge; J9 on the bottom long edge. The ESP32 module's antenna
  is on the bottom long edge.
- Test pads: EN, BOOT (GPIO9), 3V3, GND; no buttons.
- In a block the controller sits within 0.35 m of the plane core (the
  link ribbon's length, 1 m at most).

## Plane core board (core-fd)

68.0 x 72.0 mm. In a block it sits off the unit row and reaches unit 1
over two ribbons (see [Core ribbons](#core-ribbons)).

### Connector list

| Ref | Function | Part | LCSC |
|---|---|---|---|
| J1 | 48 V input | Amass XT30PW-M | C431092 |
| J2 | LINK IN (from the controller) | CJT A2541WR-2x3P, 2x3 2.54 mm right-angle pin header (plug), not keyed | C239334 |
| J3 | LINK OUT (to unit 1) | CJT A2541HWR-2x3P, 2x3 2.54 mm right-angle socket | C5333425 |
| J301 | Chain out (to unit 1) | CJT A2541HWR-2x7P, 2x7 2.54 mm right-angle socket, gold, B side | C19100281 |
| JP1 | Link termination | 2x2 2.54 mm pin header (PZ254V-12-4P) | C492419 |
| SW1 | Plane address | KingTek DSHP03TSGET, 3-way DIP | C50844 |
| J6 | SWD | Samtec FTSH-105-01-L-DV-K, ARM Cortex 10-pin 1.27 mm | C5155080 |
| J5 | SWD | Tag-Connect TC2030-NL pads (no part) | - |

### J1 - 48 V input (XT30)

| Pin | Signal |
|---|---|
| 1 | `-` bus GND |
| 2 | `+` VIN, 48-60 V |

Keyed (the only reverse-polarity protection). SMDJ60A bus TVS, input
capacitors, no fuse. Feeds the core's 3.3 V buck and the isolated
12 V for the link, nothing else.

### J2, J3 - plane link

| Pin | Signal |
|---|---|
| 1 | CANH |
| 2 | CANL |
| 3 | LINK_GND (isolated from the bus) |
| 4 | LINK_GND |
| 5 | AUX_12V |
| 6 | AUX_12V |

J2 and J3 are wired straight through; pin n mates pin n on every link
connector of every board. Behind them: an ISO1044 isolated CAN FD
transceiver (side 2 on the link, supplied by a 5 V regulator from this
core's isolated 12 V), an NUP2105L ESD diode pair on CANH / CANL at
each connector, and the core's feed onto AUX_12V.

The 12 V: an LM5018 Fly-Buck from VIN with a 1:1 coupled inductor
makes an isolated 12 V, which goes onto AUX_12V through a B5819W
Schottky diode. Every core on a link does the same, so the controller
runs while any one core runs. A 13 V TVS caps the isolated output when
nothing loads it; the green `12V` LED shows it. The controller draws
about 0.2 A at 12 V (about 2 W); that this stays within one core's
supply is to be confirmed on the first boards.

The CAN FD controller is an MCP2518FD on the MCU's SPI1 (G070 build,
the first batch); a G0B1 build would use the MCU's own FDCAN on the
same board. Data rates: 500 kbit/s arbitration, 2 Mbit/s data phase.

### JP1 - link termination

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | CANH | 2 | TERM_H |
| 3 | CANL | 4 | TERM_L |

A split termination (60.4 ohm per leg and a capacitor to LINK_GND),
switched by two jumper caps on 1-2 and 3-4. Caps go only on the last
board of the row, which in a block is unit 4 (JP301), never on a core
that has units after it.

### J301 - chain out (to unit 1)

2x7 socket, pins listed top to bottom as odd / even pin. The pinout is
the same for the plug and the socket (plug pin n mates socket pin n),
see [Chain and link joints](#chain-and-link-joints-unit-to-unit).

| Pins | Signal (odd) | Signal (even) |
|---|---|---|
| 1, 2 | EXP_RSTN | PRESS_EN |
| 3, 4 | P3 SDA (mux channel 3, unit 4) | P3 SCL |
| 5, 6 | P2 SDA (mux channel 2, unit 3) | P2 SCL |
| 7, 8 | GND | GND |
| 9, 10 | P1 SDA (mux channel 1, unit 2) | P1 SCL |
| 11, 12 | P0 SDA (mux channel 0, unit 1) | P0 SCL |
| 13, 14 | +3V3 | GND |

### SW1 - plane address

| Switch | Strap | MCU pin |
|---|---|---|
| 1 | ADR0 | PA15 |
| 2 | ADR1 | PD1 |
| 3 | ADR2 | PA8 |

Each strap has a pull-up; the switch closes it to GND. ON = 0, OFF =
1. Plane address = 0x30 + ADR[2:0]: all ON = 0x30, all OFF = 0x37. On
the CAN FD link the plane's node id is ADR[2:0] (0-7); the controller
is node 14.

### J6 - SWD, ARM Cortex 10-pin

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | +3V3 (VTref) | 2 | SWDIO |
| 3 | GND | 4 | SWCLK |
| 5 | GND | 6 | not connected (SWO) |
| 7 | not connected (key) | 8 | not connected |
| 9 | GND | 10 | NRST |

### J5 - SWD, Tag-Connect TC2030-NL

| Pin | Signal |
|---|---|
| 1 | +3V3 |
| 2 | SWDIO |
| 3 | NRST |
| 4 | SWCLK |
| 5 | GND |
| 6 | not connected |

### Core LEDs

| Ref | Label | Meaning |
|---|---|---|
| D3 | `3V3` | core 3.3 V present (also feeds the units' logic) |
| D4 | `RUN` | firmware heartbeat |
| D5 | `12V` | isolated 12 V for the link present (controller side) |

### Core I2C

| Bus | Devices | Addresses |
|---|---|---|
| Plane (I2C2, master) | TCA9548A mux, then one unit segment per channel | 0x70 |

The link to the controller is CAN FD (above), not I2C.

## Plane unit board (unit-fd)

78.6 x 72.1 mm.

### Connector list

| Ref | Function | Part | LCSC |
|---|---|---|---|
| J201 | 48 V input | Amass XT60PW-M | C98732 |
| J301 | Chain in (left edge) | CJT A2541WR-2x7P, 2x7 2.54 mm right-angle pin header (plug), gold, B side | C5136987 |
| J302 | Chain out (right edge) | CJT A2541HWR-2x7P, 2x7 2.54 mm right-angle socket, gold, B side | C19100281 |
| J303 | Link in (left edge, beside J301) | CJT A2541WR-2x3P, 2x3 2.54 mm right-angle pin header (plug) | C239334 |
| J304 | Link out (right edge, beside J302) | CJT A2541HWR-2x3P, 2x3 2.54 mm right-angle socket | C5333425 |
| JP301 | Link termination | 2x2 2.54 mm pin header (PZ254V-12-4P) | C492419 |
| J1, J51 | Node 19 V, slot A, B | Degson DG128-5.0-02P | C711349 |
| J2, J52 | Front panel, slot A, B | JST SM04B-GHS-TB (GH, 1.25 mm) | C189895 |
| J3, J53 | Module aux, slot A, B | hanxia HX PM1.27-2x3P socket | C41397112 |
| SO1-SO4, SO51-SO54 | Module mount, slot A, B | Sinhoo SMTSO2070CTJ, M2, 7 mm | C2916355 |

Slot B references are slot A's + 50.

### J201 - 48 V input (XT60)

| Pin | Signal |
|---|---|
| 1 | `-` bus GND |
| 2 | `+` VIN, 48-60 V |

Keyed. SMDJ60A TVS, input capacitors, then the unit hot-swap: LM5069-2
(auto-retry), 3 mOhm shunt (current limit about 18 A), two
BSC040N10NS5 in parallel, UVLO about 38 V, OVLO about 65 V, INA237 at
0x44 on the unit segment (its VBUS pin reads the hot-swap output, not
the XT60 side). The limiter has no latch: after a fault it restarts
every two seconds or so, for ever, which keeps the FETs inside their
safe area and keeps a plane with no core alive. The latch is the plane
core's, over the expander's U_OFF line, and `unit_reset` releases it
(see [Recovery](../operation/recovery.md)).

### J301 - chain in (from the core side)

2x7 plug, same pinout as the socket (odd = row farther from the edge,
even = row nearer the edge). IN P0 is this unit's own segment. On
unit 1 it takes the W_CHAIN ribbon from the core.

| Pins | Signal (odd) | Signal (even) |
|---|---|---|
| 1, 2 | EXP_RSTN | PRESS_EN |
| 3, 4 | P3 SDA | P3 SCL |
| 5, 6 | P2 SDA | P2 SCL |
| 7, 8 | GND | GND |
| 9, 10 | P1 SDA | P1 SCL |
| 11, 12 | P0 SDA (this unit's segment) | P0 SCL |
| 13, 14 | +3V3 (from the core) | GND |

### J302 - chain out (to the next unit)

2x7 socket, the same pinout as the plug and as the core's J301. The
unit shifts the segments by one: its IN P1..P3 leave as OUT P0..P2, and
OUT P3 (pins 3 and 4) is not connected. +3V3, GND, PRESS_EN and
EXP_RSTN pass straight through. That is how the next unit finds its own
segment on P0 without any setting.

| Pins | Signal (odd) | Signal (even) |
|---|---|---|
| 1, 2 | EXP_RSTN | PRESS_EN |
| 3, 4 | not connected (P3 SDA) | not connected (P3 SCL) |
| 5, 6 | P2 SDA (= IN P3) | P2 SCL (= IN P3) |
| 7, 8 | GND | GND |
| 9, 10 | P1 SDA (= IN P2) | P1 SCL (= IN P2) |
| 11, 12 | P0 SDA (= IN P1) | P0 SCL (= IN P1) |
| 13, 14 | +3V3 | GND |

### J303, J304 - plane link in / out

The same pinout as the core's J2 / J3 and the controller's J15:

| Pin | Signal |
|---|---|
| 1 | CANH |
| 2 | CANL |
| 3 | LINK_GND |
| 4 | LINK_GND |
| 5 | AUX_12V |
| 6 | AUX_12V |

Straight through, no parts on the unit except the switched termination
on JP301. On unit 1, J303 takes the W_CORELINK ribbon from the core.
On the last unit of the row J304 stays empty.

### JP301 - link termination

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | CANH | 2 | TERM_H |
| 3 | CANL | 4 | TERM_L |

Jumper caps on 1-2 and 3-4 on the last unit of the row only (unit 4 in
a block); see [Termination](#termination).

### J1, J51 - node 19 V

| Pin | Signal |
|---|---|
| 1 | GND |
| 2 | +19 V from the port module (250 W continuous, 13.2 A) |

5.0 mm pitch screw terminal, 14 AWG. The node end is a barrel plug
into the node's DC jack (DC-045D on the AXB35 drawn); the plug is not
chosen yet and the jack's polarity is to be metered.

### J2, J52 - front panel (JST GH 4p)

| Pin | Signal | Unit side |
|---|---|---|
| 1 | PWR_SW | TLP176AM photoMOS output to GND |
| 2 | RST_SW | TLP176AM photoMOS output to GND |
| 3 | PLED+ | TLP291 optocoupler LED anode through 220 ohm |
| 4 | PLED- | TLP291 optocoupler LED cathode |

The photoMOS LEDs are driven by the unit's TCA6424A (0x22) and return
through a FET gated by PRESS_EN: no PRESS_EN, no press. Mate: GHR-04V-S
housing, SSHL-002T-P0.2 contacts. The motherboard end of the cable and
the motherboard's front-panel pinout are not confirmed.

### J3, J53 - module aux socket (2x3, 1.27 mm)

| Pin | Signal | Direction |
|---|---|---|
| 1 | OV_KILL | module -> unit, open drain, low = overvoltage, sets the slot latch |
| 2 | PGOOD | module -> unit, open drain, high = good |
| 3 | ID | module -> unit, resistor to GND (open = no module) |
| 4 | RUN | unit -> module, open = on, pulled low = off |
| 5 | GND | |
| 6 | GND | |

### Module mount

| Standoff | Net |
|---|---|
| SO1 (S1) | VIN to the module (slot output) |
| SO2 (S2) | GND, input end |
| SO3 (S3) | GND, output end |
| SO4 (S4) | VOUT 19 V (to J1 pin 2) |

M2 x 5 A2 screws with split lock washers; torque 0.15 Nm at
most (the standoff's limit for M2, docs/pm-interface.md). The pattern is polarised: a module turned 180
degrees takes no screw.

### Unit I2C segment

| Device | Address |
|---|---|
| TCA6424A expander (both slots, unit hot-swap control) | 0x22 |
| INA238 slot A / slot B | 0x40 / 0x41 |
| INA238 unit input | 0x44 |
| ADC121C027 module ID, slot A / slot B | 0x51 / 0x52 |

Every unit carries the same addresses; the core's mux puts each unit on
its own channel.

## Chain and link joints (unit to unit)

There is no cable between units. Neighbouring units plug into each
other, in the plane of the boards, through two joints side by side:
the 2x7 chain and the 2x3 link.

A unit's J301 (left edge) is a right-angle 2x7 2.54 mm pin header,
CJT A2541WR-2x7P (C5136987, gold), on the B side with its body flush
with the board's left edge and the pins 6 mm past it. A unit's J302
(right edge) and the core's J301 are the matching right-angle 2x7
socket, CJT A2541HWR-2x7P (C19100281, gold), on the B side with its
entry face 4 mm past the board's right edge. The link joint beside it
is the same family in 2x3: plug J303 (A2541WR-2x3P) on the left,
socket J304 (A2541HWR-2x3P) on the right, same height and face rules,
so one push mates both joints.

Plug pin n mates socket pin n, so the pinouts above are the same on
both sides. The chain pins sit at 42.5 to 57.74 mm from the board's
top edge, the link's pin 1 at 30.0 mm, on every board, so plug and
socket line up.

Fully mated the boards stand 4 mm apart; the joints work from 4 to 6
mm gap (6 to 4 mm of the pins inside the socket). The boards sit on a
common mounting bar and are screwed down to it. The bar holds the
boards: the connectors carry no load and have no latch. To remove a
middle unit, slide the boards after it 6 mm away first.

## Core ribbons

The core sits off the unit row, so its chain and link reach unit 1
over two ribbons, both pin n to pin n, red stripe on pin 1 at both
ends, about 0.15 m (0.10-0.15 m from the pod layout, to measure on the
first block). The core's J301 and J3 are sockets, so each ribbon has a
male IDC plug at the core and an IDC socket at unit 1.

| Ribbon | From | To | Cable | Core end | Unit end |
|---|---|---|---|---|---|
| W_CHAIN | core J301 | unit 1 J301 | 14-way, 1.27 mm, 28 AWG, halogen-free (3M HF365/14SF) | 2x7 male IDC plug | 2x7 IDC socket, Wurth 61201423021 |
| W_CORELINK | core J3 LINK OUT | unit 1 J303 | 6-way, same cable (3M HF365/06SF) | 2x3 male IDC plug | 2x3 IDC socket, Wurth 61200623021 |

W_CHAIN carries the chain pinout of [J301](#j301---chain-out-to-unit-1),
W_CORELINK the link pinout of [J2, J3](#j2-j3---plane-link).

**Open**: the male IDC plugs (2x7 and 2x3, for 1.27 mm flat cable) are
not found yet. They have to fit the core's right-angle sockets side by
side, 7.4 mm between their end pins: try the fit first. The current per
chain pin is still to be checked against 28 AWG.

## Link ribbon (W_LINK)

Controller J15 to the core's J2 (LINK IN): 6-way halogen-free flat
cable, 1.27 mm pitch, 28 AWG (3M HF365/06SF), about 0.35 m, 1 m at
most. A 2x3 IDC socket with key and strain relief at both ends (Wurth
61200623021), stripe on pin 1, pin n to pin n:

| Pin | Signal |
|---|---|
| 1 | CANH |
| 2 | CANL |
| 3 | LINK_GND |
| 4 | LINK_GND |
| 5 | AUX_12V (about 0.2 A) |
| 6 | AUX_12V |

The controller end is keyed (J15 is a box header). The core end is a
bare plug and is not: mark pin 1 there. A cable turned round at the
core puts AUX_12V on CANH / CANL; both transceivers survive it (their
bus pins take +-58 V), but the link stays down and the controller has
no power.

## Termination

Two standard 2.54 mm jumper caps (for example Wurth 60900213421) on
the last board of the row: JP301 of the last unit, pins 1-2 (CANH) and
3-4 (CANL). In a block that is unit 4. None on any other board, core
or unit. The controller terminates the other end of the bus on the
board, with no setting.
