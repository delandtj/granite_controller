# Connections

Everything plugs into the long edges of the board. Connector names are
on the silkscreen; the pin legends below are printed under each
wire-to-board connector (pin 1 first).

```
            top edge: RJ45 and USB-C openings, then J6 J7 J9, then NODE 1-4
 +-----------------------------------------------------------------------+
 | [power]  [Ethernet]  [ESP32-C6 + USB]  [probes, I2C, expansion]        |
 |                                        [expanders]  [relays, 8 nodes]  |
 +-----------------------------------------------------------------------+
            bottom edge: ESP32 antenna overhang, then J8 J10, then NODE 5-8
```

Power, Ethernet and USB sit together at one end; the eight node
connectors fill the other end, nodes 1 to 4 (J11 to J14) on the top
edge and nodes 5 to 8 (J15 to J18) on the bottom edge. The silkscreen
names every connector.

## Connector summary

| Ref | Function | Connector | Pins (pin 1 first) |
|---|---|---|---|
| J1 | Ethernet | RJ45 magjack, 10/100 | standard; green LED = link, yellow LED = activity |
| J2 | Service port | USB-C receptacle | USB 2.0, Serial/JTAG console; bench power |
| J3 | 19 V input | JST PH 2p | `+` `-` (pin 1 = +19 V, pin 2 = GND) |
| J4..J7 | Temperature probes | JST PH 3p | `3V` `DQ` `G` |
| J8 | Expansion header | 2x6 2.54 mm, **not populated** | see [pinouts](../reference/pinouts.md) |
| J9 | I2C sensor port | JST PH 4p, Qwiic order | `G` `3V` `DA` `CL` |
| J10 | Dry-contact inputs | JST PH 5p | `1` `2` `3` `4` `G` |
| J11..J18 | Node 1..8 | JST PH 5p | `PW` `RS` `CM` `L+` `L-` |

JST PH is 2.0 mm pitch. Mating housings are PHR-n, crimp contacts
SPH-002T-P0.5S (24..30 AWG).

## 19 V input (J3)

- Pin 1 `+` is +19 V, pin 2 `-` is GND. The input is protected against
  reverse polarity by a series Schottky diode and a resettable fuse;
  reversed power simply does nothing until corrected.
- Rating: the on-board PTC holds 0.75 A, the TVS clamps at 35.5 V, the
  buck converter tolerates 36 V. Nominal draw at 19 V is under 0.4 A.
- Tap the bus **upstream of the per-node hot-swap channels** and give
  the tap **its own fuse**. A controller that loses power with a node
  channel cannot report the fault or bring the node back.
- The controller measures this input (VIN sense) and reports it as
  `vin_v` over MQTT and as Modbus input register 9. Rules can act on
  it (`vin` source, in millivolts).

## Ethernet (J1)

- 10/100 Mbit/s over a plain magjack, no PoE. Power the board from
  J3 (or USB on the bench), never expect it from the switch.
- The W5500 has no auto-MDIX. Every managed or unmanaged switch made in
  the last twenty years does, so a normal patch cable to a switch port
  works. A direct cable to a PC without auto-MDIX would need a crossover
  cable.
- Impedance of the on-board pairs is calculated for the fluid, so the
  Ethernet cable may run through the fluid to a gland.
- LEDs on the jack: green = link, yellow = activity. They are driven by
  the W5500, not the firmware, and are the only link indication visible
  on the board. Through fluid and a lid they are not reliable; use the
  status page or MQTT.

## Node cables (J11..J18)

Five wires per node, from the controller to the motherboard's
front-panel header:

| Pin | Legend | Signal | Motherboard side |
|---|---|---|---|
| 1 | `PW` | Power switch | PWR_SW (the button input pin) |
| 2 | `RS` | Reset switch | RESET_SW (the button input pin) |
| 3 | `CM` | Common return of both switches | The GND pins of PWR_SW and RESET_SW |
| 4 | `L+` | Power LED | PLED+ |
| 5 | `L-` | Power LED | PLED- |

How it works, and what to check on your motherboard:

- The switch outputs are floating photoMOS contacts with 100 ohm in
  series. Pressing is "connect PW (or RS) to CM". This matches every
  motherboard whose front-panel switch returns are ground, which is the
  normal case. Confirm it on the board's front-panel pinout: if a
  switch has a separate, non-ground return, wire that return to `CM`
  instead and note that both switches then share it.
- The power LED pins are read through an AC-input optocoupler, so
  **polarity does not matter**; swapping L+ and L- still works. What
  matters is the current the motherboard pushes through its LED
  output: the sense works from about 0.3 mA upward with the board's
  own series resistor plus 220 ohm on the controller. A 3.3 V or 5 V
  PLED drive with the usual 5..20 mA budget is fine.
- The power LED is how the controller knows a node is **on**. Without
  it the node reads `unknown`, state-dependent actions (`on`, `off`,
  `cycle`) are refused unless forced, and the firmware cannot tell a
  hung node from an off one. Wire it, and verify it on the Nodes page
  before sealing. A node whose LED cannot be wired can be configured
  with sense ignored, which drops those protections for that node.
- All five wires carry only switch and LED currents; 26..28 AWG is
  plenty. Keep every node on its own cable and do not share `CM`
  between nodes: the switch returns of different motherboards are
  different grounds, and the photoMOS outputs are floating precisely so
  nothing ties them together.

## Temperature probes (J4..J7)

- DS18B20 in powered mode: `3V` = 3.3 V supply, `DQ` = data with the
  4.7 kohm pull-up on the controller, `G` = ground. Parasitic-power
  wiring (two wires) is not supported.
- The four connectors are one shared 1-wire bus. You may put more than
  one probe on a connector by daisy-chaining; the firmware identifies
  probes by their 64-bit ROM id and keeps up to eight probe slots.
- Keep the total bus short (a few metres) and avoid star topologies
  longer than that; 1-wire does not like long stubs.
- The bus shares pins with the ESP32's boot ROM UART. At power-up a
  short burst of bootloader text appears on `DQ`; the probes ignore it
  and nothing needs to be done about it.
- Probes are assigned to named slots on the Nodes page. A probe that
  stops answering reads `null` in the state payload and `0x8000` in its
  Modbus register; a rule on a missing probe never fires.

## Dry-contact inputs (J10)

- Four inputs, pins 1..4, with a shared ground on pin 5. An input is
  "closed" when its pin is connected to `G`. Each input has a 10 kohm
  pull-up, a 1 kohm / 100 nF filter and ESD protection; wet contacts
  or external voltages are not expected.
- Typical uses: leak float, fluid level float, lid or door switch, a
  "maintenance" key switch. Rules can act on them (for example: leak
  contact closed, force every node off), and they are published in the
  state payload and as Modbus discrete inputs 8..11.

## I2C sensor port (J9)

- JST PH 4p in Qwiic pin order: `G` ground, `3V` 3.3 V, `DA` SDA,
  `CL` SCL. Many Qwiic / STEMMA QT breakouts can be wired straight to
  it with a PH-to-JST-SH adapter cable.
- Electrically this is the **external** I2C bus, shared with J8, run at
  100 kHz with 4.7 kohm pull-ups and protected by 330 ohm series
  resistors, ESD diodes and a resettable fuse on the 3.3 V pin. A fault
  on this bus (unplugged sensor, short on a cable) cannot stop node
  control: the relay and sense expanders are on a separate internal
  bus.
- Addresses 0x21..0x27 are reserved for add-on MCP23017 modules and
  0x40..0x47 for the future power board's LTC4282 hot-swap controllers.
  Pick sensors outside those ranges. Sensor support is driver by
  driver; a sensor the firmware does not know is scanned and listed but
  not read.

## Expansion header (J8)

A 2x6 2.54 mm header footprint, **not populated** on rev C boards.
It carries the same external I2C bus as J9 plus a reset line, a shared
interrupt, two spare GPIOs, 3.3 V and 5 V (both fused and clamped).
It is the planned link to the eight-channel 19 V power board. Pinout in
[Connectors and pinouts](../reference/pinouts.md). Leave it empty unless
you are building that add-on.

## Service port (J2, USB-C)

- Connects directly to the ESP32-C6's USB Serial/JTAG peripheral.
  It is the console, the flashing port and a debug port in one; it
  appears on a Linux host as `/dev/ttyACM0`.
- The board can run from USB VBUS on the bench without 19 V, through
  an OR-ing diode. Worst-case draw exceeds the USB 2.0 default budget,
  so use a port or supply that can deliver 1.5 A, and treat it as a
  bench convenience, not a power input.
- Physical access to this port is full access: the console asks for no
  password. It is intended to be used out of the fluid only, and the
  USB pair impedance is calculated for air, not oil.
- There are no buttons. If the chip has to be forced into download
  mode, the BOOT test pad (GPIO9) is pulled to ground while power is
  applied; the EN test pad resets it.
