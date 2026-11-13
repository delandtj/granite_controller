# Block switch 12 V supply: its own isolated converter off the bus

**Status**: Proposed
**Date**: 2026-11-13

Related: docs/adr/0003-plane-tile-and-module-interface.md (decision 8:
no fuses in the fluid, bus TVS per board; decision 9: input window
48-60 V; decision 14: the ground boundary), docs/adr/0005-chained-plane-boards.md
(decision 6: the core's XT30 input), docs/adr/0006-plane-link-can-fd.md
(decision 4: AUX_12V on the link, a few watts per core),
firmware/docs/switch.md (the controller manages the switch over its
web UI). Scope: one new small board, power/pp-48-sw12. No change to
pp-48-core-fd, pp-48-unit-fd or controller-fd.

---

## Context

Each block has one Ethernet switch (today a Sodola SL902-SWTGW218AS,
8 x 2.5G + 1 SFP+). It runs from 12 V and is budgeted at 30 W max
(2.5 A). On the bench it runs from its wall adapter; in a block there
is only the 48-60 V bus.

The question asked (2026-11-13) was whether the core's 12 V could feed
it. It cannot, as built:

- The core's 12 V is an LM5018 Fly-Buck sized for the controller
  (~2.5 W, ADR 0006 decision 4). 30 W is ten times that.
- Its rectifier D6 (SS110) and OR diode D10 (B5819W) are 1 A parts.
- The 12 V leaves on two pins of the 2x3 link joint and a ribbon:
  about 1 A per conductor.
- The cores diode-OR onto AUX_12V with no sharing: one core would
  carry the whole switch.

Reworking the core for 30 W touches every core in the row for a load
that exists once per block, and puts the switch on the rail the
controller lives on. The user chose a separate supply (2026-11-13).

Constraints carried over from the earlier ADRs:

- Input window 48-60 V nominal, working down to 38 V; 100 V parts on
  the bus side; bus TVS SMDJ60A on every board.
- No fuses in the fluid. Branch protection sits outside; a board
  limits itself.
- Decision 14 of ADR 0003: the outside world (controller, Ethernet,
  USB, probes) never shares the bus ground.
- Parts from LCSC stock.

---

## Decision

1. **A separate board, pp-48-sw12**, takes the bus on its own lead and
   gives 12 V / 30 W to the switch. Nothing on the link, the core or
   the controller changes.
2. **Isolated converter.** A 36-75 V input, 12 V output, 40-50 W
   isolated DC-DC module (functional isolation is enough, as for the
   core's aux supply). The output floats: neither 12V_SW nor its
   return is tied to bus GND or LINK_GND on this board.
3. **Sized with margin**: 40 W minimum module for the 30 W load, so it
   never runs at its limit in a warm box.
4. **Protection without a fuse**: the module's own current limit
   (hiccup) on the output, its UVLO on the input, SMDJ60A on the bus
   side, a 12 V TVS on the output. Reverse polarity by the keyed input
   connector only, as everywhere else.
5. **Remote on/off, default on.** The module's enable is brought to a
   2-pin header through an optocoupler. Nothing connected: the switch
   is powered. Current through the opto LED: the switch is off. A
   power-cycle of a hung switch is then possible from the controller
   side without crossing the ground boundary.
6. **A 12 V good LED** on the output, like the core's.

Why isolated, when the switch's ports are transformer-coupled anyway:
the switch's DC return usually sits on its metal case and its RJ45 /
SFP+ shields. With a non-isolated buck that case is bus GND, and one
shielded patch lead or a DAC cable on the uplink ties bus GND to the
outside world, which is exactly what decision 14 forbids. An isolated
module makes that impossible by construction and costs a few euro
more.

---

## Architecture Overview

### Component Breakdown

1. **Input** (bus side)
   - J1 XT30PW-M, same part and pinout as the core (ADR 0005
     decision 6).
   - D1 SMDJ60A bus TVS; bulk 47 uF / 100 V plus 2.2 uF / 100 V
     ceramics as the module's datasheet asks.
   - Input current: 30 W out at ~90 % is 33 W in: 0.56 A at 60 V,
     0.69 A at 48 V, 0.88 A at 38 V.
2. **Converter**
   - U1: isolated module, 36-75 V in, 12 V out, >= 3.3 A (40 W) with
     remote on/off, 1 x 1 inch or 1/16 brick class. Example families
     to pick from by LCSC stock at layout time: Mornsun URB48xx /
     VRB48xx 40-50 W, Mean Well SKM / RSDW class. The exact part is
     picked at schematic time; the footprint follows the part.
   - Dissipation at full load ~3.3 W in the module.
3. **Output** (switch side, floating)
   - 12V_SW, RTN_SW. Output caps per the module's datasheet (order of
     2 x 22 uF / 25 V), TVS SMAJ15A, LED + resistor.
   - J2: 2-pin locking power connector rated >= 5 A (Micro-Fit 3.0 2p
     proposed), with a lead ending in the switch's DC barrel plug.
4. **Enable**
   - J3: 2-pin header, EN+ / EN-, to an optocoupler (LTV-817 class)
     whose transistor pulls the module's on/off pin to the off state.
     Series resistor sized for 3.3 V drive at ~3 mA. Open header = on.
     The opto is the only thing that crosses from the outside world;
     it keeps the boundary.

### Data Flow / Interaction

```
 bus 48-60 V --XT30--> [TVS, caps] --> [isolated 48->12 V, 40-50 W] --> 12V_SW --> switch
   (bus GND)                              ^ on/off (bus side)          (floating)
                                          |
 controller side: EN+ / EN- --> [opto] ---+        (default: open = on)
```

The controller keeps managing the switch over Ethernet
(firmware/docs/switch.md). The enable input is an addition for the
case where the switch no longer answers.

---

## Alternatives Considered

### Feed it from the core's AUX_12V
- **The idea**: grow the core's 12 V to 30 W and tap the link joint.
- **Optimizes for**: no extra board, no extra bus lead.
- **Sharpest tradeoff**: a new converter, diodes and a power connector
  on every core; the switch shares the controller's rail.
- **Bets on**: the core having board space and the row needing one
  more 30 W load per core. It needs one per block.

### Non-isolated buck on its own board
- **The idea**: the same board with a 48 -> 12 V buck (LM5164-class is
  too small; a 3 A / 100 V controller plus FET).
- **Optimizes for**: cost and efficiency.
- **Sharpest tradeoff**: the switch case and shields become bus GND.
- **Bets on**: nobody ever plugging a shielded lead or a DAC cable
  into the switch. That holds on the bench and fails in the field.

### PoE or the switch's adapter on mains
- **The idea**: keep the wall adapter, or a PoE splitter.
- **Optimizes for**: zero design work.
- **Sharpest tradeoff**: a mains device inside or beside the block; a
  second power path the bus monitoring never sees.
- **Bets on**: mains being present and acceptable at the block.

---

## Consequences

### Positive
- Core, unit and controller boards stay as fabbed (fd-batch1).
- A fault in the switch supply cannot take the controller's rail down.
- The ground boundary of ADR 0003 holds whatever is plugged in.
- A hung switch can be power-cycled without opening the box.

### Negative
- One more board and one more XT30 lead on the bus per block.
- The switch's draw is not measured: there is no INA on this board.
- ~3 W of converter loss.

### Risks
- The chosen module's remote on/off polarity and reference pin decide
  the opto circuit: check the datasheet before drawing it.
- A module in fluid: check the datasheet's cooling assumption (most
  are rated in still air, fluid is kinder, but potting compatibility
  with the fluid is the open point).
- 30 W is the user's figure, not a measured one. Measure the switch
  with all ports up and the SFP+ fitted before fixing the module size.

---

## What an Expert Would Ask

**Q: What happens on hot-plugging the XT30 at 60 V?**
A: The input caps charge through the lead inductance and ring; the
SMDJ60A and 100 V caps take it, the same as on the core, which has the
same input. No inrush limiter is planned for ~50 uF.

**Q: What drives the enable input? controller-fd has no spare output.**
A: Nothing, in fd-batch1: the header is there and left open. The only
free C6 pin on controller-fd is GPIO8, a strapping pin, not suitable.
A real driver needs a controller revision or an I2C expander on J9.
Until then the power-cycle is by hand at the header.

**Q: The output floats. Will the switch's case sit at an undefined
potential?**
A: It is defined by whatever the switch is cabled to (shield, DAC,
earth lug). With unshielded leads and fibre it floats, held by the
module's stray capacitance. If that proves noisy, a 1 Mohm / 1 nF
bleed from RTN_SW to LINK_GND is the fix, and it keeps the boundary.
Not fitted by default.

**Q: Why not measure the 12 V current?**
A: Nothing would read it: the board has no MCU and no link. The switch
reports its own state over Ethernet. If a reading is wanted later, the
bus branch outside the fluid is the place.

**Q: One switch per block is a single point for the block's network.
Does this board make that worse?**
A: It adds one converter in series with the switch. It does not change
the single point; a second switch would bring its own supply.

---

## Implementation Plan

### Decisions you will probably want to tweak

- **Isolated vs non-isolated**
  - Choice: isolated module.
  - Alternative: non-isolated buck.
  - Cost to change later: a new board; the footprints do not overlap.
- **Output connector**
  - Choice: Micro-Fit 3.0 2p plus a barrel lead.
  - Alternative: XT30 (not keyed against the input one), or a barrel
    jack on the board.
  - Cost to change later: one footprint, one lead.
- **Enable sense**
  - Choice: open = on, current = off.
  - Alternative: current = on (fails off when the controller is gone).
  - Cost to change later: one opto circuit; the wrong choice here
    means a dead controller takes the network down, hence open = on.
- **Module power class**
  - Choice: 40-50 W.
  - Alternative: 30 W (smaller, no margin).
  - Cost to change later: footprint, unless a family with one
    footprint across 30-50 W is picked.

### Known unknowns and how the plan absorbs them

- **Real switch power**: default 30 W. Pivot if the measured draw with
  all ports up exceeds ~28 W (then 50 W class) or stays under 12 W
  (then a smaller module is fine).
- **Is the switch itself in the fluid?** Default: the supply board is
  built to the fluid rules either way (no fuse, no electrolytics that
  dislike the fluid). Pivot only if the switch ends up outside the
  box: then the board can sit outside too and the rules relax.
- **Mounting**: default four M3 holes, placement left to the
  mechanical design of the block.

### The mechanical work

- power/pp-48-sw12: schematic (one sheet), 2-layer board, about
  50 x 40 mm set by the module; pcbkit.toml like the other pp-48
  boards; JLC outputs.
- Functional isolation gap between the bus side and the output side as
  on core-fd (0.5 mm outer layers at <= 60 V), wider where free.
- Lead: Micro-Fit 2p to the switch's barrel plug (measure the plug).
- Bench test: 38 / 48 / 60 V in, 0 / 1 / 2.5 A out, enable on/off,
  short on the output, hot-plug at 60 V.

Review asks:

1. Isolated module, as argued above: yes / no?
2. Enable input, open = on: yes / leave the enable out entirely?
3. Output on Micro-Fit 2p with a barrel lead: yes / other connector?
4. Is the switch inside the fluid: yes / no?

---

## Open Questions

**Architecture-changers**
- [ ] Is the switch immersed with the block, or outside it?
- [ ] Does anything on the switch side get earthed (uplink DAC, earth
      lug)? It decides whether the floating output needs the bleed.

**Behavior definers**
- [ ] Measured switch power, all ports up.
- [ ] Who drives the enable input, and from which controller revision.
