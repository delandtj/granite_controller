# Assembling and cabling planes

A power plane is one core board and one to four unit boards in a row,
plugged into each other. This chapter puts a plane together, sets
its address, mounts the port modules, connects the power and the node
cables, and cables the planes to the controller. Do it with the 48 V
bus **off** (its breaker outside the fluid open) and the lid open.

```
 48 V bus (breaker outside the fluid)
   |XT30          |XT60               |XT60               |XT60
 +------+ chain +-------------+ chain +-------------+ chain +---- ...
 | core |------>| unit 1      |------>| unit 2      |------>| unit 3
 | SW1  |       | slot A  B   |       | slot A  B   |       |
 +------+       +-------------+       +-------------+       +----
  J2 J3 J4        slots 0, 1            slots 2, 3
   |  |  \__ 12 V aux to the controller (J11 or J12)
   |  \_____ link out to the next plane's J2
   \________ link in from the controller (J13 / J14) or the previous plane
```

## 1. Plan the addresses

Every plane on a controller needs a unique address from 0x30 to 0x37.
The address fixes the node numbers of the plane, and a node keeps its
number across reboots and across swapping a plane for a spare set to
the same address:

| Address | Nodes |
|---|---|
| 0x30 | 1-8 |
| 0x31 | 9-16 |
| 0x32 | 17-24 |
| 0x33 | 25-32 |
| 0x34 | 33-40 |
| 0x35 | 41-48 |
| 0x36 | 49-56 |
| 0x37 | 57-64 |

Node numbers cannot be remapped in the configuration; names can. If
your site numbers its nodes in frame order, set the plane addresses in
the same order. A plane with fewer than four units simply leaves the
higher node numbers of its block unused.

## 2. Set the address (SW1 on the core)

SW1 is a 3-way DIP switch on the core board, next to the
microcontroller. Switch 1 is ADR0, switch 2 ADR1, switch 3 ADR2. **ON
is 0**, OFF is 1, and the address is 0x30 + ADR[2:0]:

| SW1-3 (ADR2) | SW1-2 (ADR1) | SW1-1 (ADR0) | Address |
|---|---|---|---|
| ON | ON | ON | 0x30 |
| ON | ON | OFF | 0x31 |
| ON | OFF | ON | 0x32 |
| ON | OFF | OFF | 0x33 |
| OFF | ON | ON | 0x34 |
| OFF | ON | OFF | 0x35 |
| OFF | OFF | ON | 0x36 |
| OFF | OFF | OFF | 0x37 |

All three ON is the first plane. Set the switch with the core
unpowered; the core reads it when it starts. Two planes on the same
address both answer at once: the controller sees only corrupted
replies and reports a suspected address collision, which only the
switches can fix. Write the address down with the plane's position in
the frame; you cannot reach SW1 once the tank is sealed.

## 3. Mount the boards

- Every board is held on its own four M3 holes: the core in its
  corners, a unit by two holes in its input strip and two at its left
  edge. Mount them in a row on a common mounting bar, core first,
  units to its right. Fit them together as in step 5; the bar holds
  the boards, so the chain joint is not a place to take up misalignment.
- On every unit, the chain input J301 is at the left edge (towards
  the core) and the chain output J302 at the right edge, in the input
  strip. The XT60 enters from the top edge; the node cables leave at
  the bottom.
- Board sizes at the time of writing: core 56 x 72 mm, unit 78.5 x
  72 mm, so a full 8-slot plane is about 380 mm long with gaps. These
  may still change before the boards are built.

## 4. Unit order and node numbers

The chain gives each unit its position: the unit plugged into the core
is unit 1, the one plugged into unit 1 is unit 2, and so on up to unit
4. The unit boards are identical and carry no setting of their own; the
position in the row alone decides which slots a unit holds.

| Unit (from the core) | Slot A | Slot B | Nodes on plane 0x30 |
|---|---|---|---|
| 1 | slot 0 | slot 1 | 1, 2 |
| 2 | slot 2 | slot 3 | 3, 4 |
| 3 | slot 4 | slot 5 | 5, 6 |
| 4 | slot 6 | slot 7 | 7, 8 |

Slot A is the slot on the core side of a unit, slot B the one next to
its input strip. In general: node = 8 x (address - 0x30) + slot + 1.

Swapping two units' positions in the chain swaps their node numbers,
because the node number follows the position, not the board. Keep the
units in order and leave no gap: unit 3 is reached only through units
1 and 2.

## 5. Fit the boards together on the bar

There are no chain cables and nothing to crimp. Each board plugs into
its neighbour in the plane of the boards, with a right-angle 2x7 2.54
mm pin header (plug) on a unit's left edge (J301) and a right-angle 2x7
socket on the core's right edge (J301) and a unit's right edge (J302).

- Put the core on the bar first. Slide unit 1 up to it until the plug
  is home in the core's socket, then unit 2 onto unit 1, and so on.
  Fully mated the boards stand 4 mm apart. The joint works from a 4 to
  a 6 mm gap (6 to 4 mm of the pins inside the socket).
- Plug always goes into socket, so a wrong joint cannot be made: the
  core's J301 or a unit's J302 (sockets, right edge) receives the next
  unit's J301 (plug, left edge).
- Screw every board down on the bar. The bar holds the boards: the
  connector carries no load and has no latch, so a board that is not
  screwed down can pull out.
- To remove a middle unit, slide the boards after it 6 mm away from it
  first, then lift the unit out.

The chain carries the I2C segments for the units, 3.3 V from the core
for the units' logic (the units have no supply of their own for it),
three grounds, and two lines that let the core gate and reset the
button presses. It carries no 48 V.

## 6. Fit the port modules

- One PM-19V-250W per slot you will use. Set the module onto the slot
  so its 2x3 pin header goes into the aux socket (J3 on slot A, J53 on
  slot B), then fix it with four M2 x 5 stainless screws, each with a
  split lock washer under the head, into the standoffs. The screws
  carry the module's power; tighten them evenly (0.25 Nm, still to be
  confirmed on a sample).
- The screw pattern is offset: turned the wrong way round the module
  does not take a single screw. Do not force it.
- A slot without a module is reported as absent. A module whose aux
  contact is lost also reads as no module, and the plane firmware holds
  that slot off rather than run a module it cannot supervise.

## 7. Connect the nodes

Per slot:

- **Node power**: a 14 AWG pair from the slot's screw terminal (J1 on
  slot A, J51 on slot B; pin 1 GND, pin 2 +19 V) to the node's 19 V
  DC-ATX converter.
- **Front panel**: a 4-wire JST GH cable from J2 (slot A) or J52
  (slot B) to the motherboard: 1 PWR_SW, 2 RST_SW, 3 PLED+, 4 PLED-.
  The power LED input is polarised. The button contacts close to the
  slot's ground, the same ground as the node power cable, so the node
  power cable must be connected for presses to work. Details in
  [Connections](../planning/connections.md#front-panel-j2-j52).

Label the node cables outside the fluid with their node number.

## 8. Connect the 48 V power

- One cable from the bus to the core's XT30 (J1).
- One cable from the bus to each unit's XT60 (J201).
- Both are keyed (pin 1 `-`, pin 2 `+`); that keying is the only
  reverse-polarity protection, so check every cable end before it goes
  into the tank.
- The core's input feeds only the core: its 3.3 V supply for the core
  and the units' logic, and the isolated 12 V aux. A unit's input feeds
  its own two slots through its own hot-swap. No board passes the bus
  to another.
- No fuses on these cables in the fluid. The breaker for the branch
  sits outside the tank.

## 9. Cable the planes to the controller

**Link.** The controller's J13 (PLANE A) and J14 (PLANE B) are the same
bus on two connectors. Run a JST PH 5-pin cable, pin 1 to pin 1, from
one of them to the first plane's J2; from that plane's J3 to the next
plane's J2, and so on. With several planes you may split them over two
chains, one from J13 and one from J14, to keep each chain short. Up to
eight planes in total on both chains, each on its own address. Leave
the last plane's J3 empty; there is nothing to terminate.

**12 V aux.** Run a JST PH 2-pin cable, pin 1 to pin 1, from a plane
core's J4 (12V AUX OUT) to the controller's J11 (12V IN A). With two
or more planes, run a second one from a **different** plane's J4 to
J12 (12V IN B), so the controller stays up when one plane loses its
bus. With a single plane, J12 stays empty.

Both cables belong to the controller's isolated world: their ground is
the controller's ground, not the bus ground.

## 10. Checks before power-up

- [ ] Every plane's SW1 set to its planned address, no two alike, and
      recorded with the plane's position.
- [ ] Units in order from the core, no gaps; every board plugged fully
      into its neighbour (4 mm apart) and screwed down on the bar.
- [ ] Every module fitted the right way round, all four screws with
      lock washers, aux header seated.
- [ ] XT30 and XT60 cables polarity-checked; breaker outside the fluid
      for the branch.
- [ ] No connection between the bus ground and the controller's
      ground: with everything unpowered, a meter between a core's J4
      pin 2 and its XT30 pin 1 reads open.
- [ ] Link cables pin 1 to pin 1, from the controller to the first
      plane's J2 and from J3 to the next J2.
- [ ] 12 V aux to J11 (and J12 from a second plane), pin 1 to pin 1.
- [ ] Node power and front-panel cables on the right slot, labelled.

Then close the branch breaker. On each core the green `3V3` and `12V`
LEDs light; the slots come up on their hardware defaults (every slot
with a module on, after its 160 ms insertion delay). Once the plane
core firmware runs, the `RUN` LED blinks and the controller lists the
plane at its address.

**Current firmware**: the plane core firmware is not written yet at the
time of writing. It is to be flashed once over SWD on the bench (J6,
ARM Cortex 10-pin, or J5, Tag-Connect) and then updated from the
controller over the link. Until a core runs it, its plane stays on the
hardware defaults: slots on, no presses, nothing reported to the
controller.
