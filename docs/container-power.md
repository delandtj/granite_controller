# Container power

Status: exploring (2026-11-15). Nothing here is decided; the open
items are listed at the end.

One container holds 2 pods of 48 nodes (6 blocks each), 3 GPU nodes,
4 storage nodes and the switches that come with them. The feed is
3 x 400 V with neutral, 63 A per phase.

The drawing is `cabling/container-power.yml` (WireViz, generated
`.svg` and `.bom.tsv` next to it). What hangs on one block breaker is
in `docs/block-cabling.md`.

## How the phases are shared

- Each pod has its own 48 V bus. A rectifier shelf feeds it, with its
  230 V modules in multiples of three, one third on each phase. All
  modules share the bus load, so a pod loads the three phases equally
  whatever nodes are on or off.
- Losing a phase costs a pod a third of its rectifiers, not a third of
  its nodes.
- Each bus is earthed at one point, at its shelf (docs/adr/0003
  decision 14). Two pods are two buses and two earth points, both on
  the container's earth bar.
- The 230 V loads hang on one three-phase rack PDU and are spread by
  hand: one GPU node per phase; the storage nodes and switches fill
  up the phases after that. A device with two supplies takes them
  from two different phases.
- Cooling (pumps, dry cooler) shares the same 63 A. Three-phase
  motors load the phases equally by themselves.

## Budget

A feed of 63 A per phase is 43.5 kW. Taking 80 % for continuous load
leaves 50 A per phase, 34.8 kW.

Per block, at the wall. Assumed: 95 % port module, 95 % rectifier,
25 W for the switch and the core. Not measured.

| | Typical (165 W per node) | Maximum (250 W per node) |
|---|---|---|
| 8 nodes at 19 V | 1.32 kW | 2.00 kW |
| Block at 48 V | 1.41 kW | 2.13 kW |
| Block at 230 V | 1.49 kW | 2.24 kW |

Per phase:

| Load | Typical | Maximum |
|------|---------|---------|
| Pod A (6 blocks, 8.9 / 13.5 kW) | 12.9 A | 19.6 A |
| Pod B (6 blocks, 8.9 / 13.5 kW) | 12.9 A | 19.6 A |
| Both pods | 25.9 A | 39.1 A |
| Left of 50 A continuous | 24.1 A | 10.9 A |
| Left of 63 A | 37.1 A | 23.9 A |

With every node at its maximum, the GPU nodes, storage nodes,
switches and cooling together have 10.9 A per phase, 7.5 kW in all,
before the feed passes 80 %. At the typical node load they have
24.1 A per phase, 16.6 kW.

## Sizes that follow

- Rectifier shelf per pod: 12.8 kW out at most, about 270 A at 48 V.
  Six 3 kW modules (two per phase) give 18 kW; with one phase lost
  12 kW remain, which carries the typical 8.5 kW but not the maximum.
  Nine modules keep the maximum with a phase lost.
- Branch per shelf: 32 A three-phase breaker, 5 x 6 mm2.
- Bus to blocks: six 63 A DC breakers per pod, one per block (the
  option drawn in docs/block-cabling.md, not taken).
- Neutral: full size. Balanced loads leave little current in it, but
  a lost phase or an uneven PDU does not.

## Open

- **GPU and storage node power**, and whether they have two supplies.
  Until then the PDU branch and the phase table above are not
  complete. Three GPU nodes of more than 2.5 kW each do not fit beside
  two pods at maximum load.
- **Cooling load.** Not known, and it comes out of the same 63 A.
- **Rectifiers.** Not chosen: make, module size, single-phase modules
  on a three-phase shelf or three-phase modules, spare modules.
- **Bus voltage.** The planes are designed for 40-60 V in
  (docs/handoff.md); 48 V or 54 V on the bus is not fixed. At 54 V the
  bus current drops to about 240 A per pod.
- **Earth leakage protection** in front of the rectifiers (which type
  of device they need) is not looked at.
- **Where it all sits**: board, shelves and buses relative to the two
  tanks. Every length in the drawing is a placeholder.
