# Granite controller - component spec

Status: draft, 2026-10-07. Supersedes the STM32H563 + LAN8742A design
on this branch; the schematic has not been reworked yet. Open items
are listed at the end; everything else is a requirement.

## Purpose

Remote reset/power control and monitoring for one frame of 8
motherboards (nodes), immersion-cooled. One PoE Ethernet cable powers
and controls the board. It presses each node's front-panel RESET and
PWR buttons, reads temperature probes, and talks to optional add-on
modules and the power distribution board (docs/power-board.md).

## Context

```
 network/PoE ==RJ45==> [ controller ] --8x JST-PH 4p--> node front panels
                          |   |   \--4x JST-PH 3p--> DS18B20 probes
                          |   \--J8 expansion header (I2C ext) --> power board,
                          |                                         add-on expanders
                          \--USB-C (service only, out of the fluid)
```

## Requirements

### Environment: immersion

The board runs fully submerged in dielectric fluid: single-phase
(hydrocarbon) now, possibly two-phase (fluorinated, boiling ~34-61 C)
later. Choose materials for the harsher case now.

1. No aluminium electrolytic or aluminium-polymer capacitors with
   rubber end seals. Use MLCC, or molded tantalum-polymer where bulk
   capacitance needs it.
2. No silicone (potting, LED lenses, thermal pads, cable jackets)
   unless soak-tested in the target fluid. Epoxy-potted modules are
   preferred.
3. No PVC cable jackets. Connector housings in PA, PBT or LCP.
4. No conformal coating, no adhesive labels. Silkscreen is cosmetic only;
   anything that must stay readable is in copper or laser marking.
5. Avoid trapped air: no parts with sealed cavities or vents facing
   up, and no unfilled under-part voids where avoidable.
6. Controlled-impedance pairs are calculated with the fluid as the
   surrounding medium (Er ~2.1 for oil, check the two-phase fluid), not
   air.
7. Every part with an uncertain material gets a soak test in the
   target fluid before production (OCP immersion component compatibility
   guidelines v1.5: >= 500 h at ~1.2x operating temperature).
8. Cable exits from the tank are the integrator's job, but the board
   assumes solid-conductor or potted-gland cables (oil wicks along
   stranded conductors).

### Power

- PoE 802.3af PD, Class 0, via the ARJP11A magjack (integrated
  rectifier) and the Silvertel Ag9905MT module -> 5 V -> TLV62569 buck
  -> 3V3 (unchanged from the current design).
- 3V3 budget: ESP32-C6 module up to ~0.4 A peak (radio TX; normally
  off), W5500 ~0.13 A, relay expander plus LEDs ~0.1 A, rest < 0.05 A.
  Total under 0.7 A against the 2 A buck.
- The Ag9905MT potting must be confirmed epoxy (requirement 2) or the
  module soak-tested.
- C1/C2 (220 uF aluminium-polymer, rubber seal) are replaced with
  MLCC or molded tantalum-polymer of equivalent capacitance and ripple
  rating.
- USB-C VBUS can power the board for bench service (existing D2/D3
  OR-ing kept).

### MCU

- Espressif ESP32-C6-WROOM-1U-N8 (RISC-V, 8 MB flash, U.FL antenna
  connector left unused). Fallback ESP32-C6-WROOM-1-N8 (PCB antenna;
  then keep its antenna keep-out free of copper).
- Radio off in firmware (useless submerged; the fluid detunes the
  antenna anyway).
- EN: 10k pull-up + 1 uF. BOOT (GPIO9): weak pull-up in the chip; test
  pad to GND for forced download mode. EN also on a test pad. No
  buttons (nobody presses buttons in a tank).
- Decoupling at the module 3V3 pin: 10 uF + 100 nF.
- Firmware updates over the network (OTA) in normal operation; USB
  Serial/JTAG over USB-C for bench flashing, console and debug. The
  Tag-Connect SWD footprint (J3) goes away.
- KiCad has no symbol/footprint for the WROOM-1/-1U: create both from
  the Espressif datasheet (28 pads + ground pad).

Pin map:

| Function | GPIO |
|---|---|
| USB D- / D+ | 12 / 13 |
| BOOT strap (test pad) | 9 |
| W5500 SCLK / MOSI / MISO / SCSn | 19 / 20 / 21 / 18 |
| W5500 INTn (in, 10k pull-up) / RSTn (out) | 22 / 23 |
| Internal I2C (HP I2C) SDA / SCL | 2 / 3 |
| External I2C (software I2C, LP I2C pins) SDA / SCL | 6 / 7 |
| 1-wire (UART TX open-drain + RX on the same node, 4.7k pull-up) | 16 / 17 |
| Relay expander reset (EXP_nRESET_INT) | 10 |
| Header expander reset (EXP_nRESET) | 11 |
| Shared expander interrupt (EXP_INT) | 0 |
| Status LED | 1 |
| Spare to J8 | 4, 5 (lightly loaded, strapping-sensitive), 8, 15 (strapping pins: only signals that are high-Z at boot) |

### Ethernet

- WIZnet W5500 (LQFP-48), 10/100, hardware TCP/IP not used: ESP-IDF
  drives it in MAC-raw mode so lwIP and TLS run on the C6.
- SPI mode 0, 20-40 MHz.
- 25 MHz crystal, CL 18 pF (load caps ~15-18 pF after strays), 1M
  feedback across XI/XO per the datasheet figure.
- EXRES1 12.4k 1 %; TOCAP 4.7 uF; 1V2O 10 nF; VBG open; RSVD pins
  to GND; PMODE pins open (all-capable autonegotiation).
- Supplies: ferrite from 3V3 to AVDD, 100 nF on each AVDD pin and on
  VDD, 10 uF bulk.
- Magjack: ARJP11A keeps its role (1CT:1CT magnetics, PoE rectifier,
  Bob Smith termination on pin 7). Its LEDs are driven by the W5500
  (active-low sink): LINK on one, ACT on the other.
- No auto-MDIX in the W5500: fine for switch ports with auto-MDIX.

### Relay channels (unchanged from rev B on this branch)

- 8 channels x (RST, PWR): TLP176AM photoMOS, 330R LED resistor,
  100R output resistor, J11-J18 JST-PH 4p, floating outputs.
- U14 MCP23017 @0x20 on the internal I2C bus, sourcing the LED
  current (GPA0-7 = PWR, GPB0-7 = RST).
- U14 has its own reset line (EXP_nRESET_INT) with a pull-down: if
  the MCU is in reset, hung or not driving it, U14 is in reset and every
  button is released.

### I2C buses

| Bus | Devices | Addresses |
|---|---|---|
| Internal (HP I2C, on-board only) | U14 MCP23017, U5 TMP1075 | 0x20, 0x48 |
| External (J8) | add-on MCP23017 modules, power board LTC4282 x8 | 0x21-0x27, 0x40-0x47 |

A fault on the external bus (cable, add-on, power board) must not stop
relay control: the two buses share no pins, and the internal
expander does not use the header's reset line.

### Expansion header J8

Smaller than rev B's 2x20 now that the MCU has few spare pins. A
2x8 2.54 mm header (DNP), every signal through 330R + TPD4E05U06 ESD
as before:

- External I2C SDA/SCL, EXP_nRESET, EXP_INT
- Spare GPIO4, 5, 8, 15
- 3V3 and 5V (each with PTC fuse + TVS), GND x4

### Other on-board functions (unchanged)

- 4x DS18B20 probe connectors (JST-PH 3p) on the shared 1-wire bus.
- TMP1075 board temperature sensor (now on the internal I2C bus).
- Heartbeat LED (visibility in fluid is not guaranteed; also report
  status over the network).
- USB-C with USBLC6-2 ESD and 5.1k CC pull-downs, now wired to the
  C6's USB pins.

### PCB

- 4 layers, PCBWay, stackup and rules as already set up for this board.
- Differential pairs (Ethernet MDI 100R, USB 90R) recomputed with the
  fluid on the outer layers (requirement 6 under Environment).

## Removed from the previous design

STM32H563VIT6 and its decoupling/VCAP parts, Y1 (MCU crystal), the
LAN8742A and its strapping/termination network, the RMII interface,
J3 Tag-Connect, the NRST/BOOT0 test pads (replaced by EN/BOOT pads),
the 28-pin GPIO expansion header (replaced by the 2x8 header above).

## Open items and verification

1. W5500 PHY-side network (TX/RX termination, TCT/RCT treatment) was
   not confirmed from a WIZnet source; take it from the WIZ850io / W5500
   EVB reference schematic before capture.
2. ARJP11A LED polarity and pin mapping (pins 11-14) against its
   datasheet drawing.
3. Ag9905MT potting material; soak test plan for the magjack (LED
   lenses, internal potting) and the JST housings.
4. Replacement part for C1/C2 (bulk on the PoE 5 V output).
5. LP I2C from the HP core: if ESP-IDF supports it, use it instead of
   software I2C for the external bus (same pins 6/7).
6. ESP32-C6-WROOM-1U-N8 stock and price (Digi-Key lists ~$5.60).
7. Board outline, connector placement, mounting holes, assembly side,
   surface finish (carried over from the layout discussion).
