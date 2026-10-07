# Granite controller - component spec

Status: draft, 2026-10-07. Supersedes the STM32H563 + LAN8742A design.
Captured in the schematic as rev C (sheets Power, Ethernet, MCU, Relay
channels, Expansion header). Open items
are listed at the end; everything else is a requirement.

## Purpose

Remote reset/power control and monitoring for one frame of 8
motherboards (nodes), immersion-cooled. The frame's redundant 19 V
bus powers the board; one Ethernet cable controls it. It presses each node's front-panel RESET and
PWR buttons, reads temperature probes, and talks to optional add-on
modules and the power distribution board (docs/power-board.md).

## Context

```
 network ==RJ45==> [ controller ] --8x JST-PH 4p--> node front panels
 19 V bus --J3 JST-PH 2p--^
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

- Input: J3 (JST PH 2p, pin 1 = +19 V, pin 2 = GND) from the frame's
  redundant 19 V bus. Tap it upstream of the per-node LTC4282 hot-swap
  channels, with its own fuse: a node channel that trips or is disabled
  must not take the controller down with it. PoE is gone (the Ag9905MT
  module and the PoE magjack were the two most expensive parts and the
  module was not stocked at JLC).
- Input protection: F3 PTC (1206L050/33NR, 0.5 A hold, 33 V), D1 TVS
  (SMAJ22A, 22 V standoff, 35.5 V clamp, below the buck's 38 V abs
  max), D8 SS34 series Schottky for reverse polarity (with a reversed
  input D1 conducts forward and trips F3).
- U8 LMR51430YDDCR (TI, 4.5-36 V in, 3 A, 1.1 MHz PFM) -> BUCK_5V.
  Datasheet table 9-2 values: L3 3.3 uH (FXL0630-3R3-M, molded, 8.5 A
  sat), CIN 2x 4.7 uF/50 V X7R + 100 nF, COUT 2x 22 uF/25 V, CBOOT
  100 nF, EN tied to VIN. Feedback 100k / 13k: 0.6 V x (1 + 100/13) =
  5.22 V, so +5V is about 4.85 V after the D2 Schottky.
- 5 V rail: BUCK_5V or USB-C VBUS (bench service), OR-ed by D2/D3, then
  TLV62569 buck -> 3V3 (unchanged).
- 5 V budget: 3V3 load (below) plus up to 0.5 A to J8 through F2: about
  1.2 A worst case, about 0.4 A at 19 V.
- 3V3 budget: ESP32-C6 module up to ~0.4 A peak (radio TX; normally
  off), W5500 ~0.13 A, relay expander plus LEDs ~0.1 A, rest < 0.05 A.
  Total under 0.7 A against the 2 A buck.
- C2 (220 uF bulk on +5V) is a KEMET T520D227M010ATE018 molded
  tantalum-polymer (10 V, 18 mOhm, D case) instead of a rubber-sealed
  aluminium-polymer can: 5 V is 50 % of rating.
- Every other part carries an LCSC field (JLC assembly). Generic
  passives use JLC basic parts with equal or better voltage, dielectric
  and tolerance than the original MPNs.
- USB-C VBUS can power the board for bench service (existing D2/D3
  OR-ing kept).

### MCU

- Espressif ESP32-C6-MINI-1-N4 (RISC-V, 4 MB flash, PCB antenna).
  Chosen over the WROOM-1U because KiCad ships its symbol and
  footprint. The radio is unused, but keep the antenna keep-out free
  of copper anyway (module datasheet), at a board edge.
- Radio off in firmware (useless submerged; the fluid detunes the
  antenna anyway).
- EN: 10k pull-up + 1 uF. BOOT (GPIO9): weak pull-up in the chip; test
  pad to GND for forced download mode. EN also on a test pad. No
  buttons (nobody presses buttons in a tank).
- Decoupling at the module 3V3 pin: 10 uF + 100 nF.
- Firmware updates over the network (OTA) in normal operation; USB
  Serial/JTAG over USB-C for bench flashing, console and debug. The
  Tag-Connect SWD footprint (J3) goes away.

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
| Relay expander reset (EXP_nRESET_INT) | 14 |
| Header expander reset (EXP_nRESET, 10k pull-down) | 4 |
| Shared expander interrupt (EXP_INT) | 0 |
| Status LED | 1 |
| Spare to J8 | 5 (lightly loaded), 8 (strapping: 10k pull-up), 15 (strapping: only signals that are high-Z at boot) |

### Ethernet

- WIZnet W5500 (LQFP-48), 10/100, hardware TCP/IP not used: ESP-IDF
  drives it in MAC-raw mode so lwIP and TLS run on the C6.
- SPI mode 0, 20-40 MHz.
- 25 MHz crystal, CL 18 pF (load caps ~15-18 pF after strays), 1M
  feedback across XI/XO per the datasheet figure.
- EXRES1 12.4k 1 %; TOCAP 4.7 uF; 1V2O 10 nF; VBG open; RSVD pins
  to GND; PMODE pins open (all-capable autonegotiation).
- Supplies: see the AVDD line below; 100 nF on VDD.
- Magjack: HanRun HR911105A (LCSC C12074; replaces the PoE ARJP11A).
  1CT:1CT magnetics, internal Bob Smith termination (75R + 1 nF/2 kV)
  to pin 8, which goes to GND. The shield (SH) goes to ETH_SHLD (1M ||
  1 nF/2 kV to GND, R28/C36). LEDs are driven by the W5500 (active-low
  sink): green (anode 9, cathode 10) = LINK, yellow (anode 12, cathode
  11) = ACT.
- PHY-side network, copied from WIZnet's W5500 Ethernet Shield
  reference schematic (github.com/Wiznet/W5500_Ethernet_Shield,
  Schematic/W5500_Ethernet_shield.sch):
  - TXP / TXN: 33R series each, then to magjack TD+ / TD-. Each TD
    line has 49R9 1 % to AVDD (+3V3A).
  - Transmit center tap (TCT): 10R 1 % to AVDD, 22 nF to GND.
  - RXP / RXN: 33R series each to an RX node; each node has 49R9 1 %
    to the receive center tap and 6.8 nF in series to magjack RD+ / RD-.
  - Receive center tap (RCT): joined to the 49R9 pair, 10 nF to GND.
  - AVDD (+3V3A): from 3V3 through a 120R@100MHz ferrite bead
    (HH-1M1608-121JT in the reference), 100 nF per AVDD pin, 10 uF bulk.
  - LEDs: anodes to 3V3, cathodes through 330R to LINKLED / ACTLED.
  - Shield: 1 nF / 2 kV to GND (already present as C36).
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
2x6 2.54 mm header (DNP), every signal through 330R + TPD4E05U06 ESD
as before:

| Pin | Signal | Pin | Signal |
|---|---|---|---|
| 1 | 3V3 (PTC + TVS) | 2 | 5V (PTC + TVS) |
| 3 | GND | 4 | GND |
| 5 | I2C_EXT SCL | 6 | I2C_EXT SDA |
| 7 | EXP_nRESET | 8 | EXP_INT |
| 9 | GPIO5 | 10 | GPIO8 |
| 11 | GPIO15 | 12 | GND |

### Other on-board functions (unchanged)

- 4x DS18B20 probe connectors (JST-PH 3p) on the shared 1-wire bus.
- TMP1075 board temperature sensor (now on the internal I2C bus).
- Heartbeat LED (visibility in fluid is not guaranteed; also report
  status over the network).
- USB-C with USBLC6-2 ESD and 5.1k CC pull-downs, now wired to the
  C6's USB pins.

### PCB

- 4 layers, JLCPCB (bare board and assembly), JLC04161H-7628 stackup.
- Differential pairs: Ethernet MDI 0.16/0.15 mm = 101R with the fluid
  above the outer layers (requirement 6 under Environment); USB
  0.25/0.15 mm = 89R in air, since the service port is used out of
  the fluid.

## Removed from the previous design

STM32H563VIT6 and its decoupling/VCAP parts, Y1 (MCU crystal), the
LAN8742A and its strapping/termination network, the RMII interface,
J3 Tag-Connect, the NRST/BOOT0 test pads (replaced by EN/BOOT pads),
the 28-pin GPIO expansion header (replaced by the 2x8 header above).

## Open items and verification

1. Soak test plan for the HR911105A magjack (LED lenses, internal
   potting), the JST housings and the J8 header housing (Megastar,
   material not confirmed).
2. 19 V bus details from the power board: where the controller tap
   sits relative to the hot-swap channels, and its fuse.
3. LP I2C from the HP core: if ESP-IDF supports it, use it instead of
   software I2C for the external bus (same pins 6/7).
4. ESP32-C6-MINI-1-N4 stock and price.
5. Board outline, connector placement, mounting holes, assembly side,
   surface finish (carried over from the layout discussion).

Also to review: the rev C sheets are generated (one label per pin,
parts in rows). Electrically checked (ERC clean, netlist traced);
placement and readability can be tidied by hand in KiCad.
