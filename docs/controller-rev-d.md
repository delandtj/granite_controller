# Granite controller rev D - delta to rev C

**Status**: draft, 2026-10-29
**Base**: granite_controller/docs/controller.md (rev C, the full
component spec). Everything not named here is unchanged from rev C:
immersion rules, ESP32-C6 module, W5500 Ethernet and magjack, USB-C
service port, 5 V / 3V3 chain, 1-wire probes, TMP1075, heartbeat LED,
PCB stackup and fab rules.
**Firmware side**: firmware/docs/adr/0004-controller-rev-d-node-model.md
(decision 9 is the short form of this page).

## Purpose

Rev D is the controller for frames with power planes. Node control
(PWR / RST presses, power LED sense, rail switching) moved to the
planes' slots (docs/adr/0001-48v-power-plane.md, 0005), so rev C's node
section goes. What is left: Ethernet, the plane link master for up to
8 planes, probes, dry contacts, a sensor port. Powered from the
planes' isolated 12 V aux, never from the 48 V bus.

```
 network ==RJ45==> [ controller rev D ] ==PH 5p x2==> planes 0x30-0x37 (isolated I2C + EXP_INT)
 plane J4 12 V --PH 2p x2--^      |   |   \--4x PH 3p--> DS18B20 probes
                                  |   \--PH 5p--> dry contacts (leak, level, lid)
                                  \--PH 4p Qwiic--> sensors;  USB-C service (out of the fluid)
```

## Removed

- Node connectors J11-J18 (8x JST PH 5p).
- 16 TLP176AM photoMOS and their 270R / 100R resistors.
- U16 / U17 TLP290-4 LED sense, R39-R46 and the 220R series parts.
- U14 / U15 MCP23017 (relays, LED sense, dry inputs) and the
  EXP_nRESET_INT line (GPIO10).
- J8 expansion header (2x6) with its 330R / TPD4E05U06 / PTC network
  and EXP_nRESET (GPIO4). Its only named users were the LTC4282 power
  board (superseded by the planes) and add-on expanders (none exist).
  Sensors go on J9.
- The 19 V input: J3 (PH 2p), F3 PTC, D1 SMAJ22A, D8 SS34 (replaced
  below).

## Power input

- Two 12 V inputs, JST PH 2p each (B2B-PH-K-S), pin 1 CTRL_12V, pin 2
  CTRL_GND: the same pinout as the plane's J4 (power/pp-48 supplies.py),
  cable pin 1 to pin 1. One per plane, from two different planes when
  there are several (docs/plane-requirements.md R16; the controller
  survives the loss of one plane).
- OR-ing: one SS34 Schottky per input (also reverse polarity). At
  ~0.3 A typical and 0.6 A worst case the drop costs ~0.2 W; an ideal
  diode controller is not worth the parts. The higher of the two
  modules carries the load.
- TVS after the OR: SMAJ15A (15 V standoff, 24.4 V clamp, under the
  LMR51430's 36 V input and 38 V abs max).
- No fuse or PTC (R11). The plane's isolated module limits its own
  output current (6 W class); a short on the controller is that
  module's short-circuit case.
- Buck U8 LMR51430 and everything after it unchanged (4.5-36 V in).
- Budget: 3V3 ~0.6 A worst case (radio off: ~0.25 A typical) incl. the
  plane link supply below; ~2 W from 12 V, inside one 6 W module.
- Ground: CTRL_GND of both planes meet at the controller GND. That is
  the isolated side of each plane (iso module, ISO1640 side 1,
  ISO7721), never the bus ground (R20a).

## Plane link

- Bus: the ESP32-C6's HP I2C peripheral on GPIO2 / GPIO3 (was the
  internal bus to U14 / U15; hardware master with repeated start and
  a clock-stretch timeout, firmware ADR 0004). 100 kHz.
- Connectors: two JST PH 5p (B5B-PH-K-S), same bus on both, pin 1
  CTRL_3V3, 2 CTRL_GND, 3 SCL, 4 SDA, 5 EXP_INT: the plane J2 / J3
  pinout (power/pp-48 link.py), cable pin 1 to pin 1. Two connectors
  so a box can run two shorter daisy chains (half the capacitance
  each) instead of one long one.
- Pull-ups on the controller only (the planes' R28 / R29 are DNP):
  SDA / SCL 2.2k to 3V3. With ~0.5 nF on a long chain the rise time
  stays inside 1000 ns at 100 kHz; 1.5 mA sink is within the ISO1640
  side 1 rating (3.5 mA). Value to confirm on the bench with 8 planes.
- EXP_INT: GPIO0 (unchanged), 10k pull-up to 3V3; every plane pulls it
  low through its own open-drain Q1 (wired OR).
- 3V3 to the planes' isolator side 1 (ISO1640 + ISO7721, ~6 mA per
  plane, ~50 mA for 8): through one current-limited load switch per
  connector (~100-200 mA limit class, part to pick at LCSC), so a
  shorted cable does not drop the controller's 3V3.
- ESD on each connector's SCL / SDA / EXP_INT: TPD4E05U06 (as on rev
  C's J8 and J10), and a 33R series on SCL / SDA at the connector.

## I2C, local

- LP I2C pins GPIO6 / GPIO7 (were the external J8 bus): TMP1075 0x48
  and J9 (Qwiic, PH 4p: 1 GND, 2 3V3, 3 SDA, 4 SCL, with the rev C
  330R + ESD + PTC protection kept on J9 only). 100 kHz, 4.7k pull-ups.
- Rev C's internal / external split by function stays: a fault on J9
  cannot block the plane link.

## Dry-contact inputs

- J10 unchanged (JST PH 5p: 1-4 IN, 5 GND), 10k pull-up, 1k + 100 nF
  RC, TPD4E05U06 (U18).
- Now on MCU pins (U15 is gone): DRY_IN1-4 = GPIO10, GPIO11, GPIO4,
  GPIO15.
  - GPIO10 / GPIO11: free (EXP_nRESET_INT and the spare).
  - GPIO4 (MTMS): an SDIO timing strap, irrelevant here; the rev C 10k
    pull-down goes, the 10k pull-up of the input takes its place.
  - GPIO15: the JTAG-source strap, only read when the JTAG_SEL eFuse
    is burned (never, memory: devboard). A closed contact at boot is
    harmless.
  - GPIO8 stays free (download-mode strap).

## Bus voltage sense

- VIN_SENSE on GPIO5 (ADC1) now measures the OR-ed 12 V: divider 68k
  / 10k + 100 nF. 12 V -> 1.54 V; the SMAJ15A clamp (24.4 V) -> 3.13 V,
  inside the ADC range. Firmware scale changes accordingly.

## Pin map (rev D)

| Function | GPIO |
|---|---|
| USB D- / D+ | 12 / 13 |
| BOOT strap (test pad) | 9 |
| W5500 SCLK / MOSI / MISO / SCSn | 19 / 20 / 21 / 18 |
| W5500 INTn / RSTn | 22 / 23 |
| Plane link (HP I2C) SDA / SCL | 2 / 3 |
| Plane EXP_INT (in, 10k pull-up) | 0 |
| Local I2C (LP I2C: TMP1075, J9) SDA / SCL | 6 / 7 |
| 1-wire probes | 16 / 17 |
| Dry inputs 1-4 | 10 / 11 / 4 / 15 |
| VIN_SENSE (12 V, ADC1) | 5 |
| Status LED | 1 |
| Spare | 8 (strapping, 10k pull-up) |

## Connectors

| Connector | Type | Use |
|---|---|---|
| RJ45 | HR911105A magjack | Ethernet (unchanged) |
| USB-C | TYPE-C-31-M-12 | service, bench power (unchanged) |
| 12 V in A / B | JST PH 2p | from plane J4 |
| Plane link A / B | JST PH 5p | to plane J2, daisy chained from there |
| Probes 1-4 | JST PH 3p | DS18B20 (unchanged) |
| J9 | JST PH 4p | Qwiic sensors (unchanged pinout) |
| J10 | JST PH 5p | dry contacts (unchanged) |

## PCB

- Outline: set by the magjack, USB-C and 10 PH connectors along the
  edges; target ~100 x 50 mm (rev C 199 x 50). Antenna keep-out stays
  at a board edge.
- KiCad project: a new project in this repo (controller/), started
  from a copy of the rev C project with the node and J8 sheets
  removed; rev C itself stays untouched in granite_controller.
- 4 layers, JLC, same stackup and impedance rules as rev C.

## Open items

1. Load switch part for the plane link 3V3 (LCSC stock).
2. Pull-up value with 8 planes on two chains: bench measurement.
3. Mounting: where the controller sits relative to the planes (in the
   fluid like rev C, or above it); sets the cable lengths and the
   connector side.
4. ESP32-C6 module: N8 as rev C, or N16 if stocked (the plane image
   adds ~60 KB to an OTA slot, N8 is enough).
