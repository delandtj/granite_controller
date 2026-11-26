# FD batch 2 order: restart here

Self-contained state of the second JLC order (one 48-node pod plus spares),
as of 2026-11-26. Read this first when picking the order up again; the
order sheet with the per-file details is fab-archive/fd-batch2/README.md,
the wider bench history is docs/handoff.md.

## Status

- Fab sets are generated, checked and committed (master b77862c and
  earlier). Nothing is ordered yet. The user places the order.
- JLC's "Reorder" of the batch 1 orders is refused (parts out of stock),
  so this is a **new order with uploaded files** for every board.
- Stock re-checked 2026-11-26 against these exact files and quantities:
  every line in stock (tools/jlc_stock.py, below).

## What to order

| Board | Qty | Upload (gerbers zip, BOM csv, CPL csv) |
|---|---|---|
| controller-fd | 5 | fab-archive/fd-batch2/controller-fd/granite-controller-fd-* |
| pp-48-core-fd | 10 | fab-archive/fd-batch2/pp-48-core-fd/pp-48-core-fd-* |
| pp-48-unit-fd | 30 | fab-archive/fd-batch2/pp-48-unit-fd/pp-48-unit-fd-* |
| PM-19V-250W-B | 50 | fab-archive/fd-batch1/pm-19v-250w-b/pm-19v-250w-b-* (unchanged) |

Quantities are the user's (2026-11-26): a 48-node pod is 6 blocks =
6 controllers, 6 cores, 24 units, 48 modules; the user's numbers include
spares and use batch 1 boards for the rest (batch 1 boards in hand are
for development). Not ordered: PM-19V-250W rev A (out for the first run;
its U1 LTC7810 C581067 is out of stock anyway).

PM-19V-250W-B: upload the batch 1 set, not a fresh `pcbkit jlcfab`
output. A fresh CPL differs in bottom-side rotations (pcbkit changed
after batch 1); the batch 1 files assembled the modules that run on the
bench. BOM and copper are identical either way.

## In JLC's placement preview, check

Rotations of the substituted parts are assumed by analogy with the parts
they replace (pcbkit.toml rotation tables, keyed by LCSC number):

- unit-fd Q1, Q51 (CSD19532Q5B, C473333): pin 1 / gate where Q202 / Q203 have theirs.
- unit-fd U2, U52, U208 (INA228AIDGSR, C2887910): pin 1 as the INA237 was.
- unit-fd J3, J53 (HC-PM127-4.3H, C42163098): socket body over the 2x3 land.
- unit-fd J301 (PZ254-2-07-W-8.5, C2894987): pins pointing off the board edge, as J302.
- core-fd U1 (LM5164DDAT, C2071198): pin 1 and exposed pad.
- Also, as always: the -B module's bottom-side Q3.

## Substitutions (batch 1 part out of stock at JLC)

All BOM-only: same footprints, no copper change, placement files identical
to batch 1. Each checked against datasheets on 2026-11-26.

| Board | Ref | Batch 1 | Batch 2 | Why it is equivalent |
|---|---|---|---|---|
| controller-fd | U5 | TMP1075DR C2878381 | TMP75AIDR C30389 | LM75 family: SOIC-8 pinout, 0x48, temp register 0x00. Firmware writes one CFG byte 0x00 (4b53035) |
| core-fd | U1 | LM5164DDAR C477928 | LM5164DDAT C2071198 | same die and package, smaller reel |
| core-fd | R25, R26 | 60.4R C144645 | 60.4R RC0603FR C245987 | same value, Yageo standard grade |
| unit-fd | R301, R302 | 60.4R C144645 | C245987 | as above |
| unit-fd | J301 | A2541WR-2x7P C5136987 | PZ254-2-07-W-8.5 C2894987 | 6.00 mating pin, 3.00 tail, 1.02 holes, 2.54 pitch; plastic 2.5 vs 2.54 |
| unit-fd | J3, J53 | HX PM1.27-2x3P TP C41397112 | HC-PM127-4.3H-2x3PS-02A C42163098 | both 4.30 body / 4.60 overall, same 0.76 x 2.05 pads, 6.0 span: module standoffs stay 7 mm |
| unit-fd | Q1, Q51 | BSC070N10NS5 (batch 1) / ISC035N10NM5LF2 (planned) | CSD19532Q5B C473333 | ISC035 gone at JLC. 10 ms SOA ~1.3 A at 48 V, 25 C (SLPS414B fig. 10); only enough with the shorter fault time below |
| unit-fd | C1, C51 (slot LM5069 timer) | 220n C21120 | 100n C14663 | fault 10.4 -> ~4.7 ms typ (~9 ms worst), insertion 160 -> ~73 ms; SOA then ~1.3 A hot (T_C 60 C) vs 0.96 A demand: ADR 0005 3a |
| unit-fd | U2, U52, U208 | INA237AIDGSR C2864837 | INA228AIDGSR C2887910 | pin-compatible, 85 V; different register scaling: plane firmware 5cb6a8a |
| unit-fd | R4, R54, R234 | 10k | 11k C25749 | UVLO on 38.0 / off 35.0 V as designed (batch 1 is 41.3 / 38.3) |
| unit-fd | Q202, Q203 | BSC040N10NS5 | unchanged | slot-only linear FET decision (user, order date first) |

## Decisions (user, 2026-11-26)

- Global sourcing / consignment: not an option (date).
- Slot FETs only get the hot-swap-capable part; the unit input keeps
  BSC040N10NS5 (not short-proof if the unit input itself holds a short:
  unit-side short, failed slot FET, or slot and unit breakers tripping
  together, bench plan test 6).
- INA228 with plane firmware support, rather than waiting for INA237.
- R12 diode-emulation rework on the -B module: parked; R12 stays 10k.
- Core stays STM32G070 (plane updates by SWD; the G0B1 variant for
  updates from the controller is not made).

## If a part runs out again before the order

1. `tools/jlc_stock.py check NAME=BOM=QTY ...` (the command for this
   order is in the script's docstring) shows what is short.
2. `tools/jlc_stock.py search MODEL_FAMILY` for candidates.
3. Datasheets: LCSC blocks direct PDF links. Fetch
   `https://www.lcsc.com/datasheet/<LCSC>.pdf` with a browser User-Agent
   and `-e https://www.lcsc.com/product-detail/<LCSC>.html`, grep the HTML
   for `https://datasheet.lcsc.com/datasheet/pdf/<hex>.pdf`, download
   that, render drawings with `pdftoppm -r 140 -png`.
4. Change the source, never only the CSV: power/pp-48-fd/tools/sch/blocks
   (unit, core: `python3 power/pp-48-fd/tools/gensch.py unit|core`),
   controller-fd/schematic (symbol fields), then the same Value / LCSC /
   Description on the board footprints (property edits only), a
   rotation entry in the board's pcbkit.toml for a new LCSC number.
5. Checks: `kicad-cli sch erc --severity-all`; `kicad-cli pcb drc
   --schematic-parity --severity-all --refill-zones -o <scratch>`; known
   baselines: unit-fd 15 (2 diff_pair_gap, 4 lib_footprint_mismatch,
   6 pth_inside_courtyard, 3 silk_edge_clearance), core-fd 6,
   controller-fd 8; always 0 unconnected, 0 parity.
6. `~/Electronics/pcbkit/bin/pcbkit jlcfab <board>.kicad_pcb
   fab-archive/fd-batch2/<board>` from the project dir; compare the BOM and
   CPL with fab-archive/fd-batch1 (only the intended lines may differ).
7. Update fab-archive/fd-batch2/README.md and this file.

## After the boards arrive

- **Plane firmware first**: every core must run master's plane-fd (INA228
  support, 5cb6a8a, plus decision 7d) before a batch 2 unit is used; an
  older image reads an INA228 with INA238 scaling (wrong V / A / W and
  power limit). SWD over the pogo: site/src/commissioning/flash-core.md.
- Controller-fd: master's granite-fw by OTA (TMP75 CFG write). OTA pull
  procedure: docs/handoff.md, 2026-11-25 (late).
- Bench before the pod: unit-fd tests 1 to 6 (docs/unit-fd-emi-bench-plan.md),
  in particular the short test on a slot with the CSD19532Q5B and the
  100 nF timer (does the FET hold, does the unit trip with the slot), and
  the INA228 readings against a meter.

## Open risks

- CSD19532Q5B SOA margin is from datasheet curves at 25 C, derated by
  calculation; ~1.07x at 60 V / 10 ms at 25 C before the shorter fault
  time. Unproven until the short test.
- Rotations of five substituted parts are assumed (preview check above).
- MCP65R46-2.4 (C623510, -B module U2 / U3): 115 in stock for 100 on
  2026-11-26. Order soon or re-check.
- granite-sim `first_boot_then_power_a_node_on` is wall-clock flaky under
  a fully parallel `cargo test`; passes alone.
