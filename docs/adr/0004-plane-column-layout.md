# Plane of N identical 25 mm columns, front end under the module

**Status**: Proposed
**Date**: 2026-10-28
**Updated**: 2026-10-28 (user: end cap for N = 6/8, mounting holes at
the ends only, narrow end strip for N = 2/4; node power on a 2-pole
screw terminal, front panel on JST GH, ID ADC ADC121C027, 25 mm pitch)

Related: docs/adr/0003-plane-tile-and-module-interface.md (decisions
1-4 and 11 are replaced here, 5-10 and 12-14 stand),
docs/plane-requirements.md (R7-R9a, R21 amended), docs/pm-interface.md (module outline, screws, aux, host
height limits: unchanged).

---

## Context

ADR 0003 drew the plane as a 1x4 tile of ~155 x 130 mm: VBUS strip
along the top, slot front end in the column above or below the module,
node connectors at the bottom, 8 slots = two tiles. Two things moved
since:

- The aux contacts became a 2x3 1.27 mm header and socket (ADR 0003
  decision 7, 2026-10-27). Under the module outline the host now
  carries only four standoffs and one socket; the rest of the 22.9 x
  58.4 mm footprint is empty board, with 2.0 mm of part height allowed
  (pm-interface.md "Gap and part heights").
- The user wants one plane design in 2, 4, 6 and 8 modules, modules
  vertical, the plane as low as possible, and the routing of every
  module a straight copy of the others.

"Height" here is the board dimension along the module (top edge to
connector edge).

---

## Decision

1. **Column**: 25.0 mm pitch, ~72 mm tall, one per slot. Module
   vertical, input end up (host rotation 270 as in pm-interface.md).
   Height budget: 1.5 mm edge to module, 58.4 mm module, ~1.5 mm,
   10.2 mm node power terminal. The module (22.9 mm) sets the pitch;
   the bottom-edge connectors take ~18 mm of it. Standoff pads of
   neighbouring modules are 4.7 mm apart.
1a. **Node connectors (replaces ADR 0003 decision 11 and R21's
   Mini-Fit)**, both at the bottom edge, wire/cable entry from the
   edge:
   - Power: 2-pole 5.0 mm screw terminal, Degson
     DG128-5.0-02P-14-00A(H), LCSC C711349 (16k stock). UL 15 A / 300
     V, IEC 18 A, 12-24 AWG; 10.0 wide, <= 10.2 deep, 14.1 tall; PA66,
     tin-plated brass pins, nickel-plated cage. 14 AWG pair. The user
     makes the cables (screw terminal to the 19 V DC plug of the
     node's DC-ATX converter). Second source to verify: Degson
     DG500-5.08 (C691859). Still to confirm on the part: rising clamp
     and side wire entry.
   - Front panel (PWR_SW, RST_SW, PLED+, PLED-): JST GH 1x4 side entry
     SM04B-GHS-TB (C189895), clone XUNPU WAFER-GH1.25-4PWB (C3029379);
     8.25 wide, ~4 deep, positive latch; cable housing GHR-04V-S
     (C160418), crimp SSHL-002T-P0.2 (C189897).
   - Rejected on the way: Mini-Fit Jr 2x3 RA (21.3 mm deep to the rear
     pads, plane ~83 mm), PCB DC jacks (none stocked above 10 A at
     >= 24 V in the width).
2. **Front end under its own module.** LM5069-2, pass FET (TDSON-8),
   shunt (2512), INA238, TCA9535, the ID ADC and the small passives sit
   on the plane F side inside the module outline (max 2.0 mm, >= 1.0
   mm from standoff bodies and the aux socket). Parts over 2.0 mm (SMB
   clamp, 1210 bus caps, SOIC-4 photoMOS and opto, LEDs if any) go on
   the plane B side under the same outline. VBUS enters at the top,
   runs shunt -> FET -> S1 (VIN) within a few mm; S4 (VOUT) and S3
   (GND) leave at the bottom, ~10 mm from the node connector. The
   plane is assembled on both sides.
3. **Stackup and VBUS**: 4 layers, F parts and pours, In1 solid GND,
   In2 VBUS plane over the upper ~2/3 of the board plus the channel
   band (decision 5), B parts and pours. No VBUS strip along the top
   edge and no bus bar: 1 oz In2 across ~50 mm is ~1 mOhm per 100 mm,
   with the B and F pours in parallel.
4. **Family**: N = 2, 4, 6, 8. A plane is a core block plus N columns,
   generated from N (the 6-slot plane is a real 6-slot board). One MCU
   per plane, whatever N. The column copper is the same in every
   column and every variant, placed by translation only (pcbkit
   script, or KiCad 10 multichannel repeat layout).
5. **Rotating channel to the core.** The core sits at the left end.
   Below the modules a band of 8 pair positions (P0..P7) runs along
   the board on In2. Each column connects its own SDA/SCL to P0 at its
   core-side boundary and passes the far-side P0..P6 through to
   core-side P1..P7 (a jog of one pair per column); far-side P7 ends.
   At the core, Pk is the segment of column k+1. Every column has the
   same copper, no column is special, unused positions end at the far
   edge. 16 tracks at 0.35 mm pitch: a ~6 mm band on In2 under the
   lower part of the module, between the aux socket and S3/S4, so it
   adds no height (to confirm in the column layout). 3V3 and GND run
   straight through the band (same copper per column).
6. **Two lines per column (replaces ADR 0003 decisions 3 and 4 in
   part)**: SDA_n and SCL_n only.
   - INT_n dropped: the MCU polls each column's expander on the same
     mux visit as its INA238 (10-20 Hz). Status latency <= 100 ms;
     nothing in the safety chain uses INT (OV latch, current limits
     and PGOOD latch are local hardware).
   - ID_n becomes digital: a single-channel I2C ADC per column
     (TI ADC121C027, TSOT-23-6, LCSC C2670157, 9k stock; VA is the
     reference, so the reading is ratiometric to 3V3; address 0x50 /
     0x51 / 0x52 by one pin, clear of the INA238 and TCA9535; second
     source MCP3221A5T-I/OT, C36310, 0x4D, datasheet to check) reads the module's ID
     divider (10k pull-up, 100 nF as before). Same address in every
     column, behind the mux. The ID table and "ID open = slot fault"
     rule of pm-interface.md are unchanged; the MCU applies them to
     the ADC reading.
   - Mux: TCA9548A, channel k = column k+1 (all 8 used at N = 8). The
     plane front end (INA238, expander) moves to the G070's second I2C
     port.
7. **Input current scales with N** (decided 2026-10-28): N x
   ~265 W input is ~44 A at 48 V and ~56 A at 38 V for N = 8, twice the
   30 A class of ADR 0003. One 30 A hot-swap group per 4 columns. N <= 4: one group in the core (ADR 0003 decision 8 as is).
   N = 6/8: a second group in an end-cap block at the right end with
   its own keyed input connector; the In2 VBUS plane splits at the
   column 4/5 boundary. Column copper does not change; only the pour
   split and the end cap are added. Fault unit on the bus side stays
   4 slots; the MCU (one per plane) controls both groups through its
   second I2C port.
8. **Plane mounting**: holes at the ends only (core block and, for
   N = 6/8, the end cap; for N = 2/4 a narrow end strip, ~8 mm, with a
   hole pair). No hole strip
   along the top: height stays ~72 mm.

---

## Layout

```
 core              N x 25 mm                        end cap (N > 4)
 ~40 mm                                             ~25 mm
+--------+---------+---------+-- ... --+---------+--------+  -
| MCU    | S1   S2 | S1   S2 |         | S1   S2 | HS B   |  |
| mux    | :front: | :front: |         |         |        |  |
| iso12V | :end  : |         |         |         |        | 58.4
| 3V3    | module  | module  |         | module  |        |  |
| HS A   |   [aux] |   [aux] |         |   [aux] |        |  |
|<= P0..P7 rotating channel (In2, under the modules) =>|   |  |
|        | S4   S3 | S4   S3 |         | S4   S3 |        |  |
| [30A][link][12V] |[TB2][GH]   ...    |         | [30A]  | ~12
+--------+---------+---------+-- ... --+---------+--------+  -
                                                      ~72 mm total
```

Approximate sizes: N=2 ~100 x 72, N=4 ~150 x 72, N=6 ~215 x 72, N=8
~265 x 72 mm (core ~40, end strip ~8 or end cap ~25).

---

## Alternatives Considered

### ADR 0003 tile (front end beside the module, VBUS strip)
- **Optimizes for**: easy access to the front end, one side assembly.
- **Sharpest tradeoff**: ~130 mm tall; the area under every module is
  wasted.
- **Bets on**: board height not mattering in the box.

### Four lines per column (SDA, SCL, INT, analog ID)
- **Optimizes for**: interrupt-driven status, no ID ADC part.
- **Sharpest tradeoff**: 32 channel tracks (~11 mm band) and four MCU
  pins per column (ADC inputs for 8 columns do not exist on the G070).
- **Bets on**: status latency under 100 ms mattering.

### Core in the middle
- **Optimizes for**: half the VBUS current per side.
- **Sharpest tradeoff**: the channel must rotate toward the core from
  both sides, so right-hand columns are mirrored copper, not copies.
- **Bets on**: VBUS loss being the limit (it is not at 1 oz + pours).

### De Bruijn address straps instead of a mux
- **The idea**: an 8-track rotating channel tied to 00010111 at the
  core; each column taps 3 adjacent tracks as A0-A2, giving every
  column a unique address with identical copper.
- **Sharpest tradeoff**: one shared bus again (a stuck device hangs
  all columns), which ADR 0003 rejected.

### One 60 A hot-swap for N = 8
- **Sharpest tradeoff**: LM5069 at ~1 mOhm with 3-4 FETs; start-up SOA
  at 2 kW is hard to meet, and one fault drops 8 nodes.

---

## Consequences

### Positive
- Plane height ~72 mm instead of ~130 mm.
- One column layout serves every variant and every position.
- Two signal lines per column; MCU pin count no longer grows with N.

### Negative
- Two-sided plane assembly (JLC cost per board goes up).
- The front end is hidden under the module: probing needs the module
  off (test points on the B side under the outline, or at the channel
  band).
- One ADC part more per column.
- One MCU per plane means a dead MCU loses telemetry for up to 8
  slots (slots stay ON by hardware, ADR 0001 rules).

### Risks
- **Heat under the module**: the pass FET (~0.35 W) and shunt sit in
  the 7 mm gap with little fluid flow. Mitigation: FET drain pad into
  the In2/B copper; check the INA238 die temperature on the bench.
- **Board flex at 265 mm**: screwdriver force on the terminals of a
  long board held at the ends only (decision 8). Mitigation: support the connector
  edge while mating; if it still flexes, a stiffener bar on the B side
  (no height change) before a top hole strip (height -> ~80 mm).
- **Terminal depth and clamp type** come from one Degson drawing
  (the LCSC PDFs were not readable by script): confirm depth, wire
  entry and rising clamp on the part before the column is drawn.
- **Screw terminal in fluid**: a clamp that relaxes over years.
  Mitigation: rising clamp, ferrules on the 14 AWG, torque 0.4 Nm,
  check in the soak test.

---

## Open Questions

- [x] Input per 4 columns with an end cap for N = 6/8 (decision 7).
- [x] Plane mounting: holes at the ends only (decision 8).
- [x] N = 2/4 right-hand end: hole pair in a narrow end strip (~8 mm).
- [x] Front-panel connector: JST GH 1x4 (decision 1a).
- [x] ID ADC: ADC121C027 (decision 6).
- [x] Node power: DG128 2-pole screw terminal (decision 1a).
- [ ] DG128 drawing check (depth, clamp, entry) and second source.
