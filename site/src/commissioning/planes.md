# Assembling and cabling planes

A power plane is one core board and one to four unit boards. The units
stand in a row and plug into each other; the core sits off the row and
reaches unit 1 over two short ribbons. In a block (8 nodes) the plane
has four units. This chapter puts a plane together, sets its address,
makes and fits the ribbons, terminates the link, mounts the port
modules, and connects the power and the node cables. Do it with the
48 V bus **off** (its breaker outside the fluid open) and the lid open.

```
 48 V distribution block (breaker outside the fluid)
   |XT30                 |XT60            |XT60            |XT60
 +------+  W_CHAIN    +-----------+     +-----------+     +-----------+
 | core |============>| unit 1    |====>| unit 2    | ... | unit 4    |
 | SW1  |  W_CORELINK | slot A  B |     | slot A  B |     | slot A  B |
 +------+============>+-----------+     +-----------+     +-----------+
  J2                    slots 0, 1        slots 2, 3        JP301: 2 caps
   |
   W_LINK (6-way ribbon, CAN FD + 12 V)
   |
 controller J15                         ==> board to board, no cable
```

## 1. Plan the addresses

Every plane on one link needs a unique address from 0x30 to 0x37. In a
block there is one plane per controller, so 0x30 is the natural choice
for every block; give the planes different addresses only where one
controller serves more than one plane (see step 9). The address fixes
the node numbers of the plane, and a node keeps its number across
reboots and across swapping a plane for a spare set to the same
address:

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
is 0**, OFF is 1, and the address is 0x30 + ADR[2:0]; on the CAN FD
link the plane's node id is ADR[2:0]:

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
link with the same address answer the same requests under the same
identifier, and their replies collide; only the switches can fix
that. Write the address down with the plane's position in the frame;
you cannot reach SW1 once the tank is sealed.

## 3. Mount the boards

- Every board is held on its own four M3 holes: the core in its
  corners, a unit by two holes in its input strip and two at its left
  edge.
- The units go in a row on a common mounting bar, unit 1 first,
  units 2 to 4 to its right. Fit them together as in step 5; the bar
  holds the boards, so the joints are not a place to take up
  misalignment.
- The core sits off the row, in the frame's infrastructure section
  (the pod layout), close enough to unit 1 for its two 0.15 m ribbons
  and to the controller for the 0.35 m link ribbon.
- On every unit, the chain input J301 and the link input J303 are at
  the left edge, the chain output J302 and the
  link output J304 at the right edge, in the input strip. The XT60
  enters from the top edge; the node cables leave at the bottom.
- Board sizes: core 68.0 x 72.0 mm, unit 78.6 x 72.1 mm, so a row of
  four units is about 330 mm long with the 4 mm gaps.

## 4. Unit order and node numbers

The chain gives each unit its position: the unit on the core's ribbons
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

## 5. Fit the boards together and make the core ribbons

**Unit to unit: no cable.** Each unit plugs into its neighbour in the
plane of the boards through two joints side by side: the 2x7 chain
and the 2x3 link. A unit's left edge has right-angle pin headers
(plugs, J301 and J303), its right edge right-angle sockets (J302 and
J304), all 2.54 mm.

- Put unit 1 on the bar first, then slide unit 2 up to it until both
  plugs are home in unit 1's sockets, then unit 3, and so on. One push
  mates both joints. Fully mated the boards stand 4 mm apart. The
  joints work from a 4 to a 6 mm gap (6 to 4 mm of the pins inside the
  sockets).
- Plug always goes into socket, so a wrong joint cannot be made.
- Screw every board down on the bar. The bar holds the boards: the
  connectors carry no load and have no latch, so a board that is not
  screwed down can pull out.
- To remove a middle unit, slide the boards after it 6 mm away from it
  first, then lift the unit out.

**Core to unit 1: two ribbons**, both pin n to pin n, red stripe on
pin 1 at both ends, about 0.15 m (0.10-0.15 m from the pod layout;
measure on the first block):

| Ribbon | From | To | Cable | Core end | Unit end |
|---|---|---|---|---|---|
| W_CHAIN | core J301 (socket) | unit 1 J301 (plug) | 14-way halogen-free flat cable, 1.27 mm pitch, 28 AWG (3M HF365/14SF) | 2x7 male IDC plug | 2x7 IDC socket (Wurth 61201423021) |
| W_CORELINK | core J3 LINK OUT (socket) | unit 1 J303 (plug) | 6-way, same cable (3M HF365/06SF) | 2x3 male IDC plug | 2x3 IDC socket (Wurth 61200623021) |

**Open**: no male IDC plug for 1.27 mm flat cable that fits the core's
right-angle sockets has been found yet. The two plugs sit side by side
on the core, 7.4 mm between their end pins: try the fit before making
a set. Whether 28 AWG carries the chain's current per pin is still to
be checked.

The chain carries the I2C segments for the units, 3.3 V from the core
for the units' logic (the units have no supply of their own for it),
three grounds, and two lines that let the core gate and reset the
button presses. The link carries CAN FD (CANH, CANL), the link ground
and AUX_12V straight through every unit. Neither carries 48 V.

**Pressing a ribbon** (IDC, no crimping):

1. Cut the cable square. Do not strip.
2. Slide it into the socket, stripe at the socket's pin 1 triangle.
3. Press in one stroke with a parallel IDC press until both latches
   click.
4. Fold the cable back and snap on the strain relief.
5. Meter it: pin n to pin n, no shorts between neighbours. Mark pin 1
   on every end that goes onto a connector that is not keyed.

## 6. Terminate the link

The CAN FD link is one line from the controller to the last board of
the row. The controller terminates its end. The far end is terminated
by **two standard 2.54 mm jumper caps on JP301 of the last unit**, one
on pins 1-2 (CANH) and one on pins 3-4 (CANL). In a block that is unit
4. Fit no caps anywhere else: not on the other units' JP301 and not on
the core's JP1. A plane with fewer units puts the caps on its last
unit. The caps are not assembled on the boards; they come as an
accessory, two per row.

## 7. Fit the port modules

- One PM-19V-250W per slot you will use. Set the module onto the slot
  so its 2x3 pin header goes into the aux socket (J3 on slot A, J53 on
  slot B), then fix it with four M2 x 5 stainless screws, each with a
  split lock washer under the head, into the standoffs. The screws
  carry the module's power; tighten them evenly, 0.15 Nm at most (the
  standoff's limit for M2).
- The screw pattern is offset: turned the wrong way round the module
  does not take a single screw. Do not force it.
- A slot without a module is reported as absent. A module whose aux
  contact is lost also reads as no module, and the plane firmware holds
  that slot off rather than run a module it cannot supervise.

## 8. Connect the nodes

Per slot:

- **Node power**: a 14 AWG pair from the slot's screw terminal (J1 on
  slot A, J51 on slot B; pin 1 GND, pin 2 +19 V) to the node's 19 V DC
  input, a barrel plug at the node end. **Open**: the plug is not
  chosen yet (jack not measured, no plug rated for the node's 8.7 A
  found); meter the jack's polarity before the first plug goes in.
- **Front panel**: a 4-wire cable, about 0.6 m, from J2 (slot A) or
  J52 (slot B) to the motherboard's front-panel header: 1 PWR_SW,
  2 RST_SW, 3 PLED+, 4 PLED-. JST GH at the slot, single 2.54 mm
  housings at the motherboard. **Open**: the motherboard pinout is not
  confirmed; meter it first. The power LED input is polarised. The
  button contacts close to the slot's ground, the same ground as the
  node power cable, so the node power cable must be connected for
  presses to work. Details in
  [Connections](../planning/connections.md#front-panel-j2-j52).

Label the node cables outside the fluid with their node number.

## 9. Connect the 48 V power and the controller

**48 V.**

- One cable from the distribution block to the core's XT30 (J1).
- One cable from the distribution block to each unit's XT60 (J201).
- Both are keyed (pin 1 `-`, pin 2 `+`); that keying is the only
  reverse-polarity protection, so check every cable end before it goes
  into the tank.
- The core's input feeds only the core: its 3.3 V supply for the core
  and the units' logic, and the isolated 12 V for the link and the
  controller. A unit's input feeds its own two slots through its own
  hot-swap. No board passes the bus to another.
- No fuses on these cables in the fluid. The breaker for the branch
  sits outside the tank. The core's lead is thin (0.75 mm2 is drawn):
  give it its own small breaker outside the fluid, or size the wire for
  the block's breaker.

**Controller.** One 6-way ribbon (W_LINK), about 0.35 m and 1 m at
most, IDC socket at both ends, pin n to pin n, from the controller's
J15 to the core's J2 (LINK IN). It carries the CAN FD link and the
controller's only power. J15 is keyed; the core's J2 is not, so mark
pin 1 at that end and check it: a cable turned round there leaves the
controller unpowered and the link down (nothing is damaged).

The ribbon belongs to the controller's isolated world: its ground is
the controller's ground, not the bus ground.

**More than one plane on a link.** Up to eight planes fit one link, and
the controller firmware can serve them, each on its own address. The
second plane's core then continues the link from the last unit of the
first plane (its J304), and the termination caps move to the last unit
of the last plane. One plane per controller is what the block drawings
show; the cabling for two planes on one link is not drawn yet.

## 10. Checks before power-up

- [ ] Every plane's SW1 set to its planned address, no two alike on one
      link, and recorded with the plane's position.
- [ ] Units in order from unit 1, no gaps; every unit plugged fully
      into its neighbour (4 mm apart) and screwed down on the bar.
- [ ] Core ribbons pin n to pin n, stripe on pin 1 at both ends:
      W_CHAIN from core J301 to unit 1 J301, W_CORELINK from core J3
      to unit 1 J303.
- [ ] Two jumper caps on JP301 of the last unit (1-2 and 3-4), none
      anywhere else.
- [ ] Every module fitted the right way round, all four screws with
      lock washers, aux header seated.
- [ ] XT30 and XT60 cables polarity-checked; breaker outside the fluid
      for the branch.
- [ ] No connection between the bus ground and the controller's
      ground: with everything unpowered, a meter between a core's J2
      pin 3 (LINK_GND) and its XT30 pin 1 reads open.
- [ ] Link ribbon from controller J15 to core J2, pin 1 to pin 1, pin
      1 marked at the core end.
- [ ] Node power and front-panel cables on the right slot, labelled.

Then close the branch breaker. On each core the green `3V3` and `12V`
LEDs light, and the controller starts on the 12 V from the link; the
slots come up on their hardware defaults (every slot with a module on,
after its 160 ms insertion delay). Once the plane core firmware runs,
the `RUN` LED blinks and the controller lists the plane at its address.

**Current firmware**: the plane core firmware (`plane-fd`, the
`link-mcp` build for the STM32G070 with the MCP2518FD) is flashed once
over SWD on the bench (J6, ARM Cortex 10-pin, or J5, Tag-Connect). It
has not yet run on a board. Updating it from the controller over the
link needs the plane bootloader, which is not written yet. A core
without firmware leaves its plane on the hardware defaults: slots on,
no presses, nothing reported to the controller.
