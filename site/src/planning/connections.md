# Connections

This page covers every connector an installer touches: the controller's
and those on the plane boards. How to put a plane together and cable it
to the controller, step by step, is in
[Assembling and cabling planes](../commissioning/planes.md). Full
pinouts are in [Connectors and pinouts](../reference/pinouts.md).

## Controller rev D

Connector names are on the silkscreen; the pin legends below are
printed next to each wire-to-board connector (pin 1 first).

```
        top edge: RJ45 opening, J4 J5 J6 J7 (probes), USB-C opening
 +-------------------------------------------------------+
 | [Ethernet]   [probes]            [USB]         J10    |
 |              [power]    [ESP32-C6]           (dry in) |
 |                                                J9     |
 | J11 J12   J14 J13                [antenna]    (I2C)   |
 +-------------------------------------------------------+
        bottom edge: 12 V IN A / B, PLANE B / A, ESP32 antenna
```

The RJ45 and USB-C openings and the four probe connectors face the top
edge; the two 12 V inputs and the two plane link connectors sit on the
bottom edge, beside the ESP32 module's antenna; the dry-contact inputs
and the I2C sensor port are on the right-hand short edge.

### Connector summary

| Ref | Silkscreen | Function | Connector | Pins (pin 1 first) |
|---|---|---|---|---|
| J1 | ETHERNET | Ethernet | RJ45 magjack, 10/100 | standard; green LED = link, yellow LED = activity |
| J2 | USB | Service port | USB-C receptacle | USB 2.0, Serial/JTAG console; bench power |
| J4..J7 | 1-WIRE | Temperature probes | JST PH 3p | `3V` `DQ` `G` |
| J9 | I2C | I2C sensor port | JST PH 4p, Qwiic order | `G` `3V` `DA` `CL` |
| J10 | DRY IN | Dry-contact inputs | JST PH 5p | `1` `2` `3` `4` `G` |
| J11 | 12V IN A | 12 V input A | JST PH 2p | `+` `-` |
| J12 | 12V IN B | 12 V input B | JST PH 2p | `+` `-` |
| J13 | PLANE A | Plane link A | JST PH 5p | `3V` `G` `CL` `DA` `IN` |
| J14 | PLANE B | Plane link B | JST PH 5p | `3V` `G` `CL` `DA` `IN` |

JST PH is 2.0 mm pitch. Mating housings are PHR-n, crimp contacts
SPH-002T-P0.5S (24..30 AWG).

### 12 V inputs (J11, J12)

- Pin 1 `+` is +12 V, pin 2 `-` is the controller's ground. The pinout
  is the same as the plane core's 12 V aux output J4, so the cable runs
  pin 1 to pin 1.
- Feed one input from each of two different planes when the frame has
  more than one. Each input has its own series Schottky diode (which is
  also the reverse-polarity protection), so the controller runs on
  whichever input is higher and survives the loss of one plane. With a
  single plane, use one input and leave the other empty.
- There is no fuse on the controller. The plane's isolated aux supply
  limits its own output current; a short on the controller is that
  supply's short-circuit case. A 15 V TVS sits after the two diodes;
  the buck converter behind it accepts up to 36 V.
- The 12 V comes from the isolated side of each plane. The controller's
  ground is never the 48 V bus ground (see
  [Isolation](#isolation-and-grounds) below).
- The controller measures the OR-ed input (VIN sense) and reports it as
  `vin_v`. Rules can act on it (`vin` source, in millivolts).
- Open item: the controller's worst-case draw and the rating of the
  plane's 12 V aux output (about 100 mA in the current plane design)
  are still being matched.

### Plane link (J13, J14)

- One I2C bus at 100 kHz plus the shared interrupt line `IN`
  (EXP_INT), brought out on two identical connectors: `3V` supply for
  the planes' isolators, `G` ground, `CL` SCL, `DA` SDA, `IN` interrupt.
  The pinout is the same as the plane core's J2 / J3; cables run pin 1
  to pin 1.
- The planes are daisy chained: controller to the first plane's J2,
  that plane's J3 to the next plane's J2, and so on. The two controller
  connectors are the same bus; they exist so a box can run two shorter
  chains (half the cable capacitance each) instead of one long one. Up
  to eight planes in total, on either chain, each with its own address
  0x30-0x37.
- The I2C pull-ups (2.2 kohm) are on the controller only; the planes
  carry none on the controller side. The value is to be confirmed on
  the bench with eight planes.
- The `3V` pin supplies only the controller side of each plane's
  isolators (a few mA per plane). It goes through a current-limited
  switch per connector, so a shorted cable on one chain does not take
  down the controller.
- Every plane pulls `IN` low through its own open-drain transistor
  when it has an event to report; the controller then reads that
  plane's events.

### Isolation and grounds

The controller, its Ethernet, its USB port and its probes share no
ground with the 48 V bus. On each plane the link (I2C and interrupt)
crosses an isolator and the 12 V aux comes from an isolated converter,
so the controller's ground meets only the isolated side of each plane.
This isolation is functional, not a safety barrier: the bus is SELV
at or below 60 V. Do not tie the controller's ground, the Ethernet
shield or the probe ground to the bus ground anywhere.

### Ethernet (J1)

- 10/100 Mbit/s over a plain magjack, no PoE. Power the board from
  J11 / J12 (or USB on the bench), never expect it from the switch.
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

### Temperature probes (J4..J7)

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
- Probes are assigned to named slots on the setup page. A probe that
  stops answering reads `null` in the state payload; a rule on a
  missing probe never fires.
- Fluid temperature comes from these probes only; the planes report
  their own per-slot temperatures, not the fluid's.

### Dry-contact inputs (J10)

- Four inputs, pins 1..4, with a shared ground on pin 5. An input is
  "closed" when its pin is connected to `G`. Each input has a 10 kohm
  pull-up, a 1 kohm / 100 nF filter and ESD protection; wet contacts
  or external voltages are not expected.
- Typical uses: leak float, fluid level float, lid or door switch, a
  "maintenance" key switch. Rules can act on them (for example: leak
  contact closed, force every node off), and they are published in the
  state payload.
- A contact that is closed while the controller starts is harmless.

### I2C sensor port (J9)

- JST PH 4p in Qwiic pin order: `G` ground, `3V` 3.3 V, `DA` SDA,
  `CL` SCL. Many Qwiic / STEMMA QT breakouts can be wired straight to
  it with a PH-to-JST-SH adapter cable.
- Electrically this is the controller's local I2C bus (shared only
  with the on-board temperature sensor at 0x48), run at 100 kHz with
  4.7 kohm pull-ups and protected by 330 ohm series resistors, ESD
  diodes and a resettable fuse on the 3.3 V pin. A fault on this bus
  (unplugged sensor, short on a cable) cannot stop the plane link,
  which is a separate bus.
- Pick sensors outside 0x48. Sensor support is driver by driver; a
  sensor the firmware does not know is scanned and listed but not read.

### Service port (J2, USB-C)

- Connects directly to the ESP32-C6's USB Serial/JTAG peripheral.
  It is the console, the flashing port and a debug port in one; it
  appears on a Linux host as `/dev/ttyACM0`.
- The board can run from USB VBUS on the bench without 12 V, through
  an OR-ing diode. Use a port or supply that can deliver 1.5 A, and
  treat it as a bench convenience, not a power input.
- Physical access to this port is full access: the console asks for no
  password. It is intended to be used out of the fluid only, and the
  USB pair impedance is calculated for air, not oil.
- There are no buttons. If the chip has to be forced into download
  mode, the BOOT test pad (GPIO9) is pulled to ground while power is
  applied; the EN test pad resets it.

## Plane core board

| Ref | Function | Connector | Pins (pin 1 first) |
|---|---|---|---|
| J1 | 48 V input | Amass XT30PW-M, keyed | 1 `-` bus ground, 2 `+` 48 V |
| J2 | Controller link in | JST PH 5p | 3V3 (controller side), GND (controller side), SCL, SDA, EXP_INT |
| J3 | Controller link out (to the next plane) | JST PH 5p | as J2, same bus |
| J4 | 12 V aux out, isolated | JST PH 2p | 1 +12 V, 2 controller-side ground |
| J301 | Chain out, to unit 1 | JST PH 14p side entry | see [pinouts](../reference/pinouts.md) |
| SW1 | Plane address | 3-way DIP switch | ON = 0, address 0x30 + ADR[2:0] |
| J6 | SWD, bench programming | ARM Cortex 10-pin, 1.27 mm, keyed shroud | standard ARM 10-pin SWD |
| J5 | SWD, bench programming | Tag-Connect TC2030-NL pads | standard TC2030 SWD |

Three green LEDs on the core: `3V3` (the core's 3.3 V supply), `RUN`
(firmware heartbeat) and `12V` (the isolated aux output, on the
controller side of the barrier).

J2 and J3 are the same bus; which one goes to the controller does not
matter electrically, but keep "in" from the controller and "out" to the
next plane so the cabling reads the same on every plane.

## Plane unit board

| Ref | Function | Connector | Pins (pin 1 first) |
|---|---|---|---|
| J201 | 48 V input | Amass XT60PW-M, keyed, cable from the top edge | 1 `-` bus ground, 2 `+` 48 V |
| J301 | Chain in (from the core side, left edge) | JST PH 14p side entry | see [pinouts](../reference/pinouts.md) |
| J302 | Chain out (to the next unit, right edge) | JST PH 14p side entry | see [pinouts](../reference/pinouts.md) |
| J1 / J51 | Node 19 V, slot A / slot B | Degson DG128-5.0-02P screw terminal | 1 GND, 2 +19 V |
| J2 / J52 | Front panel, slot A / slot B | JST GH 4p, latching | `PWR_SW` `RST_SW` `PLED+` `PLED-` |
| J3 / J53 | Port module aux, slot A / slot B | 2x3 1.27 mm socket | mated by the module, not cabled |
| SO1-SO4 / SO51-SO54 | Port module mount, slot A / slot B | M2 SMT standoffs | carry the module's power |

Slot A is the slot on the core side of the unit, slot B the one next to
the input strip.

### Node power (J1, J51)

- A 2-pole 5.0 mm screw terminal per slot: pin 1 GND, pin 2 +19 V from
  the port module (up to 250 W continuous, 13.2 A). Use a 14 AWG pair to
  the node's 19 V DC-ATX converter.
- The rail is switched, current-limited and metered by the slot, and
  an overvoltage on the module output latches the slot off in
  hardware. Nothing else is needed between the terminal and the node.

### Front panel (J2, J52)

Four wires per node, from the slot to the motherboard's front-panel
header:

| Pin | Signal | Motherboard side |
|---|---|---|
| 1 | `PWR_SW` | PWR_SW (the button input pin) |
| 2 | `RST_SW` | RESET_SW (the button input pin) |
| 3 | `PLED+` | PLED+ |
| 4 | `PLED-` | PLED- |

How it works, and what to check on your motherboard:

- The switch outputs are photoMOS contacts that close `PWR_SW` (or
  `RST_SW`) to the slot's ground, which is also pin 1 of the node
  power terminal. There is no separate common wire: the press returns
  through the node's 19 V cable. This matches every motherboard whose
  front-panel switch returns are its ground, the normal case, with a
  DC-ATX converter whose input and output share ground. Check both on
  your parts.
- A press is possible only while the plane core keeps the press enable
  line alive; a hung or reset core ends every press within about a
  second. An unpowered or missing core means no presses at all, which
  never affects a running node.
- The power LED is read through an optocoupler with a 220 ohm series
  resistor. Unlike rev C, **polarity matters**: `PLED+` to the
  motherboard's PLED+, `PLED-` to PLED-. A 3.3 V or 5 V PLED drive
  with the usual series resistor is fine.
- The power LED is how the system knows whether the motherboard is
  running. Without it the node's host state reads `unknown`, and
  actions that depend on it (a clean `off`, `cycle`) lose their
  protection. Wire it, and verify it before sealing.
- The cable carries only switch and LED currents; 26..28 AWG is
  plenty. JST GH is 1.25 mm pitch: housing GHR-04V-S, crimp contacts
  SSHL-002T-P0.2.

### Port module mount (SO1-SO4, J3)

Each slot carries one PM-19V-250W port module, screwed down with four
M2 screws into the standoffs, which carry the module's power. The
module's 2x3 pin header mates the aux socket J3 / J53 as it goes down
(overvoltage kill, power good, module ID, run). The screw pattern is
offset so that a module turned the wrong way does not take a single
screw. A slot with no module reads as empty; see
[Assembling and cabling planes](../commissioning/planes.md).
