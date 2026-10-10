# Connectors and pinouts

Hardware reference for rev C. Pin 1 is first in every table and is
marked on the silkscreen.

## Connector list

| Ref | Function | Part | Mate |
|---|---|---|---|
| J1 | Ethernet 10/100 | HanRun HR911105A magjack | RJ45 plug |
| J2 | USB-C service port | HRO TYPE-C-31-M-12 | USB-C plug |
| J3 | 19 V input | JST B2B-PH-K-S | PHR-2 housing |
| J4, J5, J6, J7 | DS18B20 probes | JST B3B-PH-K-S | PHR-3 housing |
| J8 | Expansion header | 2x6 pin 2.54 mm, DNP | 2x6 IDC or housing |
| J9 | I2C sensor port | JST B4B-PH-K-S | PHR-4 housing |
| J10 | Dry-contact inputs | JST B5B-PH-K-S | PHR-5 housing |
| J11 .. J18 | Node 1 .. 8 | JST B5B-PH-K-S | PHR-5 housing |

Crimp contact for all PH housings: SPH-002T-P0.5S (AWG 24..30).

## J3 - 19 V input

| Pin | Legend | Signal |
|---|---|---|
| 1 | `+` | +19 V nominal (22 V TVS standoff, 35.5 V clamp; reverse-polarity protected) |
| 2 | `-` | GND |

Protection chain: 0.75 A hold PTC, SMAJ22A TVS, SS34 series Schottky.
Bus voltage is measured through a 100k/10k divider on GPIO5 (ADC).

## J4 .. J7 - temperature probes

| Pin | Legend | Signal |
|---|---|---|
| 1 | `3V` | +3.3 V |
| 2 | `DQ` | 1-wire data, 4.7 kohm pull-up to 3.3 V |
| 3 | `G` | GND |

All four connectors are one bus on GPIO16/17 (UART0 pins, open-drain
TX plus RX on the same node). Up to eight probes are tracked by ROM id.

## J9 - I2C sensor port (Qwiic order)

| Pin | Legend | Signal |
|---|---|---|
| 1 | `G` | GND |
| 2 | `3V` | +3.3 V through a PTC |
| 3 | `DA` | I2C_EXT SDA, 330 ohm series, ESD |
| 4 | `CL` | I2C_EXT SCL, 330 ohm series, ESD |

External bus: 100 kHz, 4.7 kohm pull-ups, GPIO6 (SDA) / GPIO7 (SCL).
Reserved addresses: 0x21..0x27 add-on MCP23017, 0x40..0x47 LTC4282.

## J10 - dry-contact inputs

| Pin | Legend | Signal |
|---|---|---|
| 1 | `1` | DRY_IN1 |
| 2 | `2` | DRY_IN2 |
| 3 | `3` | DRY_IN3 |
| 4 | `4` | DRY_IN4 |
| 5 | `G` | GND |

Each input: 10 kohm pull-up to 3.3 V, 1 kohm + 100 nF filter, ESD
diode, read by expander U15 port B0..B3. Closed = pulled to GND.

## J11 .. J18 - node connectors

| Pin | Legend | Signal | Controller side |
|---|---|---|---|
| 1 | `PW` | PWR_SW | photoMOS output, 100 ohm series, floating |
| 2 | `RS` | RST_SW | photoMOS output, 100 ohm series, floating |
| 3 | `CM` | Switch common | the other side of both photoMOS outputs |
| 4 | `L+` | PLED+ | AC-input optocoupler through 220 ohm |
| 5 | `L-` | PLED- | AC-input optocoupler through 220 ohm |

Relays: TLP176AM photoMOS, two per node (PWR and RST), driven by
expander U14 at 0x20 (port A = PWR 1..8, port B = RST 1..8). U14 is
held in reset by a pull-down on its reset line, so every contact is
open whenever the MCU is in reset, unpowered or not driving the line.
Sense: TLP290-4 optocouplers into expander U15 at 0x21 with 47 kohm
pull-ups; the firmware reads LED lit = node on.

## J8 - expansion header (not populated)

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | +3.3 V (PTC + TVS) | 2 | +5 V (PTC + TVS, up to 0.5 A) |
| 3 | GND | 4 | GND |
| 5 | I2C_EXT SCL | 6 | I2C_EXT SDA |
| 7 | EXP_nRESET (GPIO4, pull-down) | 8 | EXP_INT (GPIO0, shared) |
| 9 | unused | 10 | GPIO8 (strapping pin, 10k pull-up) |
| 11 | GPIO15 (strapping pin) | 12 | GND |

Every signal passes 330 ohm and a TPD4E05U06 ESD array. GPIO8 and
GPIO15 are ESP32-C6 strapping pins: anything attached must be high
impedance at boot. On add-on MCP23017 modules, GPA7 and GPB7 are
output-only and must not be used as inputs.

## J2 - USB-C

USB 2.0 only: D+ / D- to the ESP32-C6 USB Serial/JTAG (GPIO13 / 12),
5.1 kohm CC pull-downs (UFP), USBLC6-2 ESD, VBUS OR-ed into the 5 V
rail through a Schottky diode with a bleeder so a host sees vSafe0V
when the board runs from 19 V.

## MCU pin map

| Function | GPIO |
|---|---|
| USB D- / D+ | 12 / 13 |
| BOOT strap (test pad) | 9 |
| W5500 SCLK / MOSI / MISO / CSn | 19 / 20 / 21 / 18 |
| W5500 INTn / RSTn | 22 / 23 |
| Internal I2C SDA / SCL (U14, U15, TMP1075) | 2 / 3 |
| External I2C SDA / SCL (J8, J9) | 6 / 7 |
| 1-wire (J4..J7) | 16 / 17 |
| Relay and sense expander reset (EXP_nRESET_INT) | 10 |
| 19 V sense (ADC1) | 5 |
| Header expander reset (EXP_nRESET) | 4 |
| Shared expander interrupt (EXP_INT) | 0 |
| Status LED | 1 |
| Spare to J8 | 8, 15 |
| Unconnected | 11 |

## I2C buses

| Bus | Devices | Addresses | Speed |
|---|---|---|---|
| Internal (on-board only) | U14 relays, U15 sense, U5 TMP1075 | 0x20, 0x21, 0x48 | 400 kHz |
| External (J8, J9) | add-on MCP23017, power board LTC4282 x8, sensors | 0x21..0x27, 0x40..0x47 | 100 kHz |

The two buses share no pins. A fault on the external bus cannot stop
node control.

## Mechanical

- Outline 199 x 50 mm, 1 mm corner radius, 1.6 mm 4-layer FR-4.
- Four M3 non-plated holes 4 mm in from each corner, with 6.5 mm
  copper keep-outs.
- The ESP32 module antenna overhangs the bottom long edge; the RJ45 and
  USB-C openings face the top long edge.
- Test pads: EN, BOOT (GPIO9), and two more; no buttons.
