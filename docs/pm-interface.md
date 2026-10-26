# Port module interface spec (PM-19V-250W and its hosts)

**Date**: 2026-10-26
**Status**: Proposed, with ADR 0003
**Applies to**: the port module (`power/pm-19v-200w/`, 250 W update),
the plane tile column, the bench carrier (`power/pm-bench-carrier/`).

Every mount footprint (module screw pads and aux pads, host standoffs
and pogos, host silk outline) is generated from the numbers in this
file. Change a number here first, then regenerate; never edit a
generated footprint by hand.

## Coordinates

Module frame: origin at the centre of the module outline, viewed from
the module's top (F side, inductors up), +x toward the output end,
+y down (KiCad convention). All positions below are in this frame, mm.

The module sits on the host F side up, so the host footprint uses the
same coordinates, not mirrored. On the plane tile the module stands
with the input end up (toward VBUS): host footprint rotation 270
(module +x points down the board). Bench carrier: rotation 0.

## Outline

- 58.4 x 22.9 mm (eighth-brick outline, nothing else from DOSA).
- Module PCB 1.6 mm, 4 layers, ENIG (pads are contact surfaces).
- Corner radius: none (as now).

## Screws: positions and nets

Four M2 screws through the module into SMT standoffs on the host. The
screws carry the power: the module's pad is clamped between the screw
head and the standoff face.

| Screw | Net | x | y | Current (cont. / fault) |
|---|---|---|---|---|
| S1 | VIN | -25.70 | -7.95 | 5.5 A at 48 V, 7 A at 38 V |
| S2 | GND (input) | -25.70 | +5.95 | as S1 |
| S3 | VOUT | +25.70 | -7.95 | 13.2 A / 20 A |
| S4 | GND (output) | +25.70 | +5.95 | 13.2 A / 20 A |

S2 and S4 sit 2.0 mm closer to the centre line than S1 and S3. That is
the polarisation: turned 180 degrees, (x, y) -> (-x, -y), the module's
holes land at y = +7.95 and -5.95, where no host standoff is (2.0 mm
off against a 0.2 mm hole float), so not even one screw goes in with
the module reversed. A pattern where only one screw is offset is not
enough: two diagonal screws still line up and hold the module with VIN
on a GND standoff. Keep the offset at 2.0 mm or more, and keep it the
same at both ends.

Insets: 3.5 mm from the short edges; 3.5 mm from the top long edge,
5.5 mm from the bottom one. The 6.2 mm underside pad leaves 0.4 mm to
the edge at S1 and S3.

## Screw pads

Module (one footprint, four instances, net per table):
- Plated hole 2.4 mm (M2 clearance, +/-0.2 float).
- F pad 5.0 mm round (screw head 3.8 + lock washer 4.4 OD).
- Inner layers 4.0 mm round, solid to the net's pour.
- B pad 6.2 mm round (standoff face 5.56 mm), no solder mask, no paste.
- 8 vias 0.3 mm drill on a 4.4 mm circle inside the pads, untented
  (no solder on the module side, the joint is clamped).
- Solid connection to the pour (no thermal spokes) on every layer.

Host standoff (one part per designator, JLC places one part each):
- Sinhoo SMTSO2060CTJ, LCSC C2915628: M2 thread, 6.0 mm length,
  5.56 mm body, 3.6 mm pilot 1.53 mm long.
- Plated hole 3.75 mm for the pilot, round pad 6.4 mm, 4 paste
  apertures (as drawn at tag pm-mount-drawn), solid to the pour.
- 8-12 stitching vias around the pad to the inner planes.

Hardware: M2 x 5 pan or cheese head, A2 stainless, with a spring lock
washer (DIN 127 / split ring) under the head. Engagement: 5 - 1.6
(PCB) - 0.5 (washer) = 2.9 mm. Torque: 0.25 Nm (to confirm on a
sample). Check: is the standoff thread through or blind, and how deep.

## Aux contacts

Six flat pads on the module B side, six pogo pins on the host. Signals
only, never power.

| Pad | Net | x | y | Direction | Note |
|---|---|---|---|---|---|
| A1 | OV_KILL | +10.16 | -9.12 | module -> host | open drain, low = overvoltage, sets the slot latch |
| A2 | GND | +12.70 | -9.12 | | reference for ID and the open drains |
| A3 | PGOOD | +15.24 | -9.12 | module -> host | open drain, high = good |
| A4 | ID | +10.16 | -6.58 | module -> host | resistor to GND, table below |
| A5 | GND | +12.70 | -6.58 | | the spare, second GND |
| A6 | RUN | +15.24 | -6.58 | host -> module | open = on, host pulls low = off |

- Pitch 2.54 mm both ways; block centre (+12.70, -7.85). Lies on the
  module's B side under the output sense resistor, clear of the
  B-side parts (Q2, R12, C26, C5 in the current layout).
- Module pad: 2.2 mm round, ENIG, solder mask opening 2.4 mm, no paste.
  No vias in the pad.
- Host pogo: Xinyangze YZ130115073P-01, LCSC C5157258, 2 mm base, 7.3
  mm free / 5.5 mm fully compressed (from the LCSC listing, no
  drawing: measure a sample before freezing). At the 6.0 mm gap it
  travels 1.3 mm and has 0.5 mm left.
- Host side: OV_KILL and PGOOD pulled up in the column (open = good,
  so an absent module reads "good"; presence comes from ID). RUN is
  the module's UVLO divider node (~1.9 V at 60 V in): the host only
  ever pulls it low through an open drain, never drives it high.

ID table (module resistor to GND, host 10k pull-up to 3V3, 1k series,
100 nF):

| R_ID | V/3V3 | Module |
|---|---|---|
| 0 (short) | 0 | fault |
| 2.2k | 0.18 | reserved |
| 4.7k | 0.32 | reserved |
| 10k | 0.50 | PM-19V-200W rev 0.1 |
| 22k | 0.69 | PM-19V-250W |
| 47k | 0.82 | reserved |
| open | 1.00 | no module |

## Gap and part heights

- Gap module B surface to host F surface: 6.0 mm (the standoff length).
- Module B side: parts max 2.7 mm (1210 caps at 100 V are the tallest).
- Host under the module outline: parts max 2.0 mm, and none within
  1.0 mm of a standoff body or a pogo. That leaves 1.3 mm air.
- Module F side: parts max 12 mm (inductors; recheck with the 250 W
  inductors). No part taller than 1.5 mm within 6.5 mm diameter of a
  screw (head, washer and screwdriver access).
- Keep-out on the module, both sides and inner layers: 6.5 mm diameter
  around each screw for other nets' copper and for parts. Host: 6.8 mm
  diameter around each standoff for other parts.

## What the current module layout has to move

From the placement at 797cb1e (positions in board coordinates, module
centre at (129.2, 111.45)):

- S1 (103.5, 103.5): C1 and C3 (B, input caps) overlap by ~1.3 mm in
  x; D1 (B) by 0.35 mm in y.
- S2 (103.5, 117.4): R4, C9 and C11 (B, gate drive and OVLO) inside
  the keep-out.
- S3 (154.9, 103.5): C24 (F, 100 uF) overlaps by 0.7 mm.
- S4 (154.9, 117.4): C25 (F, 100 uF) overlaps by 0.7 mm.
- Aux block: clear as placed.
- TH1 (NTC), and J1/J2 (DOSA footprint and aux row) go away.

These are inputs for the 250 W re-placement, not edits to make on the
current board.

## Generator

A script (power/tools/pmmount.py, restore from tag pm-mount-drawn and
rework) reads the numbers above from one table in the script, emits:
- module: PM_Mount_M2 (x4 by net), PM_Aux_Pads_2x03;
- host: PM_Standoff_M2x6_SMTSO, PM_Pogo_YZ130115073P,
  PM_Module_Outline (silk outline with S1 marker and the input end
  marked "VIN");
- `place`: puts a host set at a module centre and rotation.
