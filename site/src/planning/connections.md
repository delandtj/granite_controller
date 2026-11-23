# Connections

This page covers every connector an installer touches: the controller's
and those on the plane boards. How to put a plane together and cable it
to the controller, step by step, is in
[Assembling and cabling planes](../commissioning/planes.md). Full
pinouts are in [Connectors and pinouts](../reference/pinouts.md).

## Controller (controller-fd)

Connector names are on the silkscreen; the pin legends below are
printed next to each wire-to-board connector (pin 1 first).

```
        top edge: RJ45 opening, J4 J5 J6 J7 (probes), USB-C opening
 +-------------------------------------------------------+
 | [Ethernet]   [probes]            [USB]                |
 |              [power]                           J10    |
 |                                              (dry in) |
 |                                [ESP32-C6]      J15   |
 |      J9 (I2C)                  [antenna]    (link,   |
 +-------------------------------------------- B side) -+
        bottom edge: J9 sensor port, ESP32 antenna
```

The RJ45 and USB-C openings and the four probe connectors face the top
edge. The dry-contact inputs J10 and the plane link header J15 are on
the right-hand short edge; J15 sits on the bottom (B) side of the
board, its shroud past the edge, so the ribbon leaves in the plane of
the board. The I2C sensor port J9 is on the bottom long edge.

### Connector summary

| Ref | Silkscreen | Function | Connector | Pins (pin 1 first) |
|---|---|---|---|---|
| J1 | ETHERNET | Ethernet | RJ45 magjack, 10/100 | standard; green LED = link, yellow LED = activity |
| J2 | USB | Service port | USB-C receptacle | USB 2.0, Serial/JTAG console; bench power |
| J4..J7 | 1-WIRE | Temperature probes | JST PH 3p | `3V` `DQ` `G` |
| J9 | I2C | I2C sensor port | JST PH 4p, Qwiic order | `G` `3V` `DA` `CL` |
| J10 | DRY IN | Dry-contact inputs | JST PH 5p | `1` `2` `3` `4` `G` |
| J15 | J15 | Plane link and 12 V supply | keyed 2x3 2.54 mm right-angle box header | 1 CANH, 2 CANL, 3 / 4 LINK_GND, 5 / 6 AUX_12V |

JST PH is 2.0 mm pitch. Mating housings are PHR-n, crimp contacts
SPH-002T-P0.5S (24..30 AWG).

### Plane link and power (J15)

- One cable: a 6-way 2x3 IDC ribbon (W_LINK), about 0.35 m, 1 m at
  most, IDC socket at both ends, pin n to pin n, from J15 to the
  plane core's J2 (LINK IN). Conductor order is pin order, so CANH and
  CANL are neighbours.
- It carries the CAN FD link (500 kbit/s arbitration, 2 Mbit/s data
  phase) and the controller's **only** power input: AUX_12V, about
  12 V, which every core on the link feeds through its own diode from
  its isolated 12 V supply. The controller draws about 0.2 A at 12 V.
  With one plane on the link the controller runs from that plane's
  core alone; with several, it stays up while any one core runs.
- J15 is keyed; the core's J2 is a bare 2x3 pin header and is not.
  A cable turned round at the core puts AUX_12V on CANH / CANL. Both
  transceivers survive that, but the link stays down and the controller
  gets no power. Mark pin 1 at the core end of the cable.
- The controller terminates its end of the bus (split termination,
  2 x 60.4 ohm). The other end is terminated by two jumper caps on the
  last board of the row (see
  [Assembling and cabling planes](../commissioning/planes.md)).
- Behind J15: reverse-polarity Schottky, a 15 V TVS, then the buck
  converter (accepts up to 36 V). No fuse: the cores' isolated supplies
  limit their own output. The controller measures the input (VIN
  sense) and reports it as `vin_v`. Rules can act on it (`vin`
  source, in millivolts).
- Open item: the controller's draw and the rating of a core's isolated
  12 V supply are still to be checked against each other on the first
  boards.

### Isolation and grounds

The controller's ground is the link ground (LINK_GND). On each plane
core the link crosses an isolated CAN FD transceiver (ISO1044) and the
12 V comes from an isolated converter, so the controller's ground
meets only the isolated side of each core. The units carry the link
straight through without parts. The controller, its Ethernet, its
USB port and its probes share no ground with the 48 V bus. This
isolation is functional, not a safety barrier: the bus is SELV at or
below 60 V. Do not tie the controller's ground, the Ethernet shield or
the probe ground to the bus ground anywhere.

### Ethernet (J1)

- 10/100 Mbit/s over a plain magjack, no PoE. Power the board from
  the link ribbon on J15 (or USB on the bench), never expect it from
  the switch.
- The W5500 has no auto-MDIX. Every managed or unmanaged switch made in
  the last twenty years does, so a normal patch cable to a switch port
  works. A direct cable to a PC without auto-MDIX would need a crossover
  cable.
- In a block, the controller goes to a management switch outside the
  block, not to the block switch: the block switch's eight copper ports
  are the eight nodes'.
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
| J2 | LINK IN, from the controller (or the previous plane) | 2x3 2.54 mm right-angle pin header (CJT A2541WR-2x3P), not keyed | 1 CANH, 2 CANL, 3 / 4 LINK_GND, 5 / 6 AUX_12V |
| J3 | LINK OUT, to unit 1 | 2x3 2.54 mm right-angle socket (CJT A2541HWR-2x3P) | as J2 |
| J301 | Chain out, to unit 1 | 2x7 2.54 mm right-angle socket (CJT A2541HWR-2x7P) | see [pinouts](../reference/pinouts.md) |
| JP1 | Link termination | 2x2 2.54 mm pin header | no caps on a core in a block row; see [pinouts](../reference/pinouts.md) |
| SW1 | Plane address | 3-way DIP switch | ON = 0, address 0x30 + ADR[2:0] |
| J6 | SWD, bench programming | ARM Cortex 10-pin, 1.27 mm, keyed shroud | standard ARM 10-pin SWD |
| J5 | SWD, bench programming | Tag-Connect TC2030-NL pads | standard TC2030 SWD |

Three green LEDs on the core: `3V3` (the core's 3.3 V supply), `RUN`
(firmware heartbeat) and `12V` (the isolated 12 V for the link, on the
controller side of the barrier).

J2 and J3 are the same bus, wired straight through; J2 takes the
controller's ribbon, J3 continues the link to unit 1 over a short
ribbon. J2 to J3 also carries AUX_12V, onto which this core feeds its
isolated 12 V through a diode. In a block the core sits off the unit
row, so its J301 and J3 reach unit 1 over two ribbons (W_CHAIN and
W_CORELINK, see [Assembling and cabling
planes](../commissioning/planes.md#5-fit-the-boards-together-and-make-the-core-ribbons)).

## Plane unit board

| Ref | Function | Connector | Pins (pin 1 first) |
|---|---|---|---|
| J201 | 48 V input | Amass XT60PW-M, keyed, cable from the top edge | 1 `-` bus ground, 2 `+` 48 V |
| J301 | Chain in (from the core side, left edge) | 2x7 2.54 mm right-angle pin header, plug (CJT A2541WR-2x7P) | see [pinouts](../reference/pinouts.md) |
| J302 | Chain out (to the next unit, right edge) | 2x7 2.54 mm right-angle socket (CJT A2541HWR-2x7P) | see [pinouts](../reference/pinouts.md) |
| J303 | Link in (left edge, beside J301) | 2x3 2.54 mm right-angle pin header (CJT A2541WR-2x3P) | 1 CANH, 2 CANL, 3 / 4 LINK_GND, 5 / 6 AUX_12V |
| J304 | Link out (right edge, beside J302) | 2x3 2.54 mm right-angle socket (CJT A2541HWR-2x3P) | as J303 |
| JP301 | Link termination | 2x2 2.54 mm pin header | jumper caps on 1-2 and 3-4 on the last unit of the row only |
| J1 / J51 | Node 19 V, slot A / slot B | Degson DG128-5.0-02P screw terminal | 1 GND, 2 +19 V |
| J2 / J52 | Front panel, slot A / slot B | JST GH 4p, latching | `PWR_SW` `RST_SW` `PLED+` `PLED-` |
| J3 / J53 | Port module aux, slot A / slot B | 2x3 1.27 mm socket | mated by the module, not cabled |
| SO1-SO4 / SO51-SO54 | Port module mount, slot A / slot B | M2 SMT standoffs | carry the module's power |

Slot A is the slot on the core side of the unit, slot B the one next to
the input strip.

### Node power (J1, J51)

- A 2-pole 5.0 mm screw terminal per slot: pin 1 GND, pin 2 +19 V from
  the port module (up to 250 W continuous, 13.2 A). Use a 14 AWG pair to
  the node's 19 V DC input. For the node drawn (an AXB35 board with a
  DC-045D jack) that end is a barrel plug, drawing about 8.7 A at
  165 W. **Open**: the plug is not chosen. The jack's pin is not
  measured, and no plug rated for 8.7 A has been found (the best found
  are rated 5 A).
- The rail is switched, current-limited and metered by the slot, and
  an overvoltage on the module output latches the slot off in
  hardware. Nothing else is needed between the terminal and the node.

### Front panel (J2, J52)

Four wires per node, from the slot to the motherboard's front-panel
header. The header is on the other side of the plane, so the cable is
about 0.6 m: a JST GH 4p housing at the slot, single 2.54 mm
Dupont-style housings (one per wire) at the motherboard. **Open**: the
motherboard's front-panel pinout and how it drives its power LED are
not confirmed; meter the header before plugging in. PWR_SW and RST_SW
go on the switch pins that are not ground; the header's ground pins
stay open.

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
  front-panel switch returns are its ground and whose 19 V DC input
  shares that ground, the normal case. Check both on your board.
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
  plenty. The block BOM makes it from 4 ways peeled off the same
  halogen-free 28 AWG flat cable as the link ribbon, red stripe on
  pin 1. JST GH is 1.25 mm pitch: housing GHR-04V-S, crimp contacts
  SSHL-002T-P0.2. Seen from the other side of the plane the order looks
  mirrored: go by the stripe and a meter, never by position.

### Port module mount (SO1-SO4, J3)

Each slot carries one PM-19V-250W port module, screwed down with four
M2 screws into the standoffs, which carry the module's power. The
module's 2x3 pin header mates the aux socket J3 / J53 as it goes down
(overvoltage kill, power good, module ID, run). The screw pattern is
offset so that a module turned the wrong way does not take a single
screw. A slot with no module reads as empty; see
[Assembling and cabling planes](../commissioning/planes.md).
