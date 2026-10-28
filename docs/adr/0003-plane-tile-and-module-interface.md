# Plane tile of 4 columns and a screwed port module

**Status**: Accepted; decisions 1, 2, 3 (INT_n, ID_n lines) and 4
(ID read path), and 11 (node connector) superseded by ADR 0004 (proposed 2026-10-28: N = 2/4/6/8
columns at 25 mm, front end under the module, rotating channel)
**Date**: 2026-10-26
**Updated**: 2026-10-26 (review asks answered: names, LM5069-1, new
module project, INT per column; decision 14 isolated aux added);
2026-10-27 (decisions 6 and 7: aux contacts are a 1.27 mm header and
socket, not pads and pogos; standoffs 7.0 mm)

Related: docs/plane-requirements.md (R1-R22, what this must meet),
docs/pm-interface.md (module mechanics and aux contacts, generated
footprints), docs/adr/0001-48v-power-plane.md (plane MCU, TLV message
format, safety rules: all carried over), docs/power-board.md (running
component spec; decision 4 there already records the screwed mount).

---

## Context

ADR 0001 defined the plane as one design in 2, 4 and 8 slots, each
slot a soldered DOSA eighth brick with a 7-pin aux row, fuses on the
plane input and the 12 V aux, and a Mini-Fit Jr 2x4 node connector
that carries power and the front-panel signals together. PP-48-2S was
drawn and routed that way.

On 2026-10-26 the module mount changed to screws (quick swap in the
tank, power-board.md decision 4). Drawing that mount (commit
abb9f50, reverted in 797cb1e, tag pm-mount-drawn) showed that the
rest of the plane rested on assumptions that no longer hold:

- The DOSA screw positions fall in the module's gate drive; the
  module only needs the brick outline, not its pins.
- Fuses do not work in the fluid (R11).
- TEMP is meaningless on a submerged module (R13), and SDA/SCL on the
  module were never used.
- Nodes peak at ~180 W, leaving the 200 W module ~10 % headroom.
- 8 slots on one board means one large board, one MCU and a single
  layout that does not repeat cleanly.
- The bus runs at 48-60 V, not 48-54 V; the OVLO windows sit too
  close to the top of the range.

---

## Decision

1. **Tile, 1x4.** The plane becomes a tile of 4 slot columns plus one
   core column, ~155 x 130 mm. Modules stand vertically, one per
   column, all the same way up ("up" as drawn). VBUS runs along the
   top edge, node connectors along the bottom edge. 8 slots = two
   tiles, each with its own MCU and its own controller address.
   Replaces the 2/4/8 slot family of ADR 0001.
2. **Identical columns.** Every slot column is one schematic block
   and one routed layout, repeated (KiCad 10 multichannel repeat
   layout, or a pcbkit script if that falls short). No column
   differs from another in copper or in I2C addresses.
3. **Column to core interface**: VBUS, GND, 3V3, SDA_n/SCL_n, INT_n
   (open drain), ID_n (analog). A TCA9548A mux in the core gives each
   column its own I2C segment, so the column's INA238 and its I2C GPIO
   expander (TCA9535 class) have the same addresses in every column.
   The expander carries the slot's control and status lines (enable,
   latch clear, PGD, PGOOD, latch state, photoMOS drive, LED sense).
   The OV latch stays local hardware, never in the expander path.
   INT_n: each column's expander INT goes to its own MCU interrupt
   pin (4 pins; the G070 has them). The MCU knows the column without
   a scan, and a stuck INT blinds only its own column. Rejected:
   wired-OR (one stuck INT hides the rest) and a TCA9545A mux that
   ORs the INTs (only 4 channels, the tile front end would move
   upstream of it). The column is the same either way.
4. **ID stays analog**: a resistor on the module, 10k pull-up and
   100 nF in the column, 1k series to an MCU ADC pin. Open = no
   module.
5. **Module (PM-19V-250W)**: the LTC7810 two-phase core stays;
   resized for 250 W continuous (~6.6 A per phase, current limit
   18-20 A). OVLO raised to ~66-68 V. TEMP (NTC) and SDA/SCL dropped.
   Not DOSA except the 58.4 x 22.9 outline; non-isolated; no pins.
6. **Mount**: four M2 screws through the module into SMT standoffs
   reflowed on the plane (Sinhoo SMTSO2070CTJ, C2916355, 7.0 mm gap;
   was SMTSO2060CTJ, 6.0 mm, to 2026-10-27); the screws
   carry VIN, GND and VOUT through clamped pads, with lock washers.
   Screw positions are not the DOSA corners; they are fixed in
   docs/pm-interface.md, asymmetric so a module turned 180 degrees
   cannot be screwed down (VIN on a GND or VOUT standoff), and the
   250 W module re-placement clears them. Every footprint (module,
   plane, bench carrier) is generated from that spec.
7. **Aux**: a 2x3 1.27 mm SMT pin header on the module underside
   (C41376035) plugs into a 2x3 1.27 mm SMT socket on the plane
   (C41397112), both LCSC stock. Contacts: OV_KILL, PGOOD, ID, GND,
   RUN, second GND. RUN leaves the power contacts. Any host treats ID
   open as a slot fault (a lost aux contact fails the other three
   signals unsafe). Was, to 2026-10-27: flat pads and pogo pins
   (C5157258); dropped because that pogo is through-hole and too
   short for the gap, the stocked SMT pogos carry flash gold only, and
   a gripping, wiping socket contact is the steadier one for a module
   bolted down for years. Parts from LCSC stock only.
8. **Protection, no fuses in the fluid**:

   | Level | Device | Reset |
   |---|---|---|
   | Box supply / branch | breaker or fuse OUTSIDE the fluid | by hand |
   | Tile input | LM5069 hot-swap, ~1.5 mOhm shunt (~30 A class), 2 FETs in parallel, INA238 | UVLO pulled low by the MCU (expander + FET); LM5069-1, latch-off |
   | Slot input | LM5069-2 + INA238 + OV latch (as ADR 0001) | MCU |
   | Module output | buck current limit; PGOOD loss -> slot latch (start-up blanking); INA238 ALERT above ~210-220 W input -> slot latch | MCU |

   - The core (MCU, mux, expanders, link isolator, 3V3 buck, 12 V aux
     supply) is fed from ahead of the tile hot-swap.
   - F1 (plane input fuse) becomes the tile hot-swap. F2 (12 V aux
     fuse) is dropped; the isolated aux supply limits on its own,
     AUX_SNS shows a short.
   - Reverse polarity: keyed tile input connector only.
9. **Input window 48-60 V**: OVLO ~66-68 V on the module (R3-R5
   divider) and on the plane (LM5069 dividers), the plane tripping
   first. 100 V parts stay; the bus TVS keeps surges under 100 V.
   The slot clamp D1 (SMBJ60A, 60 V standoff) is rechecked against
   the new top of range.
10. **Power distribution at 250 W per slot**:
    - Slot input 5.5 A at 48 V, ~7 A at 38 V: the 15 A front end of
      ADR 0001 (3.5 mOhm, 15.7 A limit, BSC070N10NS5, INA238 at ~25 of
      41 mV) is kept as is.
    - Slot output ~13 A, 18-20 A in a fault until the latch acts.
      NODE copper (VOUT standoff to connector, 15-25 mm): wide pours
      on both outer layers, 15-20 stitching vias, solid (no spoke)
      connections on standoff and connector pads. 1 oz is enough
      there.
    - Tile VBUS ~22 A at 48 V, ~28 A at 38 V: 30 A class input
      connector (keyed), bus bar or 2 oz copper along the top edge.
11. **Node connector split**: power on a Mini-Fit Jr 2x3 (3x +19 V,
    3x GND) or Mini-Fit HCS terminals, 14 AWG cable; PWR_SW, RST_SW,
    PLED+/- on a small separate latching connector. The 2x4 combined
    connector of ADR 0001 is dropped.
12. **Temperature**: per slot the INA238 die temperature; fluid
    temperature from the controller's 1-Wire probes. The tile link
    carries no 1-Wire line.
13. **Unchanged from ADR 0001**: one MCU per plane (STM32G070CBT6),
    isolated I2C link and TLV format to the controller, safety rules
    1-6 (default ON in hardware, MCU never in the safety chain), node
    front-panel primitives on the plane, the 12 V aux for the
    controller (now isolated, decision 14).
14. **Ground boundary: the controller side never shares the bus
    ground.** Power world: bus, tiles, modules, nodes (non-isolated
    modules, node GND = bus GND). Outside world: controller,
    Ethernet, USB service port, 1-Wire probes. Two things cross the
    boundary and both are isolated:
    - the I2C link and EXP_INT, through the ISO1640-class isolator on
      the tile (as ADR 0001);
    - the 12 V aux for the controller, from an isolated 48-60 V ->
      12 V supply (~12 W, flyback or an off-the-shelf module, 1.5 kV
      class functional isolation), replacing the non-isolated LM5164
      aux buck of ADR 0001. With the LM5164 the aux return tied the
      controller's GND to the bus GND and the I2C isolator isolated
      nothing.
    The node front-panel signals are already isolated (photoMOS,
    opto). This is functional isolation, not a safety barrier: the
    bus is SELV/ES1 at or below 60 V DC. The bus is earthed at one
    point, at the supply.

---

## Architecture Overview

```
  keyed 30 A input (48-60 V)
    |
    +--> core feed (ahead of the tile hot-swap)
    |      LM5164 -> 3V3 (MCU, mux, expanders, iso side)
    |      isolated 12 V aux -> controller (ground boundary)
    |
    +--> tile hot-swap (LM5069, 1.5 mOhm, 2 FET) + INA238 + TVS
           |
  VBUS ====+============+============+============+=====  (top edge)
           |            |            |            |
      [ col 1 ]    [ col 2 ]    [ col 3 ]    [ col 4 ]    [ core ]
       LM5069-2      ...          ...          ...         G070 MCU
       INA238                                              TCA9548A
       OV latch                                            ISO1640
       TCA9535                                             link conn
       module                                              12 V aux
       (screwed,
        1.27 aux)
       photoMOS x2
       LED sense
           |            |            |            |
      [ node pwr 2x3 ][ fp ]  ...                                (bottom edge)

  column <-> core: VBUS GND 3V3 SDA_n SCL_n INT_n ID_n
```

### Component Breakdown

1. **Core column** (one per tile)
   - STM32G070CBT6: I2C slave to the controller behind the isolator,
     I2C master to the TCA9548A; ADC for ID_1..4, bus and aux
     voltages; INT_1..4 on four interrupt pins, one per column.
   - TCA9548A: channels 0-3 to the columns, channel 4 to the tile
     front end (INA238, tile expander).
   - Tile hot-swap and its INA238; tile expander (or MCU GPIO) for the
     tile UVLO reset FET.
   - LM5164 3V3 and the isolated 12 V aux supply, both ahead of the
     tile hot-swap.
   - Link connector (isolated I2C + EXP_INT) and 12 V aux connector.
2. **Slot column** (x4, identical)
   - Front end: LM5069-2, 3.5 mOhm shunt, BSC070N10NS5, INA238,
     UVLO/EN chain with OV latch and the expander's open drain.
   - TCA9535: EN, latch clear, PGD, PGOOD, latch state, PWR/RST
     photoMOS drive, LED sense; INT to INT_n.
   - Module site: 4 standoffs, the aux socket, module outline on silk.
   - ID: 10k pull-up, 100 nF, 1k series to ID_n.
   - Node power connector and front-panel connector at the bottom.
3. **Port module PM-19V-250W** (`power/pm-19v-250w/`, new project
   copied from `power/pm-19v-200w/`)
   - LTC7810 two-phase buck, 250 W; OV comparator and PGOOD
     comparator unchanged in principle; NTC removed; aux pads per
     docs/pm-interface.md.
4. **Bench carrier** (`power/pm-bench-carrier/`): regenerated mount
   from the interface spec; otherwise unchanged.

### Data flow

Unchanged from ADR 0001, with one hop more: the MCU selects a mux
channel before talking to a column's INA238 or expander. At 10-20 Hz
per INA238 and 4 columns this is ~80 mux switches per second, far
inside 400 kHz I2C.

---

## Alternatives Considered

### 2x2 tile
- **The idea**: two rows of two modules, ~100 x 220 mm.
- **Optimizes for**: a narrower board.
- **Sharpest tradeoff**: the top row sits in the warm fluid rising off
  the bottom row; connectors on two edges.
- **Bets on**: the pod envelope being too narrow for 155 mm, or
  natural convection being strong enough that the second row does
  not matter.

### One 8-slot board (ADR 0001)
- **The idea**: keep one large plane with 8 slots and one MCU.
- **Optimizes for**: fewest boards and connectors per box.
- **Sharpest tradeoff**: a big board that cannot be routed as one
  repeated column, and one MCU or input failure takes out 8 nodes.
- **Bets on**: board cost per slot dominating over fault isolation
  and layout effort.

### Unique INA238 / expander addresses per column, no mux
- **The idea**: strap A0/A1 per column, as ADR 0001 did for 8 slots.
- **Optimizes for**: one part less, no mux hop.
- **Sharpest tradeoff**: columns stop being identical copies; one
  stuck device hangs the whole bus.
- **Bets on**: the repeat-layout tooling handling per-copy straps
  cleanly, and bus faults being rare.

### Keep DOSA pin holes for a commercial fallback brick
- **The idea**: plane keeps the eighth-brick hole pattern plus TRIM;
  our screws avoid it.
- **Optimizes for**: a second source for the module.
- **Sharpest tradeoff**: screws and pogos are constrained by holes
  nobody populates; the DOSA pin 3 spot is in our gate drive.
- **Bets on**: our module failing or being late badly enough that a
  24 V-trimmed brick in a soldered slot is worth having.

### Fuses on the plane (ADR 0001)
- **The idea**: keep F1 and F2.
- **Optimizes for**: a dead-simple last line of defence.
- **Sharpest tradeoff**: in fluid the fuse is cooled and blows late or
  not at all, arcs in the fluid, and cannot be replaced in the tank.
- **Bets on**: a fuse rated for immersion existing at 30 A / 60 V.

### Ideal-diode reverse protection at the tile input
- **The idea**: LM74700 class controller + FET in the 30 A path.
- **Optimizes for**: surviving a miswired bus.
- **Sharpest tradeoff**: another FET's loss and area in a 30 A path.
- **Bets on**: bus wiring being done in the field with interchangeable
  connectors.

---

## Consequences

### Positive
- One column layout, copied: the layout work is one slot plus a core.
- A tile is a unit of failure: a dead MCU, input or bus short costs 4
  nodes, not 8.
- Modules swap with a screwdriver in the tank.
- Nothing in the fluid relies on a fuse.
- 250 W modules give ~40 % headroom over a 180 W node.

### Negative
- Two tiles per 8 nodes: two MCUs, two link addresses, two input
  connectors, more cables in the box.
- A mux and an expander per column add parts and an I2C hop.
- PP-48-2S as drawn and routed is not reused as a board; its blocks
  are (front end, bucks, node interface, MCU pinout, tooling).
- No commercial module drops in.

### Risks
- **Aux connector in fluid**: contact resistance of gold-flash
  contacts over time. Mitigation: a gripping, wiping socket contact,
  signals only (no power through it), ID open = slot fault, sample in
  the soak test.
- **Standoff joint relaxing**: thermal cycling loosens the M2 clamp
  on 13-20 A contacts. Mitigation: lock washers, torque spec, check
  the joint resistance on the bench after cycling.
- **Aux pair contact depth is scaled, not dimensioned** (the makers'
  drawings show it but give no number). Mitigation: mate a sample pair
  at the 7.0 mm gap before a volume order.
- **Tile hot-swap SOA at 30 A**: two FETs in parallel share badly in
  linear mode. Mitigation: size one FET for the full SOA, the second
  only for conduction loss; check against the LM5069 power limit.
- **TCA9548A or expander lock-up**: a stuck column bus. Mitigation:
  mux RESET from an MCU GPIO; slots stay ON (default) whatever the
  bus does.

---

## What an Expert Would Ask

**Q: What happens when the tile hot-swap trips while nodes are
running?**
A: Four nodes lose power at once. The core stays up (fed ahead of the
hot-swap), reports the trip, and the controller decides whether to
retry (pull UVLO low and release). Whether the LM5069 latches (-1) or
retries (-2) on its own is still open; latch-off is the safer default
for a short in the fluid.

**Q: The slot latch on PGOOD loss: what stops it tripping at start-up
or on a load step?**
A: Start-up blanking (the latch input is gated until PGOOD has been
high once, or for a fixed time after enable). A load step that drops
VOUT below 17.3 V is a real fault at 19 V. The blanking circuit is not
designed yet.

**Q: Why not put the protection on the module output with an eFuse,
for an exact per-node limit?**
A: It would be better, and it is an optional upgrade (~15 A eFuse for
20-24 V, part not chosen). The slot front end limits input power; the
ALERT threshold on the slot INA238 approximates an output limit within
the converter efficiency. Good enough for a first tile.

**Q: What does an MCU crash do to the mux and expanders?**
A: Nothing safety-relevant: the expander outputs default to
high-impedance at reset, the enable chain's pull-ups hold every slot
ON, and the OV latch does not go through the expander. An expander
that keeps a stale OFF after an MCU watchdog reset is the case to
check: the MCU resets the expanders at boot.

**Q: Is 1 oz NODE copper really enough for 20 A?**
A: For 15-25 mm with pours on both outer layers and stitching, the
drop is a few tens of mV and the heating is small, and the fluid cools
it. It needs a calculation in the column spec, not a guess; 2 oz is
reserved for VBUS along the top.

---

## Implementation Plan

### Decisions you will probably want to tweak
- **Choice**: mux per column with identical addresses.
  **Alternative**: strapped addresses, no mux.
  **Cost to change later**: column schematic and routing, MCU firmware
  bus code. Medium; cheap before the column is routed.
- **Choice**: TCA9535 for slot GPIO.
  **Alternative**: route the slot lines to MCU GPIO (G070 has enough
  pins for 4 columns?).
  **Cost to change later**: core and column interface, high once
  routed.
- **Choice**: 2x3 aux with a spare.
  **Alternative**: 1x5.
  **Cost to change later**: module, plane and carrier footprints;
  low while generated from the spec, high once modules are built.
- **Choice**: Mini-Fit Jr 2x3 for node power.
  **Alternative**: Mini-Fit HCS, or a screw terminal / lug.
  **Cost to change later**: column bottom edge; low before routing.

### Known unknowns and how the plan absorbs them
- Pod envelope: default 1x4 at ~155 x 130 mm. Pivot to 2x2 if the
  box cannot take 155 mm.
- Aux pair mating depth: default 3.1 mm at the 7.0 mm gap (scaled
  from the drawings). Pivot to the 6.6 mm standoff (C2916353) if a
  sample makes poor contact at 7.0.
- LM5069-1 vs -2 at tile level: default -1 (latch). Pivot if field
  experience shows nuisance trips.
- Fluid compatibility of connectors: default Mini-Fit (nylon, tin).
  Pivot on a soak-test failure.

### The mechanical work
- Module: OVLO divider, remove NTC and SDA/SCL, aux header, screw pads,
  250 W rework of FETs, inductors, sense resistors, compensation.
- Generator for mount footprints from docs/pm-interface.md
  (pmmount.py at tag pm-mount-drawn as a start).
- gensch.py: slot block -> column block (mux segment, expander,
  split node connectors), core block (mux, tile hot-swap), tile
  top level.
- Layout: one column, repeat x4, core, tile outline and edges.
- Bench carrier: regenerate mount.

Review asks (answered 2026-10-26):
1. Names: PP-48-4T for the tile, PM-19V-250W for the module. Yes.
2. Tile hot-swap: LM5069-1 latch-off.
3. Module: new project `power/pm-19v-250w/`, copied from
   `power/pm-19v-200w/`, which stays for a few days until it is clear
   nothing in it is needed.
4. INT: INT_n per column to 4 MCU pins (decision 3).
5. Added in review: isolated 12 V aux, controller side never on the
   bus ground (decision 14).

---

## Open Questions

**Architecture-changers**
- [ ] Pod envelope and how tiles sit in the box (confirms 1x4).
- [ ] Slot GPIO through TCA9535 vs direct MCU pins (pin count check
  on the G070 with 4 columns).

**Behavior definers**
- [ ] PGOOD-loss latch start-up blanking: circuit and time.
- [ ] INA238 ALERT threshold (210-220 W input) and whether it latches
  or only reports.
- [ ] Tile hot-swap reset path (LM5069-1 latch-off, decided).
- [ ] D1 SMBJ60A at 60 V standoff vs the new range (SMBJ64A?).
- [ ] OV_KILL at 22.7 V against the node's input rating.
- [ ] Spare aux contact: second GND or something useful.
- [ ] Node connector fluid compatibility (soak test).
