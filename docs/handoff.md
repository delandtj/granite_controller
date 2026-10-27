# Handoff - granite controller (updated 2026-10-27)

Read this first, then docs/controller.md and docs/power-board.md.

## Start here (2026-10-27, later): PM-19V-250W cost and parts findings

The routed board (1192e81, next section) is unchanged; this is costing
and part choice, no board edits yet.

Cost per board (BOM: power/pm-19v-250w/production/bom.csv, LCSC prices
pulled 2026-10-27 via wmsc.lcsc.com/ftps/wm/product/detail?productCode=):
- LCSC parts without L1/L2: $49.56 at 5 boards, $44.58 at 100. The
  LTC7810 (C581067) is $32.55 at LCSC with 13 in stock; the user found
  it at $19 at Digikey. With that: parts ~$36 (5) / ~$32 (100).
- Second-largest line: C24/C25 Kemet T523H107M035APE070 polymers,
  $5.67 each ($11.34 per board).
- Estimates, not quotes: PCB + JLC two-sided assembly ~$27-39 per board
  at 5, ~$5-8 at 100. Total with the Digikey LTC7810 ~$70-82 (5),
  ~$42-45 (100); with the inductor swap below ~$65-75 / ~$40.
- LCSC stock 0 on 2026-10-27: MCP65R46T-1202E/CHY (C116625, U2/U3),
  C77025 (C13 2.2n); basic parts C25804, C25744, C1523, C14663 show 0
  at LCSC but JLC basic stock is separate (not checked).

Inductor alternative (JLC library scan, 15-33 uH, ~12,000 parts, via
POST jlcpcb.com/api/overseas-pcb-order/v1/shoppingCart/smtGood/
selectSmtComponentList; LCSC search API is blocked for scripts):
- COILANK APS1510M220F (C49261404): 16.5 x 15.5 x 10.0 mm max, 22 uH,
  DCR 15.8 mOhm max, Irms 14 A, Isat 18.7 A (at ~40 % drop), 303 in
  stock, $2.40. Looks like an XAL1510 clone. JLC also stocks the
  Coilcraft XAL1510-223MED (C3911658) at $17.89, 29 in stock.
- Open on the APS1510: Isat is defined at ~40 % drop (Coilcraft 10-30 %),
  check L at ~12 A (worst-case current limit); absolute maximum 55 V DC
  vs ~41 V across it at 60 V in, ~48.5 V at the 67.5 V OVLO; footprint
  not yet overlaid on pm.pretty's XAL1510 one; immersion compatibility
  of the metal-dust core coating unknown.
- Fit check 2026-10-27 (COILANK APS-F datasheet, LCSC pdf for
  C49261404): APS1510 body 16.5 +-0.3 (along the terminals) x 15.5
  +-0.3 (across) x 10.0 max; bottom terminals 13.2 x 3.2 with a 7.4 gap
  (outer 13.8). The XAL1510 pads (3.18 x 13.2 at +-5.3, outer 13.78)
  cover the terminals almost 1:1, so no footprint change is needed
  (COILANK's own land is bigger: 4.5 x 15.0 pads, 6.0 gap; optional).
  Worst-case body 15.8 x 16.8 sits inside the existing courtyard
  (15.9 x 16.9) at the current L1/L2 positions: 1.45 mm body to body,
  3.1 mm to the top/bottom edges, 0.28-0.4 mm to R9, U2, U3, C27, R6,
  R10 courtyards (same as now), >= 5.3 mm from the screw keep-outs.
  Height 10.0 max is under the 12 mm limit. The datasheet has no L-vs-I
  curve; with Isat = 40 % drop at 18.7 A a soft metal-dust roll-off
  gives roughly 15-17 uH at 12 A (estimate, not data) vs ~17-18 uH for
  the XAL1510-223. Still open: the 55 V rating (48.5 V across it at
  OVLO plus SW ringing), immersion. APS1513 lands on the same pads
  (terminal gap 7.0, 0.2 mm off the inner pad edge) but is 13 mm tall.
- Ruled out: APS1513M220F (13.86 mOhm, 22 A, but 13 mm > 12 mm limit),
  Bourns SRP1513CA-220M (same, $7.34, 22 stock), COILMX MTF1810-220M
  (18.3 mm, does not fit between L1 and L2), 12-13 mm class (22-42 mOhm,
  Isat ~12 A).

Ceramic instead of the polymers (question only, not decided): ripple is
fine with ceramics (~10 mV), but a 6 A step at ~30 kHz crossover droops
~1 V instead of ~0.15 V, the polymer ESR damps the module-cable-node
ceramic LC, and the compensation would need retuning. Options: one
polymer plus 2-4 more 1210 22 uF (~$5 less, ~2x droop); a cheaper 2 mm
class polymer; aluminum polymer cans fail the height limit. Needs the
real load-step size on the 19 V bus to decide.

Controller: KEEP the LTC7810 (user decision 2026-10-27). Cheaper
controllers are all single-phase, which would be another board (new power
stage, one 13 A phase with paralleled FETs, more output caps, all routing
around U1 redone). There is no cheap 2-phase part at 100 V: LM5143 is 65 V,
LM5170 is a bidirectional current controller that needs an outer voltage
loop. The ADR 0003 rule (surges under 100 V, 100 V parts) also rules out
the 80 V LM5148/LM5149 ($1.05, deep stock). Candidates for a possible
single-phase volume revision, JLC 2026-10-27: LM5116 (100 V, HTSSOP-20,
C13755, $2.14, 20,659 stock), SCT82A30 (100 V, QFN-20, C6041259, $2.48,
1,911), LM5146 (100 V, QFN-20, C3188679, $1.96, 46). Saving ~$17 per
board; worth it only at volume, after the first boards show the real
thermals and load steps.

Consignment (explained to the user): buy parts elsewhere, register in
JLC Parts Manager, ship to Shenzhen with spares for attrition (~$20-50
courier), JLC stores them for PCBA. Alternative: JLC Global Sourcing.
With the APS1510 only the LTC7810 needs consigning.

Next steps (ask the user which):
1. Decide the inductor: overlay the APS1510 land pattern on the XAL1510
   footprint, read its L-vs-I curve at 12 A, check the 55 V rating
   margin; if OK, change L1/L2 LCSC field to C49261404.
2. Decide C24/C25 (keep two, one + ceramics, or a cheaper polymer) once
   the 19 V bus load step is known.
3. Replace or source U2/U3 (MCP65R46 out at LCSC) and C13 (C77025).
4. Board finishing: silk + connlabels, review the KRT routes (VIN_IC /
   DRVCC->D1 loop right of U1, comparator cluster), then jlcfab and get
   real JLC PCB + PCBA quotes to replace the estimates above.

## Start here (2026-10-27): PM-19V-250W routed, 0 opens, awaiting review

Found and fixed (user chose both fixes):
- Phase 2's output current ran L2 pad 2 -> SNS2P F.Cu strip along the top
  edge -> R10 at the output end, ~30 mm, necked to 1.5 mm by a redundant
  1.6 mm VIN trace KRT had added on F (it ignores the In1 VIN pour), and
  U1's SENSE2+ was tapped at L2 pad 2, so the strip drop (~6 mOhm, ~80 mV
  at 12.5 A) sat inside the 67 mV sense loop. Now R10 sits upright left of
  L2 pad 2 (pad 1 bridged to L2 pad 2 by a short SNS2P pour), pad 2 feeds
  a VOUT riser and the VOUT strip along the top edge; the strip drop is
  outside the sense loop (still ~4 mOhm / ~0.6 W at full load: a candidate
  for doubling on another layer). No VIN trace on F any more: /VIN is no
  longer a KRT power net (pcbkit.toml).
- Input end ripped up and routed by hand (tools/route_input.py).

Layout now (tools/place.py SMALL table, tools/power.py pours):
- Kelvin taps from the inner pad edges, under the resistor bodies: SENSE2+/-
  from R10 (short, F then B to pins 13/12); SENSE1+/- from R9: pins 1/48
  out under U1's body, F under L2, In1 (SENSE1- in a band between TG1 and
  the Q2 GND vias; SENSE1+ along the bottom edge), F under R9's body from
  vias right of C6/C8 (B under R9 is C5-C8). BG1's via moved to (123.6,
  119.6) to open that In1 band (tools/route_drive.py).
- U1's compensation network on F under U1's left side (VFB/ITH/SS lanes
  from pins 47/46/45 on B to staggered vias); C14 on B in the corner.
  EXTVCC and R6/C12 take VOUT from R10 pad 2; C16/C17 over pins 15/16;
  INTVCC (pins 14, 8, 3) under U1's body.
- RUN and NDRV cross under U1 on In1 (RUN above, NDRV below, the order of
  their pins); RUN to J2.4 via F under L2 and In1 under the VIN strip
  (In1 VIN strip bottom edge now y 104.2); R1's VIN feed on In1.
- Comparators U2/U3 moved to the output end in R10's old spot (F, a hole
  in the VOUT pour), dividers and VAUX zener supply on B right of J2.
  OV_KILL/PGOOD/U2-/U3+/VAUX there are KRT-routed (hand attempts ran out
  of room; KRT's result is short and 45-degree, review it).
- Flow: mkboard -> place.py -> setrules -> power.py -> handroute
  route_drive.py --save -> handroute route_input.py --save -> group_hand.py
  (now groups every non-power track) -> krt-route -> handroute
  route_post.py --save (KRT rips ITH1/C13 despite keep_groups; this puts
  them back) -> gndpour -> silk -> tools/silk_labels.py (in place: J1 ref
  hidden, screw nets VIN/GND/GND/VOUT on both sides, J2 ref on B by its
  pads; no room for VOUT on B; no connlabels, it would put J2 on F).
DRC: 0 unconnected, 0 clearance/short; 17 starved thermals (J1 screw via
rings, R7.2, J2.6: connected anyway), silk clean (2026-10-27: 0 overlap,
0 over copper; D1's own outline clipped at the bottom edge, J2's B.Fab
"1" not mirrored), 2 dangling VIN stepping-stone vias at y 107.7 (B backbone
helpers), gndpour placed 0 stitching vias (board too dense for its rules).
Review asks: KRT's VIN_IC / DRVCC->D1 loop right of U1; output-end
supervision routes; the VOUT strip loss.

## Earlier (2026-10-26 night): PM-19V-250W layout in progress

ADR 0003 and docs/pm-interface.md Accepted. Module work in the new
project power/pm-19v-250w/ (pm-19v-200w kept a few days, then decide).
Done, committed:
- Schematic: J1 screw pads (pm:PM_Mount_M2, 1 VIN 2/3 GND 4 VOUT),
  J2 2x3 aux (1 OV_KILL 2 PGOOD 3 ID 4 RUN 5/6 GND), TH1 gone, R16 22k,
  OVLO R4/R5 13.7k/18.2k (38.5 / 67.5 V), R9/R10 7 mOhm (C20608751),
  R8 13k, L1/L2 Coilcraft XAL1510-223 (consign), C24/C25 KEMET T523H
  100u 35V 7360-20 (C2158991) on B. ERC = the 200 W baseline (5 known).
- power/tools/pmmount.py regenerates the mount footprints from the spec
  (polarised: S2/S3 2 mm inward; 180 degrees turned, no screw lines up).
- Board rebuilt from the netlist: tools/place.py (new tables), tools/
  power.py (new pours: In1 VIN feed from S1, GND islands with vias under
  the input caps, VOUT round a GND island to S4). Flow: mkboard ->
  place.py -> setrules -> power.py -> krt-route -> gndpour -> silk.
- U1 area reorganised for hand routing (a80808b): U1 small passives
  0402, VFB/ITH1 parts above pins 45-47, corridor above drive pins
  37-42 and a TG1 lane free (In1 vias go under L1's body, x 115.4-122.2,
  the only F-free spot there), C18/C19 beside SW1/SW2. KRT: 22/32, 14
  opens, all at U1 pins (TG1 TG2 BOOST1 BOOST2 DRVCC INTVCC RUN OVLO
  VIN_IC VFB): the user routes these by hand in KiCad. Then: gndpour,
  silk, connlabels, DRC, jlcfab.
- 2026-10-26 night (5e39037): user asked Claude to route. U1 turned to
  rot 0 and moved to x 114.45; the phase pairs swapped in placement
  only (no schematic change): top pair Q3/Q4/L2/R10 = phase 2, bottom
  pair Q1/Q2/L1/R9 = phase 1, so each drive group faces its FETs;
  power.py zone/via net names swapped to match. Gate drive is hand
  routed by tools/route_drive.py (pcbkit handroute), kept as group
  "hand" ([krt] keep_groups) via tools/group_hand.py. Flow now: mkboard
  -> place.py -> setrules -> power.py -> handroute route_drive.py
  --save -> group_hand.py -> krt-route. KRT leaves 14 opens, all at the
  input end (VOUT, OVLO, ITH1, INTVCC, DRVCC/NDRV pin 10, U2-, U3+, GND
  taps). Supervision passives and U1 small-signal passives are 0402.
  KRT's F.Cu tracks under the inductors and long top-side runs still
  need a review pass.
- Gotcha: a backgrounded `cd dir && pcbkit krt-route` ran in the session
  cwd and routed granite_controller in ~/.cache (stopped at the diff
  step, the board was not copied back). Run krt-route in the foreground
  with PCBKIT_CONFIG=$PWD/pcbkit.toml. `krt-route -h` starts a real run.
Open: LTC7810 typ/max current sense threshold (assumed 67/75/83 mV; only
67 known) - analog.com did not load. Stock of C22793 / C22892 / C31850
not seen (API without stock). Routing result: see below / next block.

## Earlier (2026-10-26 late): documents written, review them

Open questions 1-8 below were answered by the user and steps 1-3 of
the plan are drafted:
- docs/plane-requirements.md (R1-R22)
- docs/adr/0003-plane-tile-and-module-interface.md (Proposed; 4
  review asks at the end of its Implementation Plan: names PP-48-4T /
  PM-19V-250W, tile LM5069-1 vs -2, module edited in place or copied,
  INT wired-OR vs per column)
- docs/pm-interface.md (screw positions and nets, polarised pattern,
  pads, 2x3 aux map, ID table, heights, what the module re-placement
  must clear)
abb9f50 is reverted (797cb1e); pmmount.py lives at tag pm-mount-drawn.

Answers: 1 revert; 2 no DOSA fallback, module free; 3 tile 1x4; 4
node connector split (power 2x3 / HCS, front panel separate); 5 no
1-Wire on the link; 6 aux 2x3, spare = second GND; 7 protection chain
confirmed, reverse polarity by keyed connector only; 8 module 250 W.

Next: user reviews the three docs -> ADR Accepted -> step 4 (module
update, slot column, core column, tile).

## Earlier (2026-10-26 evening): decide on the mount commit first

The last commit, abb9f50 "screwed PM module mount footprints", was drawn
before the decisions it depends on (screw positions, aux map, plane
organisation, 48-60 V input), all of which changed in the discussion
after it. Its boards are NOT clean: plane 245 DRC / 18 unconnected,
carrier 15 / 6, module 142 / 6 (details in the 2026-10-26 mount block
further down). The user has not decided whether to keep it.

Revert point, local tags (not pushed):
  pm-mount-before = 6ce63eb  (boards routed and clean, decision 4 doc in)
  pm-mount-drawn  = abb9f50  (the drawn mount)
To drop it: `git revert abb9f50` (keeps history), or, while master is
not pushed, `git reset --hard pm-mount-before` (ask first).
master is 5 commits ahead of origin, nothing pushed today.

Decisions made in the discussion (2026-10-26, not yet in an ADR):
- Mount: screwed module, M2 into SMT standoffs on the host, pogo pins on
  the host for the aux signals pressing on flat pads under the module.
  JLC places one part per designator: one part per standoff / pogo pin.
  Parts found: standoff Sinhoo SMTSO2060CTJ C2915628 (M2 x 6, 5.56 body,
  3.6 pilot 1.53 long); pogo Xinyangze YZ130115073P-01 C5157258 (2 mm
  base, 7.3 free / 5.5 compressed per the listing, no drawing: sample).
- The module is no longer DOSA in anything but its 58.4 x 22.9 outline
  (no pins, own aux block, non-isolated). Screw positions are free and
  should follow the module layout, not the DOSA corners (the DOSA pin 3
  spot sits in the gate drive; no clear spot at the input end exists
  with the current placement). Write a module interface spec (outline,
  screws and nets, pads, aux block and map, 6 mm gap, part heights both
  sides) and generate all footprints from it.
- Input 48-60 V: OVLO to ~66-68 V on module (R3-R5) and plane (LM5069
  divider), plane trips first. Keep the 100 V parts (bus surge clamp
  must keep transients under 100 V). Recheck slot clamp D1 SMBJ60A
  (60 V standoff = top of range).
- Plane: vertical modules, one slot per column, all columns identical
  (route one, copy: KiCad 10 multichannel repeat layout or a pcbkit
  script). VBUS bus along the top, node connector at the bottom edge,
  core (MCU, bucks, link, I2C mux) as its own column. "Up" is as drawn.
  4-slot tile; 8 slots = two tiles, each with its own MCU.
- Column to core interface: VBUS, GND, 3V3, SDA_n/SCL_n (TCA9548A mux
  in the core, same addresses in every column), INT_n (open drain), ID_n.
  Per column an I2C GPIO expander (TCA9535 class) for the slot control
  and status lines; OV latch stays local hardware.
- ID stays analog (option "a0"): resistor on the module, 10k pull-up and
  100n in the column, trace to an MCU ADC pin (1k series). Open = no
  module.
- TEMP dropped (submerged modules sit at fluid temperature); per slot
  the INA238 die temperature is free. Fluid temperature: the controller's
  three 1-Wire connectors (maybe a DS18B20 per tile via the link cable).
- SDA/SCL dropped from the aux. Aux shrinks to 5 contacts:
  OV_KILL, PGOOD, ID, GND, RUN.
- OV_KILL threshold 22.7 V: probably OK, depends on the nodes.
- Energy per node, if wanted later: INA228 (energy accumulator) is
  believed to be pin-compatible with the INA238; check before use.

Open questions, for the user:
1. Keep or revert abb9f50 (tags above).
2. Off-the-shelf fallback: does it matter that a commercial eighth
   brick (e.g. 24 V trimmed to 19.2 V, needs a TRIM footprint) could be
   soldered into the plane? If yes: keep DOSA pin holes on the plane and
   put the screws / pogos of our module where they avoid them. If no:
   the module is free.
3. Tile shape: 1x4 (~155 x 130 mm, all columns same orientation, even
   cooling, connectors on one edge; recommended) or 2x2 (~100 x 220 mm,
   top row in the bottom row's warm fluid). Depends on the pod envelope
   and how planes sit in the box (6x8 or 8x8 slots per box).
4. Node connectors (now Molex Mini-Fit Jr 2x4 39-28-1083: 2x +19 V,
   2x GND, PWR_SW, RST_SW, PLED+/-): cable or direct mating, power and
   front-panel signals in one connector or split, latching / mating
   cycles, which edge, and the immersion fluid (connector plastics and
   plating compatibility).
5. Link cable: does it carry a 1-Wire line for a per-tile DS18B20?
6. Aux block shape for 5 contacts (1x5 or 2x3) and where on the module.

7. Protection without fuses (fuses are rated in air: cooled in fluid
   they blow late or not at all, arc in the fluid, and cannot be
   replaced in the tank). Proposed chain, confirm:
   | Box supply / branch | breaker or fuse OUTSIDE the fluid | by hand |
   | Tile input | LM5069 + INA238 (or I2C hot-swap LM5066I / LTC4282, check stock) | software / auto-retry |
   | Slot input | LM5069 + INA238, OV latch | software |
   | Module output | buck current limit; PGOOD-loss -> slot latch (with start-up blanking); INA238 ALERT above ~210-220 W -> latch | software |
   - Plane F1 (tile input fuse) -> tile hot-swap: shunt ~1.5 mOhm for a
     30 A class limit, 2 FETs in parallel; reset by pulling UVLO low
     (expander + FET, as on the slots), LM5069-1 latch-off or -2 retry.
   - The core (MCU, expander, mux, link, 3V3 buck; 12 V aux buck
     preferably too) must be fed from BEFORE the tile hot-swap, or a
     tripped tile kills the MCU that has to reset it.
   - Plane F2 (12 V aux 1.5 A fuse) -> drop; LM5164 (1 A) limits on its
     own (check: hiccup or continuous limit); AUX_SNS shows a short.
   - Reverse polarity was F1 + D1; LM5069 does not block reverse: keyed
     input connector, or an ideal-diode controller + FET (~25 A path).
   - Optional upgrade: eFuse on the module's 19 V output (~15 A, for
     ~20-24 V; part not chosen) for an exact per-node limit.
8. Module power class: nodes (Halo Gorgon) peak ~180 W = 9.5 A at 19 V,
   the 200 W module has ~10% headroom. Proposal: size the module for
   ~250 W (same LTC7810 two-phase, ~6.6 A per phase, current limit
   ~18-20 A; recheck FETs, inductors, sense resistors, compensation at
   full load). The module current limit cannot be lowered (boost peaks).
   Plane at 250 W per slot:
   - Slot input 5.5 A at 48 V (~7 A at 38 V UVLO): shunt, LM5069
     (15.7 A limit), BSC070N10NS5 (~0.35 W), INA238 (~25 mV of 41 mV):
     all fine as they are.
   - Slot output ~13 A, up to 18-20 A in a fault until the latch acts:
     node connector needs 3 contacts per polarity (Mini-Fit 2x5 / 2x6)
     or Mini-Fit HCS terminals; cable 14 AWG (or 3x 16 AWG per
     polarity). NODE copper: in the column layout VOUT screw to
     connector is only 15-25 mm: wide pours on both outer layers, 15-20
     stitching vias, solid (no thermal spoke) connections on standoff
     and connector pads; 2 oz not needed for NODE.
   - Tile VBUS: 4 x ~265 W = ~22 A at 48 V, ~28 A at 38 V (box 13-16 kW
     at 12-16 tiles): 30 A class tile input connector, bus bar or 2 oz
     copper for VBUS along the top (that is where heavy copper matters),
     bus TVS still keeping surges under 100 V.
   - Standoff joints: a clamped M2 joint is < 1 mOhm (~0.2 W at 20 A);
     use lock washers so the clamp does not relax.

PLAN FOR NEXT SESSION (agreed 2026-10-26): the plane gets a full
redesign, the module an update. Nothing is ordered. Stop drawing; write
the requirements down first (today's churn came from finding them
during layout: mount -> aux -> screw positions -> module interface ->
columns / tile -> I2C -> protection -> power class).

Carries over unchanged (rearranged only): slot front end (LM5069,
shunts, pass FET, INA238, OV latch, enables; already 15 A class, covers
250 W), the LM5164 bucks, node front-panel interface (photoMOS, opto),
G070 MCU and pinout work, the JLC part numbers / stock checks, and the
tooling (gensch.py slot block per column, relink, syncfields, DRC
pipeline, pcbkit).
Redesigned: board organisation (vertical columns, 4-slot tile, core
column), control interface (I2C mux + expander per column), protection
(no fuses, tile hot-swap, core fed ahead of it), power distribution
(30 A VBUS, NODE copper, connectors), module mount (screws, pogo,
interface spec), placement and routing (one column, copied).
Module: update only (OVLO, drop TEMP / SDA SCL, aux block, screw
positions, maybe 250 W sizing); the LTC7810 core stays.

Steps, documents first:
1. Requirements, one page: node power (180 W peak, Halo Gorgon), input
   48-60 V, slots per tile and per box (6x8 / 8x8), immersion
   constraints (no fuses, no wet electrolytics, connector materials vs
   fluid), what must work with the MCU dead, serviceability (quick
   swap), box-level protection (breaker outside the fluid).
2. ADR for the plane tile and module interface: the decisions above
   plus the answers to open questions 1-8.
3. Module interface spec: outline, screw positions and nets, pad sizes,
   aux block and map, 6 mm gap, part heights both sides. All footprints
   get generated from it (power/tools/pmmount.py at tag pm-mount-drawn
   is a starting point if kept).
4. Then design, in order: module update -> slot column -> core column
   -> tile (route one column, copy).
Most of the content for 1-3 is in this section already.

## Earlier: start here (written 2026-10-25 for the next session)

State: PP-48-2S (power/pp-48-2s) is schematic-complete, routed, DRC 0
errors (5 warnings: 4 starved_thermal at R107.2 U201.5 U102.1 C203.2,
1 silk_edge at J1), ERC 0, board fields in sync with the schematic
(drc --schematic-parity 0 mismatches). Nothing is ordered.
granite_controller master was pushed 2026-10-26 (in sync with origin).
~/Electronics/pcbkit main is in sync with origin
(github.com/delandtj/pcbkit). Push only when asked.

The pipeline, in order, whenever the schematic changes (all from
power/pp-48-2s/):
  python3 tools/gensch.py                         (root + sheets/*)
  kicad-cli sch erc --severity-all pp-48-2s.kicad_sch
  cp pp-48-2s.xml <scratch>/old.xml
  kicad-cli sch export netlist --format kicadxml -o pp-48-2s.xml pp-48-2s.kicad_sch
  python3 tools/schcheck.py <scratch>/old.xml pp-48-2s.xml --map <scratch>/map.txt
  pcbkit relink pp-48-2s.xml pp-48-2s.kicad_pcb --save   (--allow-shorts
      only when pins move nets inside a part; then cut the old copper
      first with a handroute plan, see tools/handroute/81-*.py)
  python3 tools/syncfields.py pp-48-2s.xml pp-48-2s.kicad_pcb
  kicad-cli pcb drc --severity-all --schematic-parity --format report -o <scratch>/drc.rpt pp-48-2s.kicad_pcb
The .kicad_sch files are generated output: edit tools/sch/blocks/*.py
or tools/gensch.py, never the sheets. If KiCad has the project open,
reload before saving from the GUI (the board and sheets changed on
disk several times on 2026-10-24).

2026-10-26: pp-48-2s.kicad_pro (solder_mask_to_copper_clearance 0.005,
STEP export path) and fabrication-toolkit-options.json are committed.
power/*/*.step and power/*/production/ are now gitignored (regenerable;
the production/ export on the laptop was made BEFORE the field sync, so
its bom.csv has old values and 92 empty LCSC cells: regenerate it, or
use pcbkit jlcfab).

2026-10-26, JLC stock check (live JLCPCB search; the lookup script was
a scratch file, the API is POST
https://jlcpcb.com/api/overseas-pcb-order/v1/shoppingCart/smtGood/selectSmtComponentList
with {"keyword", "currentPage", "pageSize"}):
- U3 changed from STM32G0B1CBT6 (0 in stock, every LQFP-48 G0B1 at 0
  or <20) to STM32G070CBT6 (C529338, 2041). Pinout checked against the
  KiCad symbols and embassy's stm32-metapac data: identical for every
  pin in use (I2C1 PB6/PB7, I2C2 PB10/PB11, ADC PA0-7/PB0-1, HSE
  PF0/PF1, SWD), pin 10 is plain NRST (was PF2-NRST). Pins 29 and 32
  are not bonded on the G070, so S7_OFF / S7_LCLR (8-slot plane only)
  moved to pins 37 (PA15) and 48 (PB9). Lost: FDCAN. ADR 0001 and
  power-board.md updated. 2-slot board: only U3's value and LCSC
  changed, DRC as above.
- Still a problem: INA238 (C2868250) 0 in stock (plan was consigned /
  Mouser); INA237AIDGSR C2864837 is pin-compatible, 50 in stock, 10x
  the offset. L2 SRR1260-680M 0 in stock: swapped to Sumida
  CDRH127/LDNP-680MC C2452974 (5987; 92 mOhm typ, 2.6 A rated).
  Datasheet land pattern 2.9 x 5.4 pads, 7.0 gap, 12.8 outside = the
  L_12x12mm_H6mm pads exactly (centres +/-4.95), kept that footprint.
  It is 8.0 mm tall (SRR1260: 6.0); nothing above it on the plane (L2
  is clear of the X101/X201 brick outlines).
  Thin: L1 SRR1260-330M 95, C338133 615 (6 per board), MCP23017 812.
- The 58 parts without LCSC: all passive values stocked (mostly
  extended); F2 one-shot 046801.5NRHF C151143; Q101/Q201
  BSC070N10NS5 C534364 (19567); J101/J201 Molex 39281083 C293503
  (listing says right angle, check); J1 Phoenix 1715721 only 6 in
  stock, MKDS 1,5/2-5,08 C5183929 (956) is the stocked one.
- PM-19V-200W LCSC pass (fields in the hand-drawn schematic, synced to
  the board with pp-48-2s/tools/syncfields.py; ERC/DRC unchanged, parity
  0): 52 of 58 parts. Flagged: U1 LTC7810ELXE C581067 only 13 at JLC
  (~$31; consign from ADI/DigiKey); U2/U3 MCP65R46T-1202E/CHY C116625 0
  in stock (the -2402 2.4 V ref variant has 90, but the dividers assume
  1.21 V); C20-C23 GMK325BJ226MM-P C778721 is X5R (no 22u 35V X7R 1210
  at JLC). Left empty, decisions: L1/L2 XAL1010-223 not at JLC (no
  22 uH >= 12 A in 10x10 there either: consign Coilcraft, or SRP1265A-220M
  C2041465 13.5x12.5 = new footprint); C24/C25 100u 35V polymer: no D
  case (7343-31) part, KEMET T523H107M035APE070 C2158991 is H case
  (7360-20, new footprint); J1/J2 pins hand-fitted.
- Brick pins (DigiKey research 2026-10-26): module small pins Mill-Max
  3136-3-00-80-00-00-08-0 (tin) / -15- (gold): .040" body .145" below a
  .078" flange, .180" tail, wants a .044" (1.12 mm) hole, our J1 drills
  1.3 / 1.9 are ~0.2 mm loose. No stock .062" pin with a .100" shoulder
  and .145" body found (4357-0-00-80-00-00-03-0 has .084" below the
  flange). Carrier receptacles: 0331-0-15-15-18-27-10-0 (DK ED90084-ND,
  ~$0.44 @100, 32k stock, flange 2.29, height 3.81) and
  9372-0-15-15-23-27-10-0 (DK ED1048-ND, ~$0.80 @100, 80k stock, accepts
  1.14-1.65 mm, flange 3.0, height 3.05). J2 1.27 mm: no long-tail
  header found (Mill-Max 850 tails 2.9-3.1 mm).
- kicad-cli pcb drc --schematic-parity needs an absolute board path,
  or the schematic load asserts and parity is skipped silently.

NEXT, in the order I would take them:
1. Parts: 58 of 174 footprints still have no LCSC number (the CA
   caps, the brick socket pins, Molex, photoMOS alternatives...).
   `grep -c` the xml or run jlcfab and read its missing list.
2. LM5164 ripple values: 3V3 CA 1.5n -> 1.8n or 2.2n (under the Eq 24
   minimum), CB 100n X7R vs TI's C0G 56-330 pF design. Table in
   tools/sch/blocks/bucks.py, then the pipeline above (values only,
   no relink needed, syncfields is enough).
3. 15 A class check of the slot front end: shunt (two 7m 2512 in
   parallel = 3.5m, 15.7 A limit), pass FET SOA against the LM5069
   power limit, docs/power-board.md "Per slot, numbers for the 15 A
   class".
4. pcbkit jlcfab -> BOM / CPL, compare with production/.
5. Module side (power/pm-19v-200w): the J2 aux header tails must
   reach the plane across the brick pin shoulder standoff (long-tail
   1.27 mm pins or a taller socket on the plane); pick the brick pin
   part number. docs/power-board.md "Mounting on the carrier".
6. 8-slot plane: the MCU pin table has ADC inputs for four slots
   only (S5-S8 ID / TEMP dangle); needed before gensch.py 8 is real.
7. Cosmetic: three wire crossings on the slot sheet, four on the
   link sheet.

Question that came up at the end of 2026-10-24 and may come back:
"what are the pins on the DOSA module place". Answer: the module PCB
has 13 holes (J1 = 8 DOSA brick pins, J2 = 5 aux pins of our own),
shouldered Mill-Max style pins are soldered into them, and the plane's
X1 (8 holes) and X2 (7 holes, 2 reserved for I2C) take those pins;
X1 and X2 are turned 180 together on the plane, geometry checked
against the module board on 2026-10-24. Plane nets: X1.1 OUT, X1.2
RUN NC, X1.3 X1.4 GND, X1.5-7 NC (no remote sense on the module),
X1.8 NODE; X2 OVK PG ID TEMP GND SDA2 SCL2.

## Session (2026-10-24, evening): schematic generator rewrite

ADR 0002 built and committed: tools/gensch.py now writes a root sheet
plus sheets/{input,bucks,mcu,link,expanders,slot}.kicad_sch, drawn
blocks with wires, power symbols at the pins, hierarchical labels for
the nets that leave a sheet; the slot is one file instantiated N
times. Engine in tools/sch/ (kicad.py emitters, draw.py Block with
wire / port / power / lint), blocks in tools/sch/blocks/, checker
tools/schcheck.py, `pcbkit relink` in ~/Electronics/pcbkit (committed
there, not pushed). ERC 0 with --severity-all. schcheck: every (ref,
pin) -> net identical to the old flat netlist; 71 nets renamed (sheet
path names, e.g. /S1_OUT -> /Slot 1/OUT); 4 expected field
differences (J101/J201 Value "NODE", INA238 description). The routed
board was relinked in place: 174 paths, 71 nets, DRC unchanged (0
unconnected, the 4 starved_thermal + J1 silk_edge). power.py and the
handroute plans carry the new names. Regenerate twice = byte
identical. N=8 generates and passes ERC except S5..S8 ID/TEMP (the
MCU pin table has ADC inputs for four slots; pre-existing).
Built by four opus subagents (engine, checker, relink, blocks) under
the coordinator; the ADR's "Built as" section lists the deviations.
The block authors found things in the pin tables that looked wrong.
Items 1-3 below are FIXED (tables, schematic, board; commit "four
fixes"), 4 was checked and left, 5 is open:
1. Every LED is reversed: Device:LED pin 1 = K, pin 2 = A, but D2 D3
   D4 D102 D103 D202 D203 have the resistor node on pin 1 and GND on
   pin 2 (anode to GND). None will light. Fix = swap the two nets per
   LED in the tables; on the board the LED footprints then need a
   180 degree turn (pad 1 / pad 2 swap places, the tracks stay).
2. LM5164 type 3 ripple injection: tables have CA from INJ to FB and
   CB (100n) from FB to VOUT. The datasheet figure has RA SW -> node,
   CA node -> VOUT, CB node -> FB. As tabled the 100n shunts FB to
   VOUT. Check the datasheet figure; if confirmed, swap the far ends
   of C5/C6 and C12/C13 and rework those four caps on the board.
3. Slot Q4 (MCU latch clear driver) is dead: its gate LCLR_D has only
   the pull-down; the MCU's Sn_LCLR goes straight to the latch /CLR
   node (and Q4's drain). Either the MCU should drive LCLR_D, or Q4 /
   R12 go. On the board S1_LCLR runs MCU -> latch today.
4. Brick X1 SENSE+ / SENSE- are NC; many DOSA bricks want remote
   sense tied to the output. Check PM-19V-200W.
5. The 8-slot plane has no MCU ADC pins for slots 5-8 ID / TEMP.
Fix status: 1 done (nets swapped, LEDnn_K -> LEDnn_A, the seven
footprints turned 180 degrees, plan 80). 2 done: TI SNVSAU4D Table 6-1
/ Figure 7-1 confirm RA SW -> node, CA node -> VOUT, CB node -> FB;
C5/C6 and C12/C13 swapped places on the board and rewired (plans 81,
82). 3 done: Sn_LCLR now ends on the Q104/Q204 gate + R112/R212, the
latch node is /Slot n/CLRN (plans 81, 83). 4 checked: the PM-19V-200W
module leaves brick pins 5-7 unconnected (its schematic and
docs/power-board.md line 592), so X1.5 / X1.7 stay NC. DRC after all
of it: the same 5 warnings, 0 unconnected; schcheck: schematic and
pp-48-2s.xml equivalent; identity relink: no change.
Value notes from the LM5164 datasheet check (not changed): the 3V3
buck's CA 1.5n is 5 % under the Eq 24 minimum of 1.58n (1.8n or 2.2n
would do; ripple at FB about 22 mV); CB 100n X7R meets the ">=" but
TI designs CB for ~75 us settling with C0G (56 pF at 12 V, ~330 pF at
3V3): revisit CB. CA parts have no LCSC number yet.
tools/syncfields.py ported to the KiCad 10 API (HasField / GetField /
SetField by name; GetFieldByName is gone) and run: 108 Value / LCSC /
Description fields on 174 footprints synced from pp-48-2s.xml, fields
hidden; kicad-cli drc --schematic-parity now reports 0 mismatches.
Run it after every relink (netlist, then relink, then syncfields).
Also open: the slot sheet keeps three unavoidable wire crossings
(OUT to the INA238 over SENSE / GATE), and the link sheet four
(J2 -> J3 chain). Cosmetic.
NEXT: CA / CB values, the LCSC numbers (58
of 174), MOSFET / shunt sizing for 15 A, jlcfab.

## Session 2026-10-24 (afternoon): J2/J3 move, silk, connector labels: PP-48-2S routed, DRC 0 errors

The PC powered off at the end of the 2026-10-23 session (krt-reroute's
GND pass sat 3 h in a "KiCad-oracle" leg before that). Nothing on disk
was lost: the scripts write straight to the project. What was lost were
the pre-route board copies and DRC reports in the session scratchpad
(/tmp), hence the rule: keep board checkpoints under ~/.cache/pcbkit or
commit WIP boards before long router runs. Committed as 1311ae7 (WIP
checkpoint: placed, power copper, gndpour, one KRT pass).

Then the 4 opens KRT left and the 2 clearance hits were closed by hand
with `pcbkit handroute`; the plans are in power/pp-48-2s/tools/handroute/
(run in number order after krt-route + power.py + gndpour, see the flow
comment in pcbkit.toml):
- 10: +3V3 to C203.1 (KRT's GND F.Cu chain around C203 removed, the pour
  takes it; via beside C203 down to the +3V3 PWR trunk at y 162.8).
- 20: /FB2 to C13.1: the +12V tap via sat between C12.2 and C13.1; moved
  to the right of C13.2 (B.Cu link), FB2 is a straight F.Cu track.
- 30: /S1_LQ to U103.5: KRT's GND tracks under U103 removed, via between
  the pad rows, PWR west to the existing S1_LQ PWR run at x 133.8.
- 40: S1_PGD diagonal shifted 0.1 mm east of the S1_OUT corner (0.149 ->
  0.22 mm).
- 50: /S1_ID to U3.11 (MCU): OSC_OUT's via moved up 0.5 mm, S1_ID via at
  the pad's inner end under the LQFP body (0.12 mm from the pad, tent
  it), PWR down x 226.1 and west along the empty y 133.3 lane to the
  X102.3 via. NOTE: the via sits under the MCU body; fine for JLC, but
  check the mask on the fab preview.
- 60: `pcbkit prune` ate the +3V3 pad stubs of U103.8 and U203.8 (far
  end free, junction mid-track); 60 puts them back. Run 60 after prune.
Silk and connlabels run (silk: 52 references placed, 122 hidden; labels
for the 9 connectors, pin legends on J1, J4, X102, X202). pcbkit
connlabels was fixed for this board (commit in ~/Electronics/pcbkit,
not pushed): connectors with the pins along y (J1 left edge, J2-J4
right edge) got all legends on one spot; outline rectangles (the brick
socket) blocked everything inside them.
J2 / J3 moved 1.7 mm inboard (and 0.5 / 0.6 mm up, courtyards: J5 + H2
above J2, C26 below J3) so their pin legends fit between the body and
the board edge (plan 70-j2-j3-left.py; a 1 mm move was not enough: INT
is 2.1 mm wide and connlabels keeps 0.3 mm to the edge; moving them
towards the isolators only shrinks the gap the legends wanted). The
pads landed on the F.Cu corridor between the isolators and the old pad
column, so CTRL_INT / CTRL_SCL / CTRL_SDA / EXP_IRQ / LED_RUN / the
CTRL_GND via to C26 / the J3.1 feeder were rerouted by hand (comment
block in the plan). pcbkit handroute got a ('move', ref, (dx, dy)) op
for it. Legends now on all of J1-J4, X102, X202. J2 / J3 names are the
bare "J2" / "J3": "J2 CTRL IN" fits nowhere (1.8 mm between the two
bodies, J5 / H2 above J2, diagonals below J3).
State: DRC 0 unconnected, 0 clearance, 0 dangling, 0 silk overlap; 4
starved_thermal warnings (R107.2, U201.5, U102.1, C203.2: one pour
spoke, pad is connected) and 1 silk_edge_clearance (J1's own outline
at the left edge). NEXT: the LCSC numbers still missing (58 of 174),
MOSFET / shunt sizing for 15 A, jlcfab.

## Session 2026-10-23: power plane architecture (docs only)

Discussion only, no hardware or tools touched. Written into
docs/power-board.md, section "Power plane: PP-48-xS":
- The carrier board is the "power plane": one design in 2, 4 and 8
  slot sizes (6 = depopulated 8), 48 V only, one slot kind (eighth
  brick, 15 A plane-side front end). Mains / 220 V modules were
  discussed and dropped; inductive measurement discussed and rejected
  for DC slots (percent-of-full-scale error at 4 A), the front end's
  shunt meters every slot bus-side.
- One STM32 per plane (G0B1 candidate) as telemetry aggregator and
  command relay, one I2C address per plane (proposal 0x30-0x37), one
  register map for every plane type. SUPERSEDES the 2026-10-21 "no
  MCU on the power board" decision. Safety stays in hardware: enable
  pins pulled to ON, MCU pulls low for OFF, OV cut is a hardware
  latch, stagger by the MCU degrades to all-on. Rules listed in the
  doc.
- The controller is plane-type agnostic: enumerates planes, reads
  type / slot count / map version, exposes slots. Future plane types
  for larger loads, never a second slot kind on this plane.
- Decision 9 taken: J2 grows to 7 pins (SDA/SCL reserved, unpopulated
  on rev 0.1). The bench carrier's 5-pin X2 stays.
- Node control moves to the plane (decision 6): PWR/RST photoMOS and
  PLED sense per slot, one Mini-Fit Jr class cable per node from the
  slot; primitives and the power-cycle sequence on the plane MCU,
  policy on the controller. Next controller rev sheds the node
  section; rev C unchanged until then.
- Controller supply (decision 8 closed): 12 V always-on aux output on
  every plane, OR-ed from two planes when several; never from slot 1's
  brick (rejected: controller downstream of a switched slot). Rev C's
  J3 takes it as is.
- Front end part chosen (decision 7): LM5069-2 (switch, 55 mV current
  limit, power limit, auto-retry; LCSC C111822, $3.70 at 100, in
  stock) + INA238 (16-bit monitor, +/-5 uV, +/-0.1 %, 16 addresses;
  not at LCSC, Mouser stock) on one 3.5 mOhm shunt. Rejected: ADM1272
  (end of life), LTC4286/4287 (price, not at LCSC, 48-QFN), LM5066I
  (12-bit telemetry +/-3.5 to 6 %, misses the 2 % requirement, $8.58).
  INA228 is the drop-in upgrade when LCSC has stock. Datasheet
  extracts used for this are not in the repo (scratchpad only).
- ADR docs/adr/0001-48v-power-plane.md ACCEPTED 2026-10-23: TLV
  message format over I2C (CRC-8, global tags), one Mini-Fit Jr 2x4 node
  housing, STM32G0B1, primitives on the plane / policy on the
  controller. Next: power/pp-48/ KiCad project (core + slot sheets),
  2-slot board first.
- STARTED power/pp-48-2s/ (2-slot plane): tools/gensch.py generates the
  flat schematic (187 symbols, every net a label at the pin, power nets
  as global labels anchored to power symbols + PWR_FLAGs at the sheet
  bottom); ERC 0 violations; netlist exported; pcbkit mkboard/outline
  (150 x 80 mm, 4L)/setrules run. pp.kicad_sym = PM_Brick_Socket (from
  the carrier) + TLP176AM (from the controller); pp.pretty/
  Brick_Eighth_DOSA_Plane = socket pattern with 1.5 / 2.05 mm holes for
  soldered brick pins. Values: LM5164 3V3 (41.2k RON 200 kHz, 33 uH,
  86.6k/49.9k, type 3 ripple injection 453k/1.5n/100n) and 12 V (100k
  300 kHz, 68 uH, 453k/49.9k, 453k/2.2n/100n, TI example), EN/UVLO
  1M/52.3k = 30 V; LM5069: 2 x 7 mOhm 2512, UVLO 143k/10k (38 V), OVLO
  240k/10k (62 V), RPWR 13.7k (~50 W), CT 220n; OV latch 74LVC1G74
  (/PRE = OV_KILL, /CLR = 100k/100n RC + MCU 2N7002), 2N7002s on
  UVLO for OFF and latch; INA238 A1/A0 straps 0x40 + slot - 1; MCP23017
  per 2 slots (0x20+), GPA = odd slot, GPB = even slot, bit order PWRD
  RSTD PGD PG LQ LED; MCU pins in the generator's mcu_nets table.
  Pass FET BSC070N10NS5 as placeholder (SOA open). NEXT: tools/
  place.py (brick sockets, node connectors on one edge, bus terminal,
  MCU corner), tools/power.py pours (VBUS, S*_SENSE, S*_OUT, S*_NODE),
  route, LCSC numbers (INA238, LM5164, STM32G0B1CBT6, ISO1640, ISO7721,
  MCP23017, 74LVC1G74, TLP291, Mini-Fit, JST PH, crystal, inductors).
- Open: MOSFET and shunt for the LM5069 at 15 A, OV latch part, node
  connector and cable, aux buck and OR-ing, plane input/fuse, outline,
  ID table, STM32 part and bootloader, controller-side plane discovery.

## Session 2026-10-22: PM-19V-200W routed, tools reinstalled

Machine was rebooted after this session; nothing of the state below
lives outside git. Everything is pushed (granite_controller master
fe73f87 and later, pcbkit main).

- PM-19V-200W board (power/pm-19v-200w/): the 4 U1 opens hand-routed,
  gndpour run, VIN neck at J1.1 widened (pour polygon in tools/power.py
  and on the board), J1.3 F.Cu island stitched, silk tidied, via-in-pad
  check done. DRC: 0 unconnected, 0 clearance, 0 silk; only the 7 old
  via_dangling items remain. Details in the "State (2026-10-22)" bullet
  under the module section below. Next for the board: bench values
  (MOSFET, inductor, compensation), pin part and standoff for mounting,
  then `pcbkit jlcfab`. Later the same day: big "19 V" silk legend
  on both sides, and a 24 V variant recipe (both in the module section
  below).
- Carrier interface: docs/power-board.md got a "Mounting on the
  carrier" section (DOSA pins, carrier holes, soldered not socketed,
  J2 absent on a fallback brick). Pin 1 chirality of the DOSA footprint
  checked against the SynQor NQ90 and Artesyn drawings: correct.
  CORRECTION (later the same day): that section first said the carrier
  pattern is the module's J1 mirrored / on the B side. Wrong: module
  and carrier are both viewed from above, the host pattern is the same
  as the module's, unmirrored. Fixed in the doc.
- Bench carrier drawn: `power/pm-bench-carrier/` (schematic, board,
  pcbkit.toml, tools/place.py, tools/power.py, tools/legend.py,
  carrier.kicad_sym, carrier.pretty/Brick_Eighth_DOSA_Socket). 130 x
  55 mm 2L: DOSA socket with Mill-Max 0331 / 9372 solder-mount
  receptacles, 1.27 mm aux socket, screw terminals in/out, fuse, TVS,
  bulk caps, Kelvin test points at the socket pins, VIN/VOUT/OV/PGOOD
  LEDs, 5 V aux rail (10k 1W + zener), ID/TEMP pull-ups + test points,
  ON/OFF jumper, sense solder jumpers. ERC 0, DRC 0 errors (2 silk
  edge warnings from the terminal blocks), 0 unconnected. Details and
  open points (module J2 pin reach, no OV input cut-off) in
  docs/power-board.md "Bench carrier". The schematic was written by a
  generator script in the session scratchpad (not kept: the
  .kicad_sch is the source now; edit it in eeschema). Board flow is the
  comment at the top of its pcbkit.toml. Note the order: gndpour runs
  BEFORE krt-route (tools/power.py puts the GND zones in the kept
  group, KRT then skips GND as a plane net: no GND tracks), and again
  after it to refill and restitch. LCSC numbers added to the 20 SMD
  parts (schematic LCSC field and board footprint field), jlcfab run:
  BOM/CPL filled; D1, D2, Q1, F1 and the LEDs have no rotation entry in
  pcbkit's table, verify in the JLC preview. F1 is now 10 A (0451010.MRL,
  C44479; the 8 A nano2 is not at LCSC). Next: `pcbkit jlcfab`, LCSC
  numbers, order with the module.
- Tools: pcbkit and KRT v0.23.1 reinstalled on this machine (see
  "Tools and gotchas"); pcbkit gained a side-aware `silk`, and
  `handroute` / `view` (the scripted hand-routing loop and the
  all-layer region picture used this session). Bench board 1 USB
  problem (below) still not looked at.

## Session 2026-10-21: power expansion, first port module

Bench: board 1 stopped enumerating on USB at the start of the session
(-71 "device not accepting address" on hub port 1-1.2, also no ping on
192.168.64.115). Hub reset, port power toggle and xHCI rebind did not
help: it needs a physical replug / power cycle (user powering the host
off). Not looked at since; first thing to check next session. Console
without espflash: `python -m serial.tools.miniterm --raw --rts 0 --dtr 0
/dev/ttyACM0 115200`; MQTT log topic `granite/bench/granite-cb374b/log`.

Power expansion (docs/power-board.md, all decisions dated there):
- Link to the controller: galvanically isolated I2C on J8, isolator on
  the power board, board side on its own 3.3/5 V. No MCU on the power
  board. No power logging on the controller ever: it measures and
  publishes on MQTT, a backend (RRD or TSDB, open) stores.
- Direction (thinking stage): 48-54 V bus (design for 40-60 V in), board
  in the fluid, each port = common front end (48 V class hot-swap +
  metering, default ON) + per-load port module (own connector,
  hardware-fixed Vout and OV cut, self-ID). LTC4282 is out at 48 V
  (LM5066I / ADM1272 / LTC4286-7 candidates). 50-80 W conversion loss per
  frame is significant to the user; doc compares whole-chain losses.
- Production controllers move to 16 MB ESP32-C6 modules after testing.
- Cost per port (rough): 1-port module ~$30-35 at volume (LTC7810 ~$14
  of it, LCSC C581067 out of stock); 4/8-port boards ~30-40 % less per
  port by running each LTC7810 channel as its own single-phase port.
  Open decision: ports per module.

First port module PM-19V-200W, KiCad project `power/pm-19v-200w/`:
- Schematic (rev 0.1, wired, A3): LTC7810 two-phase 200 kHz/phase, 19.0 V
  (180k/10k), UVLO 38 / OVLO 62.7 V, gate drive 10 V, EXTVCC = VOUT,
  spread spectrum off; independent OV_KILL (>22.7 V) and PGOOD (>17.3 V)
  comparators (MCP65R46) on a 4.7 V zener rail from VIN. Values are first
  estimates (MOSFETs BSC070N10NS5, XAL1010 22 uH, 8 mOhm sense,
  compensation). Netlist checked against the intended nets.
- Board: 58.4 x 22.9 mm DOSA eighth-brick outline, 4 layers (In1 signal,
  In2 solid GND), pm.pretty/Brick_Eighth_DOSA (pin positions from the
  SynQor NQ90 drawing; pin-1 chirality checked 2026-10-22 against the SynQor NQ90 and Artesyn drawings: matches, see power-board.md).
  Scripted flow, all re-runnable: `pcbkit mkboard` -> tools/place.py ->
  `pcbkit setrules` -> tools/power.py (outer power pours, SW/GND/VIN via
  fields, backbone tracks inside the pours so KRT sees them as one node;
  group "power", kept by krt-route) -> `pcbkit krt-route` /
  `krt-reroute`.
- State (2026-10-22): the 4 U1 opens (OVLO, INTVCC, ITH1, TRACK/SS1)
  hand-routed with a scripted plan (pcbnew python: delete KRT segments
  by coordinate, add H/V/45 tracks and vias, refill zones, kicad-cli
  DRC on a scratch copy, loop until clean). DRC: 0 unconnected, 0
  clearance; only silk and the gndpour stitch "dangling" vias remain.
  None of the four closed by adding copper alone, so neighbours moved.
  Right column (pins 8-10): OVLO and DRVCC each got a via at the pad
  end (119.4, 116.95 / 117.75); OVLO continues on In1 to a via by R5,
  DRVCC links on F.Cu to the D2.2 via moved to (119.8, 116.3); BOOST2's
  via moved above D2.1 (121.05, 117.7) so INTVCC's pin-14 wraparound
  could shift east between the two new vias; pin 3 goes straight to
  C10.1 and the pins 4-7 GND tie ends in a new via (119.5, 114.95)
  under a TEMP/VAUX shift; SW2's F.Cu and SW1's In1 diagonals moved
  east. Top row: ID's via left the band above the pins (now 111.3,
  110.6, In1 at y 111.2); BOOST1 pin 40 exits inward to a via in the
  ring (112.68, 113.7), In1 at y 113.85; SW1 pin 41 takes the lower
  channel lane (113.85); BG1's via moved up to (119.3, 112.3), In1 at
  112.5; VFB1 takes the upper band lane into pin 47, ITH1 the lower
  lane west to a via (114.2, 110.6), In1 under the TG1 via, via on the
  R8-C14 track; TRACK/SS1 goes inward, west along the channel to a via
  (113.85, 113.25), In1 east at y 113.03, via under C15.1. VAUX got a
  second via (108.6, 110.0) for its north branch. `pcbkit gndpour` run
  after that (F.Cu/B.Cu GND pours, 7 stitch vias): DRC 0 unconnected,
  0 clearance; the 7 via_dangling left are old (5 VIN field vias at x
  136-140, BOOST1 at 106.0,110.1 and 124.5,110.9), plus one new
  starved_thermal on J1.3 (F.Cu pour makes an isolated island at the
  VIN neck). VIN neck at J1.1 widened the same day: the B.Cu VIN pour
  polygon (tools/power.py ZONES and the board) now dips to y 108.3 for
  x <= 108.6, so the pour wraps under C3.2 (about 1.35 mm, plus the
  leg beside D1/the DRVCC track) instead of the single 0.8 mm KRT
  track; J1.1's pour is no longer an island. The J1.3 starved_thermal
  (F.Cu GND pour island by pin 3, fenced by OVLO/RUN/BOOST1/BOOST2
  F.Cu tracks) got a GND stitch via (102.0, 118.6; not in the
  gnd-stitch group, so a gndpour re-run keeps it). Silk: `pcbkit silk`
  run (now side-aware, see pcbkit): 17 refs placed, the rest hidden
  (U1, Q1, Q3, C1-C5/C7, C20/C21: no pad-free spot; Fab keeps
  ${REFERENCE}); J1's 0.12 mm pin-label text widened to 0.15. DRC
  after all this: only the 7 old via_dangling (VIN field x 136-140,
  BOOST1 at 106.0,110.1 and 124.5,110.9). Via-in-pad check done: the
  only via in an SMD pad was KRT's SNS1P plane tap at (117.3, 111.5)
  inside L1.2 (with 2 mm stubs on F.Cu and In2); removed, the second
  SNS1P via at (117.9, 111.8) outside the pad already links L1.2 to
  U1 pin 48. No via-in-pad left. KRT relaxed min via drill 0.3 ->
  0.2 (fine for JLC 4L).
  Copper: ask 2 oz outer or 1 oz inner (10.5 A return on In2 0.5 oz).
- Silk (2026-10-22): a large bold "19 V" legend on both sides, so the
  rating is readable on a module out of its carrier. F.SilkS at
  (117.6, 120.0), size 4.2 x 3.2 mm, 0.5 mm stroke, in the strip under
  L1 between J2 and C28/R11 (L1's reference moved above its courtyard
  to 119.5, 104.5 to make room); B.SilkS at (144.3, 113.3), 3.6 x 2.8
  mm, 0.45 mm, mirrored, in the strip between the R14-R16 row and
  C5/C6. Both are board-level PCB_TEXT items (not footprint fields):
  `pcbkit silk` leaves them alone, a `pcbkit mkboard` from scratch
  loses them. Vias are tented, so silk over vias is fine; DRC after:
  still only the 7 old via_dangling. A voltage variant changes this
  text too.
- 24 V variant (PM-24V-200W), not started, recipe worked out
  2026-10-22. Same PCB, same footprints; the output voltage is fixed
  in hardware on purpose (no Trim; the DOSA trim convention is +/-10 %
  so 19 V could not reach 24 V anyway, and a runtime-adjustable Vout
  would decouple the independent OV cut from the output, which the
  port-module rule forbids). Parts that change:
  - R6/R7 (LTC7810 VFB, VOUT = 1 V x (1 + R6/R7)): 180k/10k ->
    232k/10k = 24.2 V (230k for 24.0 V exactly). C12 feed-forward
    across R6: 47p -> 39p (keeps the zero near 9 us).
  - R12/R13 (OV comparator, 1.21 V x (1 + R12/R13)): 178k/10k (22.7
    V) -> 226k/10k = 28.6 V. Forgetting this kills a healthy 24 V
    module.
  - R14/R15 (PGOOD): 133k/10k (17.3 V) -> 169k/10k = 21.7 V (90 %).
  - R16 (ID): a different value, so the controller sees a 24 V module
    (ID table still to define, power-board.md).
  - C20-C23 output ceramics: 35 V X7R 1210 -> 50 V parts (DC bias at
    24 V, and they must hold 28.6 V until OV_KILL acts). C24/C25
    polymers 35 V are at 69 % of rating: acceptable, 50 V if a part
    fits.
  - Silk "19 V" -> "24 V" on both sides, schematic title, project
    name.
  Unchanged and checked: duty 40 % at 60 V in, 63 % at the 38 V UVLO;
  EXTVCC at 24 V well inside the 65 V limit, but the EXTVCC LDO drops
  14 V instead of 9 V at the gate-drive current (about 0.5 W instead of
  0.3 W in the LTC7810, thermal check on the bench); inductor ripple
  about 3 A pk-pk per phase at 54 V with 4.2 A per phase at 200 W (22
  uH fine, 33 uH for the same ripple ratio); current limit stays about
  14 A DC = 340 W, so either change R9/R10 8 mOhm -> 12 mOhm (about
  9.5 A DC) to keep a 200 W class module or let the front end's power
  limit do it; compensation (R8, C13, C14) is a bench item either
  way. Bench carrier: nothing changes, set the load OVP for 24 V.
- Tried 2026-10-21 and dropped (683ed2a, reverted): U1 parts pinned in
  pin order, R3/R1/TH1 to the left edge: fresh route 6 opens, but TH1
  then sits away from the hot parts; with TH1 back near L1/U1 a fresh
  route gives 12 (TEMP crosses the controller). Kept from it: gate,
  boost and DRVCC router width 0.4 -> 0.3 mm in pcbkit.toml (0.4 cannot
  leave U1's 0.3 mm pads at 0.5 mm pitch with 0.2 clearance; KRT necks
  down only plane taps). Rerouting the 4 opens at 0.3 gives 5, so the
  board stays at its 0.4 routing.
- KiCadRoutingTools updated 2026-10-21 to v0.23.1 (was 5622 commits
  behind); prebuilt grid_router downloaded, deps refreshed, every flag
  pcbkit passes still exists. Its JSON_SUMMARY in the route log now
  names blockers per failed net ("blockers") and pads sealed by
  same_net_pad_clearance. 2026-10-22: this machine got a fresh setup
  (pcbkit cloned to ~/Electronics/pcbkit from github delandtj/pcbkit,
  `~/.local/bin/pcbkit` symlink to its bin/pcbkit; KRT v0.23.1 checkout
  at ~/Electronics/krt/KiCadRoutingTools, venv ~/Electronics/krt/venv
  with --system-site-packages for pcbnew 10.0.7, requirements.txt
  installed, grid_router built from source with CARGO_TARGET_DIR
  unset). The old stale-pip-shebang note no longer applies. The `gh`
  CLI token on this machine is rejected ("Bad credentials"); git over
  ssh works, so clone/push with git, or run `gh auth login`.

## Next session: rev C bring-up, continued

Rev C boards arrived 2026-10-16. Board 1 is on the bench: USB-C to this
host (/dev/ttyACM0 when the devboard is unplugged), Ethernet on the bench
LAN, **no 19 V yet** (no JST-PH plugs on hand; ordered). It runs entirely
from USB.

Board 1 identity (secrets in firmware/board-cb374b_recovery_token,
gitignored, mode 600):
- device id granite-cb374b, base MAC 90:64:9b:cb:37:48, Ethernet MAC
  ...:4b, DHCP 192.168.64.115, mDNS granite-cb374b.local
- admin password: `deadbeef` (also in firmware/board-cb374b_recovery_token; 5 failed
  logins = 60 s lockout). Make an API token for scripted tests (POST
  /api/v1/session, then POST /api/v1/security/tokens); the values of the
  existing ones are not kept anywhere.
- running ota_1 (valid, master 2026-10-20 with the rules fixes);
  ota_1 holds the previous build. Modbus TCP is enabled (allow
  192.168.64.99/32).

Done on board 1 (2026-10-16 .. 19):
1. Power and USB: enumerates, C6 v0.2, 8 MB flash, no eFuses burned.
2. `hwtest` example: U14/U15/TMP1075 answer (0x20/0x21/0x48), press
   deadline drops GPIO10 (659 ms for 600 ms) and resets U14, U15 sense
   reads, 1-wire and external I2C come up empty, status LED patterns seen.
3. Ethernet: link, DHCP, SNTP server from option 42, mDNS, /id on 80 and
   443, cert fingerprint matches the console, 0.8 ms ping.
4. OTA rollback: good image pushed over HTTPS -> ota_0 pending -> valid
   after an authenticated request; panicking image -> ota_1 -> bootloader
   rolled back to ota_0 ("last invalid ota_1", boot reason panic).
5. MQTT: bench broker (below) over TLS + password; staged section
   auto-confirmed on connect; status/state/event/log all publish.

Firmware fixes found by the board (all on master):
- 2076c02: SPI2 had no DMA (64-byte cap broke every W5500 frame); port-80
  httpd stack 6144 overflowed on /id.
- SNTP: lwIP drops the option 42 servers of a DHCP ACK unless "servers
  from DHCP" was on before it arrived; SNTP was started after the
  address, so it never synced. Now started in bring-up, before the
  interface starts (net.rs start_sntp_early, also in wifi_dev).
- 8419e1f: ESP-IDF app version came from git describe (25 chars) and
  overflowed esp-idf-svc's 24-char FirmwareInfo::version, so every OTA
  slot read failed (ESP_ERR_INVALID_SIZE); now CONFIG_APP_PROJECT_VER,
  checked against the crate version in build.rs. esp_idf_svc::ota
  "Dropped" spam filtered to Warn.

Bench MQTT broker: `firmware/tools/mqtt-broker.sh <ip> [user ...]` sets
up mosquitto as the systemd user service `granite-mqtt` (8883 TLS with a
local CA + password on all interfaces; 1883 anonymous on 127.0.0.1 only).
State in ~/.local/share/granite-mqtt (ca.crt for the board's MQTT page,
users/<name> holds each password). Users: granite-cb374b (the board),
bench (host-side tools). Board 1 publishes under granite/bench/granite-cb374b.
Watch: `mosquitto_sub -h 127.0.0.1 -v -t 'granite/#'`.

Bench gotcha: `espflash monitor` (even `--no-reset`) connects through the
ROM loader and can leave the chip halted in download mode. The W5500
keeps the switch link LED on, so the board looks connected but answers
nothing. For a passive capture use a plain reader (e.g. `picocom` or
`cat /dev/ttyACM0` after `stty`), and only use espflash monitor when a
reset is wanted.

Next steps, in ADR 0001 bring-up order:
1. MQTT: done 2026-10-20. `cmd` round trip with `ack/<id>` works
   (config_get). `config_set` of the mqtt section (site bench -> bench2)
   over MQTT: ack "staged" on the old topic, `online:false` there, client
   rebuilt and online under bench2 within 4 s, no reboot. With
   auto_confirm_on_connect off the section was reverted at exactly 300 s
   ("not confirmed in time") and the client went back to bench, uptime
   continuous; with it on, both directions confirmed on connect. Host
   tools can use the anonymous 127.0.0.1:1883 listener (no password).
   Bench admin password is `deadbeef` (dev only, fine in the open);
   API token `bench-2026-10-20` exists, value only in that session's
   scratchpad. SNTP from DHCP
   option 42 works since the fix below (the bench dnsmasq now hands out
   162.159.200.1; for sites, run an NTP server on the site router).
   Design point for the fleet ADR: MQTT `ts` is monotonic ms since boot
   by definition (granite-core msg.rs); fleet logs will want a wall-clock
   field too.
2. Actuator: done 2026-10-20. Raw 10 s `press` on node 1 over MQTT:
   ~100 ohm (100R series + photoMOS) across J11 1-3 for PWR, OL
   otherwise; RST the same on 2-3. Deadline: an image built with the
   bench-only `deadline-test` feature (relays.rs: release() stalls 8 s)
   and a 2 s press: meter showed ~2.5 s then OL (esp_timer deadline =
   press + 500 ms slack, U14 reset by GPIO10), the task woke 8 s later,
   logged "press deadline fired", the action failed with
   `expander_fault`, a `fault`/`press_deadline` event went out and the
   fault bit cleared itself 1 s later. Normal image back on ota_1.
3. OTA: done 2026-10-20, with event gaps (below). Pull over MQTT
   (`ota` with url + sha256; host served the image over HTTPS with the
   bench broker's server.crt, which the board trusts through the MQTT
   CA): started, downloading every 10 % (1.7 MB in ~6 s), verified,
   reboot, valid ("expanders, link and the broker"). Probation timeout:
   t_validate_s 120, broker stopped at 90 % and no authenticated HTTPS:
   "not confirmed within 120 s (expanders=true link=true broker=false
   https=false)", marked invalid, bootloader back to ota_1 ("last
   invalid ota_0"). t_validate_s back to 600; a second, normal pull left
   the board on ota_0 (same normal build as ota_1), valid.
   OTA event gaps, all FIXED and verified on board 1 2026-10-20:
   - `ota` command ack: the worker resolves a pending `ota` on the
     `verified` / `failed` event ("verified, rebooting into the new
     image"). `reboot` and `factory_reset` ack "rebooting" on accept.
   - Lost events: the event bus keeps the first 32 events of a boot for
     the first `subscribe_replay` (the MQTT worker), and the worker
     holds up to 64 events while it has no session and sends them on
     connect. `pending_verify` now arrives before `valid`.
   - `rolled_back`: the failing image writes its verdict to NVS
     (ota/rb_why); the image the bootloader falls back to emits
     `rolled_back` on its first broker session ("ota_0 not confirmed
     within 120 s (expanders=true link=true broker=false https=false);
     version 0.1.0; now running ota_1") and records the slot's ELF sha
     in ota/rb_seen, so it goes out once. A rollback after a reset or
     crash gets "did not confirm (reset or crash before its probation
     ended)" (seen in the console after a hard reset).
   - Cut-off events: `planned_reboot` calls `mqtt::flush` (up to 3 s):
     the worker marks a flush ticket done once the events and replies
     queued before it are handled and the esp-mqtt outbox is empty. The
     pull's `verified` event and ack went out 0.7 s before the reboot.
   OPEN, intermittent: twice the board hung on the first boot of a
   freshly uploaded image (16:27 and 16:59, both upload + /reboot, no
   console reader). Console stopped mid-line at ~871 ms, right after the
   port-80 httpd registered "/id"; no network. A hard reset (RTS via
   pyserial, not espflash) recovered it through a bootloader rollback.
   Not reproduced in 3 new-image boots + 3 reboots without a reader.
   ESP-IDF's USB-JTAG VFS write drops after 50 ms, so a blocked console
   is not proven. Field risk: an image that hangs early never runs its
   probation timer, and nothing resets it unless the task watchdog sees
   it. FIXED 2026-10-20 with a hardware probation deadline: the
   bootloader's RTC watchdog stays on into the app
   (CONFIG_BOOTLOADER_WDT_DISABLE_IN_USER_CODE; app side only, no
   bootloader reflash); confirmed images switch it off at the top of
   main, a probation image keeps it (60 s early, then t_validate + 60 s)
   until it confirms. Verified on board 1: a probation boot armed at
   120 s (t_validate 60), confirmed at 17 s, no reset by 154 s; a
   `boot-hang-test` image (bench feature: hangs at the top of main while
   on probation) was reset by the watchdog 59 s after boot, rolled back
   to ota_0, which reported `rolled_back` on MQTT. Boot reason reads
   `int_watchdog`. The cause of the hang itself is still unknown.
4. Modbus TCP: done 2026-10-20 (bench client: a stdlib Python script,
   raw MBAP; pymodbus/mbpoll not installed). Enabled through `sec.modbus`
   (staged, confirm needed) with allow 192.168.64.99/32; left on.
   Verified: every table matches /status (board 26.25 C, uptime, link,
   mqtt, probes 0x8000, fault flags 8 = VIN window on USB, fw 0x0001,
   map 1); exception 2 for any address outside the map, 3 for qty 0 /
   too large / bad command word / bad coil value, 1 for an unknown
   function (a PDU under 5 bytes, e.g. FC 7 or a 4-byte FC 43/14, is
   closed instead: rmodbus' MBAP minimum); coil/command writes become the
   same commands as MQTT (on: accepted -> power_on_timeout without a
   motherboard; reset on an off node refused); allow-list refuses,
   disable closes the port, max_conn 4 holds.
   Fixed on the way (all verified on board 1):
   - unit 0 / 0xFF were rmodbus "broadcast": a write was EXECUTED with
     no reply, so a client (Modbus TCP says send 0xFF) retried and
     pressed again. Now served as this unit, request unit echoed.
   - holding register 9 stayed `accepted` forever; the server now
     follows action_done / action_failed of its last command (2 -> 4
     seen for power_on_timeout).
   - lwIP socket table was 10: Modbus got 2 connections, and on ENFILE
     the listener was dropped and rebound. Now 20 sockets, and ENFILE /
     EMFILE backs off without dropping the listener.
   FIXED (found here, not Modbus): every bearer-token request ran
   PBKDF2 (20k) per stored token: 3.4 s per request, and three parallel
   ones reset the board through the task watchdog (the KDF starved the
   idle task); parallel logins would do it without credentials. Now
   tokens are `s256:` SHA-256(salt || token) (old PBKDF2 ones rehashed
   on first use), and the hand-written PBKDF2 sleeps one tick every 500
   rounds (a 1 ms sleep is a busy-wait in ESP-IDF's usleep; 10 ms is a
   real vTaskDelay). Verified: token request 0.43 s, unknown token
   0.42 s, login 2.5 s; 6 bogus tokens + 4 logins in parallel queue for
   24 s with no reset while Modbus and MQTT keep answering. Bench
   tokens now: bench-2026-10-20 and bench-fast (values only in that
   session's scratchpad); the lost ota-test token was deleted.
5. Rules: done 2026-10-20. Explained to the user; three fixes, all
   verified on board 1 (rule set and node sense restored afterwards):
   - a rule's force_off is a safety action (`Fired::request`): force
     (acts on an `unknown` node) and preempt (a busy node's job ends
     `failed: preempted`, its relay opened first). Bench: a 10 s RST
     press on node 1 preempted by a board_temp > 20 C rule, then an 8 s
     PWR force_off (node 1 sense `ignore` for the test). Host tests
     cover the unknown-state case (no way to make a node unknown on the
     bench).
   - `missing: ignore|trip` per rule; shipped rule 1 (probe_max) trips,
     so no probe answering forces all off after 30 s. Bench: rule_fired
     with value null after the hold.
   - the dispatcher and sense threads are under the task watchdog
     (ADR said so, nothing did it). Bench: `dispatcher-hang-test`
     feature hangs it on probe_scan; reset 10.05 s later, boot reason
     task_watchdog. Six parallel logins plus MQTT commands: no reset.
     The watchdog log names the task `pthread` (Rust thread names do
     not reach FreeRTOS).
   Still not tested with real inputs: dry_in (short J10 1 to G),
   node_on (needs a motherboard), probes.
6. Once plugs arrive: 19 V on J3 (VIN sense, trim), DS18B20 probes
   (3-wire only; fault bit 2 with a configured absent probe), dry
   contacts on J10, a real motherboard on a node connector (PLED level,
   docs/controller.md item 4).

Fleet backend (discussed 2026-10-20, no decisions yet): tens to
hundreds of containers with tens of controllers each, MQTT, "always
reachable". Human-facing parts will be Geomind/Hero. The user is talking
to the developer of granite_code/granite_metal_provider_and_broker
(empty today; its source is geomind_code/my_metal_provider_and_broker:
a per-site provider with PDU power control and PXE, plus a marketplace
broker - no MQTT, metrics, logs or uptime history). Strawman for the
ADR: the site provider hosts the container's edge broker and gains a
"granite controller" power driver next to the PDU drivers; a separate
DC-side fleet service keeps the registry, node uptime history, metrics,
logs and a command API with an HTTPS fallback path. Firmware impact
already known: a broker failover list (one host today) and a wall-clock
field in MQTT messages. Write the ADR only after that conversation.

Open nits seen on the board:
- /api/v1/firmware `slots` lists only the next slot, size 0, empty
  version; `validate_left_s` stays null while pending.
- Every status poll logs ESP-IDF's `E esp_ota_ops: ota data invalid`
  while otadata is blank (gone on this board after the first OTA); an
  option is to have ota::info() skip the boot-slot query when blank.
- hwtest's first read_leds/read_dry runs before the sense poller's
  first read and logs "not present"; its messages assume a bare devboard.
- PUT /api/v1/security/mqtt-credentials returned
  an empty body although it worked.
- `probe_scan` is still a logged TODO (1-wire re-scan not wired).
- `config` "staged" event has detail null when staged by an MQTT
  `config_set` (the HTTP path says "confirm within N s").
- `security` events (login, token_created) carry peer "0.0.0.0": the
  HTTP layer does not pass the client address through.
- `fault` and `action_failed` events from a stalled tick carry the `ts`
  of the tick start (accepted + 2.0 s in the deadline test), not of the
  moment the fault was seen (8 s later).

Bench gotcha (2026-10-20): the host's xHCI controller 0000:7b:00.0 died
(`HC died; cleaning up` in the kernel log) with the board on its hub.
The port kept powering the board, but after a software reboot the board
never came back on the network until the controller was rebound
(`echo 0000:7b:00.0 | sudo tee /sys/bus/pci/drivers/xhci_hcd/unbind`,
then `.../bind`), which also power-cycled it; the same image then
booted in seconds. No `/dev/ttyACM0` at session start = check
`journalctl -k` for this first. Passive console capture that survives
re-enumeration: a loop of `stty -F /dev/ttyACM0 raw 115200; cat`.

Firmware docs: firmware/docs/adr/0001-firmware-architecture.md,
firmware/README.md, firmware/docs/gaps-2026-10.md (all 14 closed).
`cargo run -p granite-sim -- serve` runs the real core and page over
fakes for page work without a board.

Decided (user, 2026-10-09):
- Code lives in firmware/ in this repo.
- Node control: everything - power on/off, button press, long press
  (forced off), reset, power cycle with delay, staggered power-on of the
  8 nodes. Node state read back from the power LED sense.
- Network loss / standalone: configurable default states plus local
  rules (e.g. temperature threshold -> action), persisted on the board.
- Security is in scope: signed firmware updates, authenticated API.

Decided (user, 2026-10-10):
- Rust on ESP-IDF confirmed.
- Northbound: MQTT (logging, monitoring, control) + HTTPS setup page.
  Modbus is required too (Modbus TCP on the controller, RTU to ducks
  later; "maybe in some way over that" is an open question in the ADR).
- Boards are submerged after configuration: preconfiguration and full
  reset must work with only network access; the user notes each
  board's MAC. The ADR covers this (commit-confirmed network changes,
  OTA self-validation + factory partition, per-device recovery token).

Background on the ESP-IDF choice:
- Rust on ESP-IDF (esp-idf-svc, std): ESP-IDF's W5500 MAC-raw driver +
  lwIP, two-slot OTA with rollback (8 MB flash), NVS, HTTP server. User
  knows ESP-IDF. Bare-metal esp-hal + Embassy (embassy-net-wiznet) was
  the alternative; rejected for now because OTA/rollback is less mature
  and a sealed, submerged board needs safe remote updates. Put hardware
  access behind traits so a later move stays possible.
- Firmware scope from docs/controller.md: 8 relay channels via MCP23017
  (IOCON.ODR = 1 on every expander at boot, defined state while in
  reset), power LED sense, 1-wire temperature probes, external I2C
  (software I2C on pins 6/7 unless LP I2C works from the HP core - open
  item 3), dry-contact inputs, radio off, console moved off the probe
  pins, USB as service/recovery port. Later: RS-485 ducks (Modbus RTU),
  power board via J8 (LTC4282 EEPROM provisioning, auto-retry, disable a
  channel after repeated faults).

## Boards ordered

ORDERED at JLCPCB 2026-10-09 (user): rev C, 199 mm, HRO USB-C, from
fab/ regenerated that evening (commit df7f4d1). Waiting for the boards;
bring-up when they arrive.

Last change before the order: C36 (1n/2kV Bob Smith cap, 1812) KEMET
C1812C102KGRACTU C2167446 (141 left at JLC) -> Yageo CC1812KKX7RDBB102
C118019 (26k), same footprint. Basic-part alternative if it runs out
again: C9196 1206B102K202NT (1206, needs a footprint change).

State 2026-10-09, branch eight-node-expander:
- J2 is now HRO TYPE-C-31-M-12 (C165948; the GCT USB4105-GF-A was down to
  140 at JLC). Commit c0d9b0e. Rear fan-out re-laid (D+ join behind the
  pads, CC straight back to vias, VBUS 0.5 mm on B.Cu).
- Board shortened 250 -> 199 mm (tools/compact.py): power, Ethernet and
  MCU blocks unchanged with their routing; everything right of x 134.8
  re-placed and rerouted by KRT. Top edge J6 | J7 | J9 | J11-J14, bottom
  J8 | J10 | J15-J18. U14 over U15 next to the relays; U16/U17 in the band
  between the relay halves (inputs towards their connectors); relay
  columns at 13.6 mm pitch.
- Checks: ERC 0; DRC 0 unconnected, 0 parity, violations: 2 U1 antenna
  silk (intended), J2 differs from library (silk dropped, GND pads solid),
  single-spoke thermals on THT GND pins J8.4 (as before) and J5.3 (both
  also on the In1 plane). Vias in pads: only the 4 U1 pad 29 thermal vias.
  Inner planes carry no tracks (KRT's redundant In1/In2 tracks removed).
- Silk: pin legends on all connectors except J9/J10 (no room between
  U14/U15 and the connectors; names are there).
- The previous 250 mm board with the GCT USB is tag rev-c-usb4105.
- fab/ (gitignored) regenerated with pcbkit jlcfab after the C36 swap:
  BOM 63 lines / 198 parts, CPL 198 placements (32 rotation-corrected).
  fab/granite_controller-jlc-order.zip bundles Gerbers zip, BOM, CPL and
  the rotation list; jlcfab.py does not build it, re-zip by hand.
- J2 3D model: 3dmodels/HRO_TYPE-C-31-M-12.step (EasyEDA), board-only.
- J8: stays DNP (user, 2026-10-09).

## Repo state

- Documentation site (installer/operator, mdBook) in site/: `mdbook build`
  or `mdbook serve` in site/, output site/book/ (gitignored). Written
  2026-10-10 from README, ADR 0001, controller.md and granite-core; the
  Modbus map and the Home Assistant page are included from firmware/docs.
  Not hosted anywhere yet (a GitHub Pages workflow was not added; the
  user decides on hosting).
- `master` holds all current work (rev C). On 2026-10-20 it was
  fast-forwarded to the old `eight-node-expander` branch, which was then
  deleted, and the colleague's upstream (rev A, STM32) was dropped as a
  remote: the designs have diverged. GitHub still marks the repo as a
  fork until the user detaches it. Push only when the user asks.
- Remotes: `origin` = github.com/delandtj/granite_controller (fork),
  `upstream` = github.com/LeeSmet/granite_controller (colleague, Lee
  Smet). No PR to upstream yet: the user wants to discuss more first.
- Untracked, keep out of git: `W5500_ds_v110e-*.pdf` (datasheet the
  user dropped in the root), KiCad lock files.

## What the board is now (rev C)

19 V-powered controller (PoE until 2026-10-07) for one frame of 8 Strix
Halo motherboards, fully submerged in dielectric fluid (single-phase now, maybe
two-phase later; materials chosen for two-phase). Details and pin
map: docs/controller.md.

- ESP32-C6-WROOM-1-N8 (8 MB, OTA headroom; N16 drop-in) + W5500 SPI Ethernet (replaced STM32H563 +
  LAN8742A). PoE dropped: J3 19 V input -> PTC/TVS/Schottky ->
  LMR51430 buck -> 5 V, HR911105A plain magjack; TLV62569 3V3 unchanged.
  All parts carry LCSC numbers for JLC assembly.
- 8 relay channels: 16 TLP176AM photoMOS (RST + PWR per node), driven
  by MCP23017 U14 @0x20 on the internal I2C bus, held in reset by
  default (all buttons released if the MCU is not driving it).
- Node power sensing: 5-wire node cable (PWR_SW, RST_SW, GND, PLED+,
  PLED-), 2x TLP290-4 AC-input optos -> U15 MCP23017 @0x21 (internal
  bus) GPA0-7. U15 GPB0-3 read 4 dry-contact inputs (J10). J9 I2C
  sensor port (Qwiic order). VIN_SENSE divider on GPIO5.
- Layout target (user): long strip ~30 mm high along the top of the
  frame: power 30x30, MCU+ETH+USB 30x40 (RJ45 on the top edge, antenna
  toward the bottom edge), probes + J8, then the 8 node connectors.
- External I2C + EXP_nRESET + EXP_INT on a protected 2x6 header J8 for
  add-on MCP23017 modules (0x21-0x27) and the future power board
  (LTC4282 x8 @0x40-0x47).
- Sheets: Power, Ethernet, MCU, Relay channels, Expansion header.
  Generated, then tidied by hand-style layout; netlist verified
  identical across the tidy. ERC 0. PCB regenerated from scratch after
  the 19 V change (162 footprints, 0 parity issues); nothing placed,
  outline 250 x 50 mm (180 x 40 was too dense) with 4 corner M3 NPTH
  holes; first grouped placement done (tools/place.py), unrouted.

## PCB setup already done

4 layers, JLCPCB JLC04161H-7628 1.6 mm stackup (0.2104 mm 7628
prepreg, 1 oz outer, 0.5 oz inner), net classes with widths, diff
pairs from a 2D field solve: ETH 0.16/0.15 = 101R in fluid (Er 2.1),
USB 0.25/0.15 = 89R in air (service port, used out of the fluid).
Confirm with JLC's impedance calculator before ordering.
granite_controller.kicad_dru
(USB-C NPTH exception; the PoE clearance rule is gone with PoE).

## Decisions made with the user

- Fab: JLCPCB for bare board and assembly (changed from PCBWay on
  2026-10-07; the colleague preferred PCBWay). 4 layers. Claude does board setup, outline,
  holes and grouped placement; the user routes critical nets (Ethernet
  pairs, USB, switcher loop, crystals).
- 8 nodes per frame. MCP23017 relay drive, fail-safe via reset.
- MCU: ESP32-C6 (user has S3/C5 experience and likes ESP-IDF).
- Power board (docs/power-board.md): separate 8-channel 19 V board fed
  by two 3 kW supplies; LTC4282 per channel with I2C monitoring;
  default-ON; auto-retry on faults (firmware disables a channel after
  repeated retries); LTC4282 EEPROM provisioned by controller firmware
  over I2C on boot, never storing FET_ON = 0.

## Open items

Controller (docs/controller.md "Open items"):
1. Soak tests (HR911105A magjack, JST housings, J8 header housing).
2. 19 V tap location and fuse on the power board side.
3. ESP32-C6 LP I2C usable from the HP core? (else software I2C stays).
4. ESP32-C6-MINI-1-N4 stock/price.
5. Layout: board outline, connector edges (8 node connectors, 4 probe
   connectors, J8, RJ45, USB-C), mounting holes, assembly side,
   surface finish (ENIG vs HASL). The user said "a lengthy board".
6. New passive MPNs follow YAGEO/Samsung naming but are unchecked
   against distributors.

Power board: PSU model (paralleling, PMBus), node 19 V input connector
(barrel jack too weak for 10.5 A), main fuse location, outline.

## Next likely steps

1. Architecture is being iterated (see "Pod architecture" below): this
   board stays as the integrated controller for small (one-frame) sites.
2. PCB: review fix pass done (see the top of this file); next is the
   independent check, then order prep.
3. Open hardware questions: motherboard front-panel pinout (PLED drive,
   ground switch returns) - user waits for the board doc/STEP; J8 as the
   power-board link (fit by default, keyed connector?); PSU remote control
   (ON/OFF via photoMOS, current/voltage setpoint via I2C DAC with a
   hardware-clamped range) - belongs on the power board.

## Pod architecture (2026-10-08, iterating)

A pod holds 4 nodes up to 9 frames x 8 plus storage, PSUs and more. Idea:
one controller per pod outside the fluid (network, Wi-Fi, bus master) and
a family of "ducks" near the equipment (frame duck, storage duck, PSU
duck, more to come) on RS-485 / Modbus RTU (I2C is board-local only).
Proposed: a common duck core as a KiCad design block (MCU, RS-485, 19 V
front end, unique ID), self-describing ducks (identity + capability
registers), generic channel kinds (contact, binary in, temperature,
analog in/out, switched out). Open: controller location and bus length,
max ducks per pod, polling vs events (CAN), ground bonding across frames,
addressing without DIP switches. Docs to write when settled:
architecture.md, duck-core.md, duck-protocol.md.

## Tools and gotchas

The generic board tools moved to ~/Electronics/pcbkit (own git repo,
2026-10-09): `pcbkit <tool> ...`, run from the project root; settings for
this board are in pcbkit.toml (stackup, net classes, connector legends,
KRT paths and nets, U1 pad 29 exception). pcbkit README has the pipeline.
Only the board-specific placement scripts stay here: tools/place.py,
tools/compact.py.

- `pcbkit mkboard <netlist.xml> <project_dir> <out.kicad_pcb>`:
  builds a fresh board from a KiCad XML netlist (footprints linked by
  symbol UUID, nets assigned, parked in a grid). Only for an unplaced
  board: it discards placement. Write `out` to a scratch directory and
  copy only the .kicad_pcb back: pcbnew's Save rewrites the
  .kicad_pro next to the board and strips schematic settings and net
  class patterns.
- `pcbkit setrules <pcb> <pro>`: applies the stackup, board rules
  and net class values. Run after mkboard.
- `pcbkit outline <in.pcb> <out.pcb> [L] [H]`: Edge.Cuts rectangle
  (default 180 x 40 mm, board uses 250 x 50, 1 mm corner radius) plus H1-H4 M3 NPTH holes
  4 mm in from each corner, board-only. Idempotent; re-run after
  mkboard (which parks parts at x < 0, left of the outline). Same
  scratch-dir rule as mkboard. Order: mkboard -> outline -> place -> silk -> setrules -> krt_route -> gndpour.
- `tools/place.py <in.pcb> <out.pcb>`: first grouped placement for the
  230 x 40 strip (zones along x: power 0-28, Ethernet 28-60, MCU/USB
  60-100, probes + J8/J9/J10 100-130, U14/U15/U16/U17 130-160, node
  channels 160-220 in 4 columns, ch1-4 top row, ch5-8 bottom row). RJ45
  and USB-C openings on the top edge, WROOM antenna overhanging the
  bottom edge. Anchors big parts, packs the rest per sheet. A start for
  hand placement only: re-running it discards hand moves.
- `pcbkit silk <in.pcb> <out.pcb>`: values to F.Fab, silk refs hidden
  on small passives (F.Fab keeps them), other refs 0.8 mm placed clear
  of pads, silk outlines, other courtyards and each other (connectors
  first). Run after place. Only expected silk DRC hits: U1's outline
  where the antenna overhangs the bottom edge.
- `pcbkit krt-route`: routes with KiCadRoutingTools (drandyhaas; checkout
  and venv in ~/Electronics/krt, pcbkit's default; rebuild the Rust core with
  CARGO_TARGET_DIR unset). Strips old copper, then planes (GND In1, +3V3
  In2), Ethernet and USB pairs, then everything on F.Cu/B.Cu only. Runs on
  a copy in ~/.cache/granite-pcb/route because KRT rewrites the .kicad_pro
  next to the board; only the .kicad_pcb comes back. Keeps hand placement.
  Grade with kicad-cli DRC, never with KRT's own checks.
- `pcbkit gndpour <in.pcb> <out.pcb>`: GND pours on F.Cu/B.Cu (1 mm short
  of the short edges for the rails) and
  0.6/0.3 stitching vias (group "gnd-stitch", re-runnable). Run after
  routing.
- `tools/compact.py <in> <out>`: the 199 mm re-placement of the region
  right of the MCU block (deletes its copper, places, moves the right edge,
  H2/H3, pours and keep-outs, lays the GND straps and escape vias KRT
  cannot find). Then silk.py, connlabels.py, refill, `KRT_EXTRA="--ordering
  mps" pcbkit krt-reroute --keep "*"`, then targeted passes on what is open.
- `pcbkit viaout <in> <out>`: moves vias out of SMD pads (KRT's plane
  welds) where a clear spot exists. KRT runs now pass
  --same-net-pad-clearance 0.15, which avoids most of them; check with a
  via-in-pad count after every KRT run.
- Prune (`pcbkit prune`) only when DRC shows 0 unconnected: on an open
  route both halves count as dangling and prune eats them.
- After KRT: delete any tracks on In1/In2 (redundant, they cut the 3V3
  plane) and any vias KRT adds inside U1 pad 29 beyond the 4 thermal ones.
- `pcbkit krt-reroute [--keep] NET...`: targeted pass after moving a few parts.
  Deletes all copper of the named nets, reroutes them on F.Cu/B.Cu, runs a
  GND pass without rip, refills zones via kicad-cli and copies the board
  back. Name every non-GND net on the moved parts plus the open nets.
  Does not clean GND stubs left at old pad positions.
- `pcbkit jlcfab [pcb] [out]` (KiCad python): JLC outputs into fab/
  (gitignored): Gerbers + Excellon zip, BOM (Comment/Designator/Footprint/
  LCSC Part #, grouped by LCSC), CPL (absolute mm, Y flipped like the
  Gerbers, THT parts at pad centre), and a list of rotation-corrected parts
  (subset of matthewlai/JLCKicadTools' table) to check in JLC's preview.
  Skips DNP (J8) and parts without LCSC (H1-H4, TP1-TP4).
- `pcbkit connlabels <in.pcb> <out.pcb>`: connector function labels on
  F.Silkscreen ("J11 NODE 1", "J3 19V IN", ...) plus a pin legend under
  the wire-to-board connectors (node: PW RS CM L+ L-; 19 V: + -; 1-wire:
  3V DQ G; I2C: G 3V DA CL; dry in: 1 2 3 4 G). Hides those silk refs
  (F.Fab keeps them), places around existing silk and pads, group
  "conn-labels". Run after silk.py, which would show the refs again.
- `pcbkit/pcbtool.py`: helpers for scripted hand routing (add tracks and
  vias at exact coordinates, delete a net's copper in a region) and
  render(): a region PNG with F.Cu red, B.Cu blue, highlighted nets and
  DRC opens/violations drawn in. `pcbkit prune <pcb> <drc.rpt>` deletes
  what DRC flags as dangling; repeat DRC + prune until clean.
- `pcbkit handroute <pcb> <plan.py> [--save]` (added 2026-10-22, the
  loop that closed the pm-19v-200w opens): a plan of ('delseg', net,
  layer, (x0,y0,x1,y1)) / ('delvia', ...) / ('delreg', ...) / ('trk',
  net, layer, [pts], w) / ('via', net, (x,y), size, drill) applied to a
  copy in ~/.cache/pcbkit/<board>/handroute, zones refilled, kicad-cli
  DRC, region issues listed; H/V/45 enforced on tracks. `pcbkit view
  <pcb> <png> x0 y0 x1 y1 [NET...]` draws every copper layer (In1
  green, zone fills, labelled vias) for reading a spot before planning.
  Working method: dump the region's pads/tracks/vias with pcbtool, draw
  it with view, write the plan with explicit coordinates, run handroute
  until the region is clean, then --save.
- Board state 2026-10-08 (afternoon, superseded): placement by place.py + the user's
  hand moves (whole board shifted +25/+28.6 mm on the sheet, MCU-corner
  parts moved), routed by krt_route.sh, poured by gndpour.py, targeted KRT
  passes on the open nets. J8 area spread: RN1 (132, 68.5) and RN2
  (132, 73) rot 180 between U1 and J8, U7 (146.3, 69) beside J8 pins 9-11,
  U18 (151.3, 68.5) above J10. U7/U18 centre GND pads 3+8 strapped on F.Cu
  to a 0.6/0.3 GND via 1.2 mm right of pad 8 (KRT cannot reach them). 22
  nets rerouted (all J8/RN/DRYC/EXP nets incl. EXP_nRESET_INT, which ran
  through the RN1 spot, plus the W5500 SPI and ETH_LED_ACT).
- Board state 2026-10-08 (evening): FULLY ROUTED, first pass. DRC
  (refill zones first: KRT does not refill, a stale fill shows as
  hundreds of fake clearance errors): 0 unconnected, 0 parity, 4
  violations: 2 silk (antenna overhang, intended), 2 starved thermals on
  J8 GND pins 3/4 (THT, also tied to the In1 plane). ERC 0. Hand work
  (scripted through tools/pcbtool.py, geometry in the commit messages):
  - Ethernet MDI: J1 -> R20/R21, C28/C29, R22/R23 -> 33R row straight;
    TX/RX crossovers between the 33R row and U3 on 0.5/0.2 vias (N line
    on B.Cu). W5500 AVDD pins 4-8 and 11-15 joined under the LQFP body,
    GND pins 9/19 to inner plane vias, EXRES straight to R15.
  - Crystal: XO on F.Cu around Y2's right, XI under Y2 on B.Cu.
  - SPI/INT/RST: the W5500 edge order (CS SCLK MISO MOSI INT RST) is the
    reverse of the ESP32 west column (CS SCLK MOSI MISO INT RST), so each
    line runs east on F.Cu at its own 0.8 mm level and transposes on its
    own B.Cu column at x 96-100; RST stays on F.Cu. Pull-ups R16/R17/R18
    moved into the bundle (x 80-86) with +3V3 plane vias.
  - GPIO_IO15 to RN2 under the module on B.Cu (clear of the U1 thermal
    pad), GPIO_IO8 over to a via beside RN1; C6_EN, EXP_nRESET,
    I2C_EXT_SCL/SDA, HDR_IO15 rerouted by KRT around them.
  - USB: R34/R35 moved next to U1 pins 13/14, U4 -> R34/R35 routed as a
    0.25/0.15 pair (KRT route_diff); connector side unchanged (D+/D- pad
    joins under the receptacle, normal for USB-C).
  - Power section re-placed for tight loops (the old one had U8's input
    caps ~17 mm and its bootstrap cap ~20 mm away): U8 at (47.5, 62) with
    L3 0.7 mm off SW, C42/C41/C40 on a VIN bar under pin 3, GND via under
    the body, EN tied to VIN under the body, C43 above, R37/R38 at FB;
    C44/C45 and D2 at L3's output. U2 at (32, 59) with L1 at SW, C3 under
    VIN, C4 at the output, EN tied to VIN under the body. Input chain
    J3 -> F3 -> D8 with D1 below. C51 moved left of its old spot.
  - The rest of the board is KRT. KRT put many F.Cu tracks under the ESP32
    module body (see next steps).
  - Later the same day: connector function labels + pin legends on the
    silk (tools/connlabels.py); R15 swapped to C11692 (stock); 3D models
    for K1-K16, U16/U17, J1, U1 in 3dmodels/ (see its README); JLC
    outputs via tools/jlcfab.py. All LCSC parts were in stock for 8
    boards on 2026-10-08 (jlcsearch); 27 Basic / 33 Extended.
- Board state 2026-10-08 (night): review fix pass (docs/review-2026-10.md
  "Status"). New parts R65/C52 (VBUS bleeder, left of J2), D9 (BUCK_5V
  crowbar, below D2). Rule areas: 6.5 mm F/B.Cu keep-outs at H1-H4 and
  "U1 body: no F.Cu tracks"; DRU rules for hole clearance 0.3 mm and
  non-GND vias 0.5 mm from U1 pad 29. Zones end 1 mm from the short
  edges. Autorouter vias moved out of SMD pads (only the U1 pad 29
  thermal vias stay in a pad). tools/silk.py and tools/connlabels.py
  now write 1.0/0.15 mm text and widen footprint silk to 0.15 mm.
- Check after any change: `kicad-cli sch erc --severity-all`,
  `kicad-cli pcb drc --schematic-parity`. Expect 0 parity issues.
- kicad-cli occasionally re-serializes granite_controller.kicad_pro;
  `git diff` it and restore with `git checkout` if it changed.
- Do not use the KiCad MCP schematic tools on these files: they
  reformat whole sheets. Edit s-expressions as text.
- If KiCad has the project open, it can overwrite files edited on
  disk; ask the user to close or reload.
- Commits are GPG-signed; the pinentry times out when the user is
  away. Retry when they are back; never bypass signing.
- The original schematic generator scripts lived in a session
  scratchpad and are gone; the sheets are hand-editable now.
