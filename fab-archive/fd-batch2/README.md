# FD batch 2: one 48-node pod (6 blocks)

Order sheet for the second JLC order (user, 2026-11-26). The batch 1
boards in hand stay for development; this order fills the pod on its
own.

| Board | Pod (6 blocks) | Spares | Order | Fab set |
|---|---|---|---|---|
| controller-fd | 6 | 2 | **8** | `../fd-batch1/controller-fd` (unchanged) |
| pp-48-core-fd | 6 | 2 | **8** | `../fd-batch1/pp-48-core-fd` (unchanged, STM32G070) |
| pp-48-unit-fd | 24 | 6 | **30** | `pp-48-unit-fd/` here (new BOM, see below) |
| PM-19V-250W-B | 48 | 8 | **56** | `../fd-batch1/pm-19v-250w-b` (unchanged, R12 10k) |

## What changed and what did not

Fresh `pcbkit jlcfab` sets from master (2026-11-26; the unit set after
the slot-only FET change, same commit as this line) were compared with
the batch 1 sets:

- **controller-fd, pp-48-core-fd**: BOM and CPL identical. Reorder the
  batch 1 sets as they are. The core keeps the STM32G070CBTx: plane
  firmware updates stay on SWD (the G0B1 variant for updates from the
  controller is not made).
- **PM-19V-250W-B**: BOM identical, Gerbers and drill identical apart
  from header lines. The fresh CPL differs in bottom-side rotations
  (pcbkit's bottom-side handling changed after the batch 1 set was
  made); the batch 1 set is the one that assembled the modules running
  on the bench, so reorder **that** one.
- **pp-48-unit-fd**: new set, generated from master:
  - BOM: **slot FETs only** (user, 2026-11-26, the order date wins over
    the stock): Q1 / Q51 -> ISC035N10NM5LF2 (C22408528, extended part),
    were BSC070N10NS5; Q202 / Q203 on the unit input stay BSC040N10NS5
    (C534334) as in batch 1. R4 / R54 / R234 10k -> 11k (C25749), UVLO
    off 35.0 V / on 38.0 V (batch 1: 38.3 / 41.3 V).
  - Copper: CAN termination stubs 0.3 -> 0.2 mm and the CAN nets
    renamed (28ecb36, after the batch 1 tag). No other copper change.
  - CPL identical to batch 1; the FET rotation corrections are the same
    as for the parts they replace (Q1 / Q51 270 -> 180, Q202 / Q203
    90 -> 0). Still check Q1 / Q51 in JLC's placement preview.
  - Checks: ERC 0; DRC with schematic parity: 0 unconnected, parity
    clean, the known 15 (2 diff pair gaps at the connector ends, 4
    library copy mismatches, 6 PTH in courtyards, 3 silk at the edge).

## Before ordering

- **ISC035N10NM5LF2 stock**: 60 needed (30 units x 2, slot positions).
  JLC had 58 on 2026-11-12; the stock lookup used here was broken (it
  showed 0 for basic parts too), so check on the JLC site. If it is
  still 58, 29 units get it: order 29 or source two FETs.
- **What slot-only leaves**: Q202 / Q203 (BSC040N10NS5) are outside their
  SOA if the unit input holds a short in power limit: a short on the
  unit itself, a slot FET that failed, or slot and unit breakers
  tripping together on a dead short (bench plan test 6). ADR 0005
  decision 3a.
- **Unproven**: the new FET has not passed the short test (bench plan
  test 6), and bench tests 1 to 5 (ripple, C201, latch inputs, CAN
  counters, INA237) are open. See docs/handoff.md, "Plan from here".
- C25749 (11k 0402): not checked whether JLC lists it as basic.

## Not from JLC, per pod

Six block switches and six CUI DQD50-48-S12-T (one per block), the
48 V supplies (6 x CB3000), bus bars and holder, an ideal-diode stage
per supply (not designed), a Modbus TCP to RTU gateway, a management
switch for the controllers, cabling (docs/block-bom.md) and the printed
frames and carriers.
