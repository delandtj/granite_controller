# FD batch 2: one 48-node pod (6 blocks)

Order sheet for the second JLC order (user, 2026-11-26). The batch 1
boards in hand stay for development.

| Board | Pod (6 blocks) | Order | Fab set to upload |
|---|---|---|---|
| controller-fd | 6 | **5** | `controller-fd/` here (U5 changed) |
| pp-48-core-fd | 6 | **10** | `pp-48-core-fd/` here (U1, R25 / R26 changed) |
| pp-48-unit-fd | 24 | **30** | `pp-48-unit-fd/` here (new BOM, see below) |
| PM-19V-250W-B | 48 | **50** | `pm-19v-250w-b/` here (a copy of the batch 1 set, unchanged, R12 10k) |

The quantities are the user's (2026-11-26). A pod of 6 blocks needs 6
controllers, 6 cores, 24 units and 48 modules: the controller count is
one under that and the core count four over, the user's call with the
batch 1 boards in hand.

PM-19V-250W-B (`pm-19v-250w-b/` here) is a byte-identical copy of the **batch 1** set, on purpose: its BOM is
unchanged and its Gerbers and drill are identical apart from header
lines, but a fresh CPL differs in the bottom-side rotations (pcbkit's
bottom-side handling changed after the batch 1 set was made). The batch
1 set is the one that assembled the modules running on the bench, so
that is the one to reorder.

The core keeps the STM32G070CBTx: plane firmware updates stay on SWD
(the G0B1 variant for updates from the controller is not made).

## Substitutions (2026-11-26)

The batch 1 parts below ran out at JLC. Each replacement was checked
against both datasheets and approved by the user; all are in the source
(the generators, the schematics and the board fields), none of them
changes copper, a footprint or a net.

| Board | Ref | Was | Now | Why it fits |
|---|---|---|---|---|
| controller-fd | U5 | TMP1075DR, C2878381 | **TMP75AIDR, C30389** (TI) | SOIC-8, LM75 pinout and register map, address 0x48 with A0-A2 low as today |
| pp-48-core-fd | U1 | LM5164DDAR, C477928 | **LM5164DDAT, C2071198** (TI) | same die and HSOP-8 package, smaller reel |
| pp-48-core-fd | R25, R26 | 60.4R AC0603FR-0760R4L, C144645 | **RC0603FR-0760R4L, C245987** (Yageo) | same 60.4R 1 % 0603, RC series instead of AC |
| pp-48-unit-fd | R301, R302 | the same C144645 | **C245987** | the same (link.py is shared with the core) |
| pp-48-unit-fd | J301 | A2541WR-2x7P, C5136987 (CJT) | **PZ254-2-07-W-8.5, C2894987** (HCTL) | same 2.54 mm pitch, 6.00 mm mating pin, 3.00 mm tail, 1.02 mm holes; plastic 2.5 mm per pin against 2.54, which the body width absorbs, so the footprint, the mating height and the 4 mm board gap stay |
| pp-48-unit-fd | J3, J53 | HX PM1.27-2x3P TP, C41397112 (hanxia) | **HC-PM127-4.3H-2x3PS-02A, C42163098** (Hong Cheng) | the second source already named in docs/pm-interface.md: same 4.30 mm body / 4.60 mm over the PCB, 1.27 mm pitch, 0.76 x 2.05 pads, 6.0 mm row span |
| pp-48-unit-fd | Q1, Q51 | ISC035N10NM5LF2, C22408528 (Infineon) | **CSD19532Q5B, C473333** (TI) | 100 V, 4.9 mOhm max, SON 5x6 on the same TDSON-8-1 land (pinout and pitch checked against TI's land pattern); linear-mode SOA about 1.3 A at 48 V / 10 ms at T_C 25 C (SLPS414B figure 10) |
| pp-48-unit-fd | C1, C51 | 220 nF 0603, C21120 | **100 nF 0603, C14663** (Yageo, basic) | the slot LM5069 timer caps, with the new FET: fault time 10.4 -> about 4.7 ms typ (about 9 ms worst case), insertion 160 -> about 73 ms. The unit input's C203 stays 220 nF |
| pp-48-unit-fd | U2, U52, U208 | INA237AIDGSR, C2864837 | **INA228AIDGSR, C2887910** (TI) | pin-compatible VSSOP-10, 85 V; address straps unchanged |

The slot FET and the 100 nF timer cap go together: ADR 0005 decision 3a
has the SOA sum (about 1.3 A at 48 V / 10 ms at 25 C, x 0.72 at T_C
60 C, x (10 / 4.7)^0.5 for the shorter fault time -> about 1.3 A hot
against the 0.96 A the power limit asks; at 60 V about 0.9 A against
0.84 A, thin). It also supersedes the 2026-11-11 note that had read and
rejected this FET.

**The INA228 is hardware only here.** The plane firmware still talks to
an INA237 (different register widths and ADC range), and "INA237" is
still written all over docs/ and firmware/: a separate job.

What did **not** change: **slot FETs only** (user, 2026-11-26, the
order date wins over the stock) - Q202 / Q203 on the unit input stay
BSC040N10NS5 (C534334), outside their SOA if the unit input holds a
short in power limit (a short on the unit itself, a slot FET that
failed, or slot and unit breakers tripping together: bench plan test
6). ADR 0005 decision 3a. R4 / R54 / R234 stay 11k (UVLO off 35.0 V /
on 38.0 V; batch 1: 10k, 38.3 / 41.3 V). Copper: unit-fd's CAN
termination stubs 0.3 -> 0.2 mm and the CAN net rename (28ecb36, after
the batch 1 tag); no other copper change on any of the three boards.

## Checks

Generated 2026-11-26 with `pcbkit jlcfab <board>.kicad_pcb <dir>` from
each project directory, after the substitutions.

- ERC (`kicad-cli sch erc --severity-all`): **0** on all three.
- DRC (`kicad-cli pcb drc --schematic-parity --severity-all
  --refill-zones`): 0 unconnected and 0 parity issues on all three, and
  the same violations as before the change - controller-fd 8 (silk at
  the board edge), pp-48-core-fd 6 (3 library copy mismatches, 3 silk
  at the edge), pp-48-unit-fd the known 15 (2 diff pair gaps at the
  connector ends, 4 library copy mismatches, 6 PTH in courtyards, 3
  silk at the edge).
- Against the batch 1 sets: Gerbers and drill identical apart from the
  header dates on controller-fd and pp-48-core-fd; unit-fd as the
  earlier batch 2 set. **All three CPLs are identical to batch 1**
  (same positions and rotations; only the LCSC numbers in
  `-rotations.txt` differ). BOMs differ in exactly the lines above
  (plus unit-fd's 11k and the slot FET already in the batch 2 set).
- Rotation corrections: pcbkit's table is keyed by LCSC number, so each
  new part got an entry in the project's `pcbkit.toml` by analogy with
  the part it replaces - C2071198 (as C477928), C2894987 (as
  C5136987), C42163098 (as C41397112), C473333 (as the TDSON-8-1
  entries), C2887910 (as C2864837). **Assumed, not fitted against each
  part's own EasyEDA footprint**: check J301, J3 / J53, Q1 / Q51, U2 /
  U52 / U208 and core U1 in JLC's placement preview. C30389 (U5) and
  C245987 / C14663 need no entry: the generic SOIC-8 rule and the
  unpolarized-passive rule cover them. No part is missing from the
  table ("NOT IN THE TABLE" is empty in all three `-rotations.txt`).

## Stock check (JLC parts API, 2026-11-26)

208 BOM lines over the four boards at the quantities above: **every
line has stock >= need**, nothing SHORT. All eight new parts are well
stocked (TMP75AIDR 4439 for 5, LM5164DDAT 375 for 10, RC0603FR 38144
for 80, PZ254-2-07 179 for 30, HC-PM127 6554 for 60, CSD19532Q5B 5095
for 60, INA228 2063 for 90, CC0603 100n 49.9M for 60).

One line is tight (under 1.5x):

| Part | Where | Need | Stock | Ratio |
|---|---|---|---|---|
| C623510 MCP65R46-2.4 | PM-19V-250W-B U2 / U3 | 100 | 115 | 1.15 |

Totals for parts shared between boards were checked as well; the
largest are C1525 (100n 0402) 645, C25741 (100k) 590, C25744 (10k)
590, C8545 (2N7002) 550, all with millions in stock. The tightest
shared lines are C19100281 (chain socket) 40 of 396, C338133 (2.2u
100V 1210) 180 of 2405, C5333425 (link socket) 40 of 1349 and
C2990209 (SMDJ60A) 40 of 1607 - 10x or better.

## Before ordering

- Re-check C623510 (115 for 100) on the day of the order.
- Check the placement preview for the five assumed rotations above.
- **Unproven**: the new slot FET has not passed the short test (bench
  plan test 6), and bench tests 1 to 5 (ripple, C201, latch inputs,
  CAN counters, the current monitors) are open. See docs/handoff.md,
  "Plan from here".

## Not from JLC, per pod

Six block switches and six CUI DQD50-48-S12-T (one per block), the
48 V supplies (6 x CB3000), bus bars and holder, an ideal-diode stage
per supply (not designed), a Modbus TCP to RTU gateway, a management
switch for the controllers, cabling (docs/block-bom.md) and the printed
frames and carriers.
