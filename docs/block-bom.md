# Block wiring BOM

Status: draft (2026-11-22), nothing ordered. One block: 8 nodes, 1 core,
4 units, 8 port modules, 1 controller-fd (FD batch 1 connectors).
Cables and routing: `docs/block-cabling.md`.

`cabling/block-mouser.csv` is the list to upload to Mouser's BOM tool.

**Mouser was not checked live.** It refuses automated fetches. Part
numbers come from datasheets, distributor listings and search snippets.
That Mouser stocks each one, its price, its minimum quantity and the
exact suffix are to be checked in the BOM tool before ordering.

Power wiring and connectors are on hand (user, 2026-11-22) and not
listed. See "Check before building".

## Why not pre-crimped leads

Pre-crimped AWG 28 was the preference. It does not work here:

- No non-PVC pre-crimped GH, PH or 2.54 mm lead was found. JST's
  AGHGH28K305 is UL1571, which is PVC (not allowed in the fluid).
- The longest catalog GH lead found is 305 mm. W_FP needs up to 0.6 m.

So one halogen-free AWG 28 flat cable makes both W_LINK and W_FP. To
stay pre-crimped, have a cable house make 8 pieces of W_FP, non-PVC, once
one hand-made cable has proven length and pinout.

## Mouser list

| Use | Maker | Part number | Need | Order | Note |
|-----|-------|-------------|-----:|------:|------|
| Flat cable | 3M | HF365/06SF | 5.4 m | 1 x 30 m | 6-way, 1.27 mm, 7/36, polyolefin. Not 3365, which is PVC |
| Flat cable, 14-way | 3M | HF365/14SF | 0.15 m | 1 roll | W_CHAIN. Smallest roll seen is 100 ft; roll sizes not confirmed |
| Link sockets | Wurth | 61200623021 | 3 | 12 | 2x3, key, strain relief (W_LINK both ends, W_CORELINK unit end) |
| Chain socket | Wurth | 61201423021 | 1 | 5 | 2x7, key, strain relief, PBT; part number from listings, not the datasheet |
| W_FP plane housing | JST | GHR-04V-S | 8 | 18 | |
| W_FP plane contact | JST | SSHL-002T-P0.2 | 32 | 120 | AWG 30-26, insulation 0.76-1.0 mm |
| W_FP board housing | Harwin | M20-1060100 | 32 | 60 | 1-way Dupont style, as the header pinout is unknown |
| W_FP board contact | Harwin | M20-1180042 | 32 | 120 | gold, AWG 30-22 |
| Termination | Wurth | 60900213421 | 2 | 12 | JP301 of unit 4: 1-2 and 3-4 |
| Option: probes | JST | PHR-3 | 4 | 8 | controller J4-J7 |
| Option: I2C | JST | PHR-4 | 1 | 4 | controller J9 |
| Option: dry contacts | JST | PHR-5 | 1 | 4 | controller J10 |
| Option: PH contact | JST | SPH-002T-P0.5S | 21 | 60 | insulation from 0.8 or 0.9 mm (sources differ) |
| Option: sensor | Analog Devices | DS18B20+ | 4 | 5 | TO-92, for home-made probes |

Order quantities for cables and connectors, here and outside Mouser,
include 20 % on top of the spares (user, 2026-11-22), rounded up. The
30 m roll already covers that: 5.4 m needed, 6.5 m with 20 %. The extra
contacts and sockets are for practice.

Not in the CSV yet, no part found:

| Use | Need | Order | Why it is not listed |
|-----|-----:|------:|----------------------|
| Male IDC plug 2x7 for 1.27 mm cable, W_CHAIN core end | 1 | 4 | no plug confirmed that takes 1.27 mm cable and fits the core's right-angle socket. Look at Wurth WR-BHD male cable plugs |
| Male IDC plug 2x3, same family, W_CORELINK core end | 1 | 4 | as above. The two plugs sit 7.4 mm apart: try the fit |
| 19 V barrel plug for the AXB35's DC-045D jack, W_19V node end | 8 | 10 | jack size not measured (DC-045D is not one standard size). The best solder plugs found (Switchcraft 760 / 761, 5.5 x 2.5 mm) are rated 5 A, under the node's 8.7 A |

## Bench and board fixes (also in the CSV)

| Use | Maker | Part number | Need | Order | Note |
|-----|-------|-------------|-----:|------:|------|
| controller-fd J15 | Wurth | 61200621721 | 1 per controller | 3 | 2x3 right-angle THT box header, PBT, replaces LCSC C34945; JLC left it off (only bottom-side part). Hand-solder on B, pin 1 at the bottom of the column. Check the body overhang past the edge against C34945 |
| Core flashing | Adafruit | 2743 | 1 | 2 | SWD 2x5 1.27 mm to 2.54 mm breakout, takes the TC2030-CTX-NL. Check that the 2x5 header comes fitted |
| Core flashing | Adafruit | 1675 | 1 | 1 | 2x5 1.27 mm socket-socket ribbon, 150 mm: core J6 to the breakout, no hand-held Tag-Connect |

Breakout to ST-LINK/V2: 2 SWDIO, 4 SWCLK, 3 GND, 10 NRST (optional);
pin 1 is the board's 3V3, left open on a clone ST-LINK.

## Tools

| Job | Tool | Where seen | Note |
|-----|------|------------|------|
| IDC sockets on flat cable | Duratool D03028 | Farnell 3763200 | parallel press plier, 6-27.5 mm with adaptor |
| GH and PH contacts | Engineer PA-09 | Misumi, Pimoroni | listed for GH; JST's own GH tool is WC-GH2630 |
| M20 contacts | Harwin Z20-320 | Harwin | maker's tool; try the PA-09 first |

An IDC plier cannot crimp GH, and a crimp plier cannot press IDC.

## Outside Mouser (none found there, or not worth it)

| Use | Item | Need | Order | Note |
|-----|------|-----:|------:|------|
| Module screws | M2 x 5 socket cap DIN 912, A2: TME Kraftberg M2X5/D912-A2 | 32 | 100 | head 3.8 mm, 1.5 mm hex key, 0.15 Nm max; TME minimum 100 |
| Module washers | M2 split washer DIN 127 B, A2: TME Kraftberg K2_D127-A2 | 32 | 100 | OD 4.4 mm; TME minimum 100 |
| Board mounting | M3 screws, washers | 24 | 30 | hold: length and standoff follow from the bar |
| Node network | Cat6 patch, non-PVC jacket and cores | 8 | 12 | hold: length; LSZH alone does not prove the cores |
| Controller network | Cat5e or better, solid, non-PVC, straight | 1 | 2 | 3 m; solid, as stranded wicks oil through the wall |
| Uplink | SFP+ DAC 10G, generic coding | 1 | 2 | hold: route and jacket material |
| Option: potted probe | Adafruit 642 (PTFE cable) | 4 | 5 | not Adafruit 381 or SparkFun 11050 (PVC) |
| Splices | PVDF or polyolefin shrink, 1.6 and 2.4 mm | - | 1 m each | no adhesive liner (TE RW-175, HellermannTyton TCN24) |
| Dressing | PA66 ties 100 x 2.5 mm | - | 100 | HellermannTyton T18R |

## Core ribbons: core to unit 1

The pod layout puts the core in the infra frame, so its chain (2x7) and
link (2x3) reach unit 1 over ribbons, about 0.15 m, pin n to pin n.
Male IDC plug at the core (J301, J3 are sockets), socket at unit 1.
Pressing as for W_LINK below. Drawing: `cabling/core.yml`.

## W_FP: round the plane to the motherboard

The motherboard's header is on the other side of the plane (user,
2026-11-22).

- J2 / J52 sit 3.5 mm from the unit's bottom edge and exit over it. The
  cable turns round that edge, and round the bar if it runs there.
  Leave room below the plane for the bend.
- Start at 0.6 m, make one, route it, then cut the other seven:

  | Part of the run | Worst case |
  |-----------------|-----------:|
  | out of the plug and round the edge | 25 mm |
  | sideways, slot to node (36 or 58.7 mm pitch) | 100 mm |
  | across a 180 x 180 mm motherboard, header position unknown | 360 mm |
  | slack | 50 mm |
  | total | 535 mm |

- Peel 4 of the 6 ways and keep the red stripe on pin 1: 1 PWR_SW,
  2 RST_SW, 3 PLED+, 4 PLED-. Separate the last 20 mm for crimping.
- From the other side of the plane the order looks mirrored. Go by the
  stripe and a meter, never by position.

## W_LINK: pressing the flat cable

1. Cut square. Do not strip.
2. Slide the cable in, stripe at the socket's pin 1 triangle.
3. Press in one stroke until both latches click.
4. Fold the cable back, snap on the strain relief.
5. Meter it: pin n to pin n, no shorts between neighbours. The core
   end is not keyed, so mark pin 1 there.

## Check before building

- **Test crimp one of each.** The ribbon is 0.89 mm thick. That suits
  GH on paper; PH is at its lower limit; M20's insulation range was
  not found.
- **Soak test** the ribbon, sockets and housings in the fluid.
  Halogen-free is not the same as oil-proof.
- **Motherboard header**: standard single 2.54 mm Dupont pins by eye
  (user, 2026-11-22), so the M20 parts fit. Pinout and LED drive are
  not confirmed: meter them before plugging in. PWR_SW and RST_SW go on
  the switch pins that are not ground, the header's grounds stay open.
- **Power on hand**: 4 x XT60H-F and 1 x XT30U-F (female, cable side),
  no PVC wire in the fluid. Silicone is accepted in PAO (user,
  2026-11-23).
- **Core and switch-supply leads**: drawn as 0.75 mm2 behind the 63 A
  block breaker, with no fuse on the core. A short on one of them can
  overheat the wire without tripping 63 A. Give them their own small
  DC breaker outside the fluid, or wire sized for the block breaker.
- **Switch supply**: a bought CUI DQD50-48-S12-T with screw terminals
  (24-12 AWG, 0.4 Nm max), so no XT30 on its input. The switch-end
  barrel plug is not measured.
- **19 V barrel plug**: decided (user, 2026-11-23). Measure the jack,
  then find a plug rated for 8.7 A, or derate: none found above 5 A.
