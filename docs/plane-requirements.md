# Power plane tile and port module - requirements

**Date**: 2026-10-26
**Scope**: the power plane (tile of 4 slots, PP-48-4T) and the port
module (PM-19V-250W, update of PM-19V-200W). Decisions on how these
are met: docs/adr/0003-plane-tile-and-module-interface.md. Module
mechanics and aux contacts: docs/pm-interface.md.

These are written down before any more layout. The 2026-10-26 churn
(mount -> aux -> screw positions -> module interface -> columns ->
I2C -> protection -> power class) came from finding them in the
middle of drawing.

## Load

- R1. Node: Halo Gorgon (Strix Halo) motherboard, 19 V DC input,
  ~180 W peak, lower average. One node per slot.
- R2. Module output 19.0 V +/-2 %, 250 W continuous (13.2 A), current
  limit 18-20 A (cannot be set lower: boost and turn-on peaks).
- R3. Overvoltage on the node is the one fault that destroys a node:
  the cut must be hardware, independent of the converter's own
  controller and of any MCU.

## Input

- R4. Bus 48-60 V nominal range. UVLO ~38 V. OVLO ~66-68 V, the plane
  (tile and slot front ends) trips before the module.
- R5. All parts on the bus rated 100 V; surge clamping keeps bus
  transients under 100 V. Slot clamp standoff must not conduct at
  60 V.
- R6. Reverse polarity: prevented by a keyed tile input connector (no
  silicon reverse block).

## Scale

- R7. 4 slots per tile, one MCU per tile. 8 slots = two tiles.
- R8. A box holds 6x8 or 8x8 slots: 12-16 tiles, 13-16 kW. Tile
  input ~22 A at 48 V, ~28 A at 38 V: 30 A class.
- R9. All slot columns on a tile are identical copies (one column
  routed, then repeated).

## Immersion

- R10. Tiles and modules are submerged in dielectric fluid. No fans,
  no heatsinks, no wet electrolytic capacitors.
- R11. No fuses in the fluid (rated in air: they blow late or not at
  all when cooled, arc in the fluid, and cannot be replaced in the
  tank). Overcurrent is handled by electronic limiters; the only
  fuse or breaker sits outside the fluid, per box or branch.
- R12. Connector housings, plating and cable insulation must be
  compatible with the fluid (part of the soak test).
- R13. Module temperature is the fluid temperature: no NTC on the
  module. Per-slot temperature from the INA238 die. Fluid
  temperature from the controller's own 1-Wire probes (not through
  the tile link).

## Behaviour with the MCU dead

- R14. Every slot defaults ON in hardware: an MCU in reset,
  unpowered or crashed leaves all slots running (ADR 0001 safety
  rules carry over unchanged).
- R15. OV latch, slot current limit, tile current limit and module
  current limit work without the MCU.
- R16. The tile core (MCU, mux, expanders, link, 3V3) is fed from
  ahead of the tile hot-swap: a tripped tile must not take down the
  MCU that resets it. The 12 V aux output for the controller is fed
  the same way.

## Serviceability

- R17. A module is swapped in the tank without soldering: screwed
  down, aux contacts by pogo pins. Soldered joints on a board that
  has been in fluid need cleaning before rework.
- R18. A failed slot is isolated (latched off) without affecting the
  others on the tile; a failed tile without affecting other tiles.
- R19. Module presence and type are readable (ID resistor, open = no
  module).

## Interfaces

- R20. Controller to tile: isolated I2C, TLV message format of ADR
  0001, unchanged.
- R21. Node connector: power and front-panel signals split. Power 3
  contacts per polarity (Mini-Fit Jr 2x3 or HCS class), 14 AWG.
  Front panel (PWR_SW, RST_SW, PLED) on a small separate latching
  connector.
- R22. Commercial eighth bricks do not need to fit a slot. The module
  keeps the 58.4 x 22.9 mm outline only.

## Out of scope

- Mains (220 V) modules, loads that are not a 19 V node, PoE.
- Energy metering on the module (INA228 on the plane stays an
  optional drop-in for the INA238).
