# Connectors and pinouts

Hardware reference for controller rev D and the chained power plane
(one core board, 1-4 unit boards). Pin 1 is first in every table and is
marked on the silkscreen. The plane boards are not final at the time of
writing; the pinouts below are those of the current schematics.

## Controller rev D

### Connector list

| Ref | Function | Part | Mate |
|---|---|---|---|
| J1 | Ethernet 10/100 | HanRun HR911105A magjack | RJ45 plug |
| J2 | USB-C service port | HRO TYPE-C-31-M-12 | USB-C plug |
| J4, J5, J6, J7 | DS18B20 probes | JST B3B-PH-K-S | PHR-3 housing |
| J9 | I2C sensor port | JST B4B-PH-K-S | PHR-4 housing |
| J10 | Dry-contact inputs | JST B5B-PH-K-S | PHR-5 housing |
| J11 | 12 V input A | JST B2B-PH-K-S | PHR-2 housing |
| J12 | 12 V input B | JST B2B-PH-K-S | PHR-2 housing |
| J13 | Plane link A | JST B5B-PH-K-S | PHR-5 housing |
| J14 | Plane link B | JST B5B-PH-K-S | PHR-5 housing |

Crimp contact for all PH housings: SPH-002T-P0.5S (AWG 24..30). Rev C's
J3 (19 V input), J8 (expansion header) and J11..J18 (node connectors)
do not exist on rev D; J11 and later are new functions.

### J11, J12 - 12 V inputs

| Pin | Legend | Signal |
|---|---|---|
| 1 | `+` | +12 V from a plane core's J4 (CTRL_12V) |
| 2 | `-` | GND (the plane's isolated CTRL_GND) |

Each input: SS34 series Schottky (OR-ing and reverse polarity). After
the OR: SMAJ15A TVS (15 V standoff, 24.4 V clamp), then the LMR51430
buck (4.5-36 V in). No fuse or PTC. The OR-ed input is measured
through a 68k / 10k divider on GPIO5 (ADC).

### J13, J14 - plane link

| Pin | Legend | Signal |
|---|---|---|
| 1 | `3V` | 3.3 V to the planes' isolator side 1, through a current-limited switch per connector |
| 2 | `G` | GND |
| 3 | `CL` | SCL, 33 ohm series, ESD |
| 4 | `DA` | SDA, 33 ohm series, ESD |
| 5 | `IN` | EXP_INT, 10 kohm pull-up, ESD; planes pull low (wired OR) |

J13 and J14 are the same bus (HP I2C on GPIO2 SDA / GPIO3 SCL, 100
kHz, 2.2 kohm pull-ups on the controller only) and the same EXP_INT
(GPIO0). The current-limited switches are TPS2553 parts in the
schematic; the pull-up value is to be confirmed on the bench with eight
planes. Same pinout as the plane core's J2 / J3, cable pin 1 to pin 1.

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
| W5500 SCLK / MOSI / MISO / SCSn | 19 / 20 / 21 / 18 |
| W5500 INTn / RSTn | 22 / 23 |
| Plane link (HP I2C) SDA / SCL | 2 / 3 |
| Plane EXP_INT (input, 10k pull-up) | 0 |
| Local I2C (TMP1075, J9) SDA / SCL | 6 / 7 |
| 1-wire (J4..J7) | 16 / 17 |
| Dry inputs 1-4 | 10 / 11 / 4 / 15 |
| 12 V sense (ADC1) | 5 |
| Status LED | 1 |
| Spare | 8 (strapping pin, 10k pull-up) |

### I2C buses

| Bus | Devices | Addresses | Speed |
|---|---|---|---|
| Plane link (J13, J14), isolated on each plane | plane cores | 0x30..0x37 | 100 kHz |
| Local (on-board and J9) | U5 TMP1075, sensors on J9 | 0x48, sensors | 100 kHz |

The two buses share no pins. A fault on J9 cannot block the plane link.

### Mechanical

- Outline 105 x 50 mm, 1.6 mm 4-layer FR-4.
- Four M3 holes 4 mm in from each corner.
- RJ45, USB-C and the probe connectors on the top long edge; 12 V
  inputs and plane link connectors on the bottom long edge, beside the
  ESP32 module's antenna; J10 and J9 on the right-hand short edge.
- Test pads: EN, BOOT (GPIO9), 3V3, GND; no buttons.
- Mounting position relative to the planes (in the fluid or above it)
  is still open.

## Plane core board

### Connector list

| Ref | Function | Part | LCSC |
|---|---|---|---|
| J1 | 48 V input | Amass XT30PW-M | C431092 |
| J2 | Controller link in | JST B5B-PH-K-S | C157993 |
| J3 | Controller link out | JST B5B-PH-K-S | C157993 |
| J4 | 12 V aux out, isolated | JST B2B-PH-K-S | C131337 |
| J301 | Chain out | JST S14B-PH-K-S (side entry) | C157938 |
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
12 V aux, nothing else.

### J2, J3 - controller link

| Pin | Signal |
|---|---|
| 1 | CTRL_3V3 (from the controller, supplies isolator side 1) |
| 2 | CTRL_GND |
| 3 | SCL (controller side) |
| 4 | SDA (controller side) |
| 5 | EXP_INT (open drain, pulled low by this plane) |

J2 and J3 are wired straight through (daisy chain). Behind them: an
ISO1640 I2C isolator and an ISO7721 for EXP_INT. The controller-side
pull-ups R28 / R29 are not fitted; the controller carries them.

### J4 - 12 V aux out

| Pin | Signal |
|---|---|
| 1 | CTRL_12V, about 12 V, isolated |
| 2 | CTRL_GND |

LM5018 Fly-Buck from VIN with a 1:1 coupled inductor, about 100 mA
(about 1.2 W). Starts at about 30 V bus. A 13 V TVS caps the output
when no controller is connected; the green `12V` LED is the preload.

### J301 - chain out (to unit 1)

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | EXP_RSTN | 8 | GND |
| 2 | PRESS_EN | 9 | P1 SCL (mux channel 1, unit 2) |
| 3 | P3 SCL (mux channel 3, unit 4) | 10 | P1 SDA |
| 4 | P3 SDA | 11 | GND |
| 5 | GND | 12 | P0 SCL (mux channel 0, unit 1) |
| 6 | P2 SCL (mux channel 2, unit 3) | 13 | P0 SDA |
| 7 | P2 SDA | 14 | +3V3 |

This is the OUT pinout; see [Chain cable](#chain-cable) for how it
meets a unit's IN header.

### SW1 - plane address

| Switch | Strap | MCU pin |
|---|---|---|
| 1 | ADR0 | PA15 |
| 2 | ADR1 | PD1 |
| 3 | ADR2 | PA8 |

Each strap has a 100 kohm pull-up; the switch closes it to GND. ON = 0,
OFF = 1. Plane I2C address = 0x30 + ADR[2:0]: all ON = 0x30, all OFF =
0x37.

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
| D5 | `12V` | isolated 12 V aux present (controller side) |

### Core I2C

| Bus | Devices | Addresses |
|---|---|---|
| Link (I2C1, slave, through the ISO1640) | this plane | 0x30 + ADR[2:0] |
| Plane (I2C2, master) | TCA9548A mux, then one unit segment per channel | 0x70 |

## Plane unit board

### Connector list

| Ref | Function | Part | LCSC |
|---|---|---|---|
| J201 | 48 V input | Amass XT60PW-M | C98732 |
| J301 | Chain in (left edge) | JST S14B-PH-K-S (side entry) | C157938 |
| J302 | Chain out (right edge) | JST S14B-PH-K-S (side entry) | C157938 |
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

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | +3V3 (from the core) | 8 | P2 SDA |
| 2 | P0 SDA (this unit's segment) | 9 | P2 SCL |
| 3 | P0 SCL | 10 | GND |
| 4 | GND | 11 | P3 SDA |
| 5 | P1 SDA | 12 | P3 SCL |
| 6 | P1 SCL | 13 | PRESS_EN |
| 7 | GND | 14 | EXP_RSTN |

### J302 - chain out (to the next unit)

The OUT pinout, as on the core's J301. The unit shifts the segments by
one: its IN P1..P3 leave as OUT P0..P2, and OUT P3 is not connected.
+3V3, GND, PRESS_EN and EXP_RSTN pass straight through. That is how the
next unit finds its own segment on P0 without any setting.

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | EXP_RSTN | 8 | GND |
| 2 | PRESS_EN | 9 | P1 SCL (= IN P2) |
| 3 | not connected (P3 SCL) | 10 | P1 SDA (= IN P2) |
| 4 | not connected (P3 SDA) | 11 | GND |
| 5 | GND | 12 | P0 SCL (= IN P1) |
| 6 | P2 SCL (= IN P3) | 13 | P0 SDA (= IN P1) |
| 7 | P2 SDA (= IN P3) | 14 | +3V3 |

### J1, J51 - node 19 V

| Pin | Signal |
|---|---|
| 1 | GND |
| 2 | +19 V from the port module (250 W continuous, 13.2 A) |

5.0 mm pitch screw terminal, 14 AWG.

### J2, J52 - front panel (JST GH 4p)

| Pin | Signal | Unit side |
|---|---|---|
| 1 | PWR_SW | TLP176AM photoMOS output to GND |
| 2 | RST_SW | TLP176AM photoMOS output to GND |
| 3 | PLED+ | TLP291 optocoupler LED anode through 220 ohm |
| 4 | PLED- | TLP291 optocoupler LED cathode |

The photoMOS LEDs are driven by the unit's TCA6424A (0x22) and return
through a FET gated by PRESS_EN: no PRESS_EN, no press. Mate: GHR-04V-S
housing, SSHL-002T-P0.2 contacts.

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

M2 x 5 A2 screws with split lock washers; torque 0.25 Nm, to be
confirmed on a sample. The pattern is polarised: a module turned 180
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

## Chain cable

Two PHR-14 housings, 14 wires, SPH-002T-P0.5S contacts, about 50-100
mm, flat. The facing headers are turned 180 degrees to each other, so
**wire k runs from pin k at one end to pin 15 - k at the other**: an
OUT header's pin 14 (+3V3) meets the IN header's pin 1 (+3V3), OUT pin
13 (P0 SDA) meets IN pin 2, and so on. Every joint uses the same cable:
core J301 or unit J302 (OUT) to the next unit's J301 (IN).

## Link cable

JST PH 5-pin to 5-pin, pin 1 to pin 1: controller J13 / J14 to a plane
core's J2, or a plane core's J3 to the next core's J2.

| Pin | Signal |
|---|---|
| 1 | 3V3 (controller side) |
| 2 | GND (controller side) |
| 3 | SCL |
| 4 | SDA |
| 5 | EXP_INT |

## 12 V aux cable

JST PH 2-pin to 2-pin, pin 1 to pin 1: a plane core's J4 to controller
J11 or J12.

| Pin | Signal |
|---|---|
| 1 | +12 V |
| 2 | GND (controller side) |
