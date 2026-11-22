# Block cabling

Status: exploring (2026-11-15). Nothing here is decided beyond what the
ADRs it cites decide; the open items are listed at the end.

A block is the unit everything else is a multiple of: one row of 8
nodes, one plane (core + 4 units, a port module per node), one block
switch and one controller. A pod is N blocks; the table at the end
scales the counts.

The harness drawings are WireViz files in `cabling/`. Each `.yml` is
the source; the `.svg` and `.bom.tsv` next to it are generated:

```sh
cd cabling
uvx wireviz -f st *.yml
```

| Drawing | What it shows |
|---------|---------------|
| `cabling/block-power.yml` | 48 V from the supply to the units, the core and the switch supply |
| `cabling/node.yml` | the 19 V and front-panel cables from a unit slot to its node |
| `cabling/link.yml` | the controller's ribbon to the core (CAN FD and its 12 V) |
| `cabling/network.yml` | node patch cables, the switch uplink, the controller's port |

The level above, the container's three-phase feed and the 48 V bus per
pod, is in `docs/container-power.md`.

## What is cabled and what is not

Board to board, no cable (docs/adr/0005, 0006):

- core to unit 1, and unit to unit: the 2x7 chain and the 2x3 link,
  plugged edge to edge at 4 mm, all boards screwed to one bar;
- port module to unit: four M2 screws carry the power, a 2x3 1.27 mm
  header the signals.

Cabled:

- 48 V: one XT60 lead per unit, one XT30 lead for the core, one XT30
  lead for the switch supply. No 48 V passes between boards.
- 19 V and front panel: two cables per node.
- Link: one ribbon from the controller to the core. It is also the
  controller's only supply (12 V from the core, isolated from the bus).
- Ethernet: one patch cable per node, one uplink per switch, one patch
  cable for the controller.

## Cable schedule, one block

| Id | Qty | From | To | Cable | Ends | Current |
|----|-----|------|----|-------|------|---------|
| W1-W2 | 1 | supply bus, via a 63 A DC breaker outside the fluid | distribution block in the fluid | 2 x 16 mm2 | lugs | about 44 A |
| W3-W6 | 4 | distribution block | unit J201 | 2 x 4 mm2 | XT60H-F | 14 A, limit 18.3 A |
| W7 | 1 | distribution block | core J1 | 2 x 0.75 mm2 | XT30U-F | under 1 A |
| W8 | 1 | distribution block | pp-48-sw12 J1 | 2 x 0.75 mm2 | XT30U-F | 0.6-0.9 A |
| W9 | 1 | pp-48-sw12 J2 | switch 12 V inlet | 2 x 0.75 mm2 | Micro-Fit 3.0 2p, barrel plug | up to 2.5 A |
| W_19V | 8 | unit J1 / J51 | node 19 V input | 2 x 14 AWG | screw terminal, node end open | 8.7 A at 165 W, 13.2 A max |
| W_FP | 8 | unit J2 / J52 | motherboard front-panel header | 4 x 28 AWG ribbon, 0.6 m | JST GH 4p, 2.54 mm housing | signal |
| W_LINK | 1 | controller-fd J15 | core J2 | 6-way IDC ribbon, 1 m at most | 2x3 IDC socket both ends | about 0.2 A at 12 V |
| P1-P8 | 8 | node RJ45 | switch port 1-8 | Cat6 patch | RJ45 | - |
| UP | 1 | switch port 9 | top-of-rack SFP+ | SFP+ DAC, or modules and fibre | SFP+ | - |
| PC | 1 | controller-fd J1 | management switch | Cat5e patch, straight | RJ45 | - |

Not a cable: two jumper caps on JP301 of unit 4 terminate the link.

What to buy is in `docs/block-bom.md`. The front-panel cable is 0.6 m
because the motherboard's header is on the other side of the plane.

Lengths in the harness files are placeholders (0.3-0.4 m inside the
block otherwise, 3 m to the outside). They follow from where the plane sits
relative to its 8 frames, which is not laid out yet.

## Grounds

Three domains, kept apart on purpose (docs/adr/0003 decision 14,
docs/adr/0006 decision 5, docs/adr/0008):

- Bus ground: supply, units, port modules, nodes. Earthed at one
  point, at the supply. Node ground is bus ground.
- Link ground: the link, the controller and everything on it
  (Ethernet, USB, probes). Isolated from the bus at the core.
  Functional isolation, not a safety barrier.
- Switch supply output: floats (proposed), so the switch's shielded
  ports do not tie bus ground to the network.

The front-panel cable has no ground wire of its own: a press returns
through the 19 V cable.

## Network

- The block switch has 8 copper ports and one SFP+, and a block has 8
  nodes. There is no port for the controller, so it is cabled to a
  management switch outside the block (firmware/docs/adr/0006,
  "Physical layout"). A controller on its own block's switch would
  cost a node.
- VLAN 1 is the switch's management VLAN (firmware/docs/adr/0006
  component 25). Node ports do not carry it. The uplink carries it
  untagged, and the PVID of the upstream port puts it in the
  management VLAN; the data VLANs ride the uplink tagged.
- The controller's port is 10/100 without auto-MDIX: a straight cable.

## Open

- **How 19 V ends at the node.** The AXB35 has a DC-045D jack. A
  barrel plug carrying 9-13 A, or leads soldered to the jack pins:
  not decided. The plug size is not measured.
- **Motherboard front-panel pinout** and how it drives the power LED:
  not confirmed, so the node end of W_FP is not fixed.
- **Protection outside the fluid.** One breaker per block is the
  option drawn (docs/adr/0005, open point of 2026-11-12); it is not
  taken. The distribution block in the fluid is not chosen.
- **4 mm2 into an XT60H-F.** The ADR asks for 4 mm2 or more to a
  unit; check that it fits the cup, or use 12 AWG.
- **Supply and tank entry.** Which supply, and how 48 V and the
  network cables pass the tank wall (docs/handoff.md).
- **Switch supply.** docs/adr/0008 is proposed; the board does not
  exist, and whether the switch sits in the fluid is open.
- **Where the plane sits.** A plane is 398 mm long; 8 frames at the
  58.7 mm pitch of today are 476 mm, at 36 mm they are 294 mm. At 36 mm
  the units are 8 mm too wide each and the core has to leave the row.
  Every length above depends on this.
- **Standby controller.** Not cabled here. It would sit at the far
  end of the link and terminate it instead of the jumper caps.

## Scaling to a pod

Per block, so multiply: 4 unit leads, 1 core lead, 1 switch-supply
lead and its 12 V lead, 8 node power cables, 8 front-panel cables, 8
node patch cables, 1 uplink.

Per controller: 1 link ribbon and 1 patch cable to the management
switch. One controller per block is what is drawn. Up to 8 planes fit
one link (docs/adr/0006), so a controller could take 2 blocks (16
nodes) with the second core plugged onto the first plane's last unit;
that needs both planes in one line.

| Pod | Blocks | Unit leads | Node cable sets | Patch cables | Uplinks | Controllers at 1 / 2 blocks each |
|-----|--------|-----------|-----------------|--------------|---------|-----------------------------------|
| 8 nodes | 1 | 4 | 8 | 8 | 1 | 1 / 1 |
| 48 nodes | 6 | 24 | 48 | 48 | 6 | 6 / 3 |
| 64 nodes | 8 | 32 | 64 | 64 | 8 | 8 / 4 |
| 96 nodes | 12 | 48 | 96 | 96 | 12 | 12 / 6 |
