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
| Flat cable | 3M | HF365/06SF | 5.1 m | 1 x 30 m | 6-way, 1.27 mm, 7/36, polyolefin. Not 3365, which is PVC |
| Link sockets | Wurth | 61200623021 | 2 | 12 | 2x3, key, strain relief |
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
30 m roll already covers that: 5.1 m needed, 6.2 m with 20 %. The extra
contacts are for practice crimps.

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
| Module screws | M2 x 5 pan head, A2 | 32 | 50 | head at most 4.4 mm, 0.15 Nm max |
| Module washers | M2 spring washer DIN 127, A2 | 32 | 50 | |
| Board mounting | M3 screws, washers | 24 | 30 | hold: length and standoff follow from the bar |
| Node network | Cat6 patch, non-PVC jacket and cores | 8 | 12 | hold: length; LSZH alone does not prove the cores |
| Controller network | Cat5e or better, solid, non-PVC, straight | 1 | 2 | 3 m; solid, as stranded wicks oil through the wall |
| Uplink | SFP+ DAC 10G, generic coding | 1 | 2 | hold: route and jacket material |
| Option: potted probe | Adafruit 642 (PTFE cable) | 4 | 5 | not Adafruit 381 or SparkFun 11050 (PVC) |
| Splices | PVDF or polyolefin shrink, 1.6 and 2.4 mm | - | 1 m each | no adhesive liner (TE RW-175, HellermannTyton TCN24) |
| Dressing | PA66 ties 100 x 2.5 mm | - | 100 | HellermannTyton T18R |

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
  no PVC or silicone wire in the fluid.
- **Core and switch-supply leads**: drawn as 0.75 mm2 behind the 63 A
  block breaker, with no fuse on the core. Give them their own small
  DC breaker outside the fluid.
- **Switch 12 V lead**: not listed, its board does not exist yet.
- **Installer manual**: site/ still lists rev D's PH 5p link and 2p
  12 V cables. FD uses neither.
