# Hardware ask: a press deadline line in the chain

**For**: the plane hardware (power/pp-48-core, power/pp-48-unit, chain.py)
**From**: firmware ADR 0002 (firmware/docs/adr/0002-plane-core-firmware.md)
**Date**: 2026-10-29
**Status**: decided 2026-10-29 (user): option B + option A on a 14-pin chain (S14B-PH-K-S), core deadline = 74LVC1G123; drawn in chain.py / unit.py / mcu.py, ADR 0005 decision 4

## Problem

The PWR and RST buttons of a node are pressed by a photoMOS (U6 / U7 per
slot, TLP176AM), whose LED is driven by the unit expander's PWRD / RSTD
outputs through R19 / R20 (270 Ohm, LED cathode to GND).

The TCA6424A keeps its outputs until the unit loses 3V3. Its /RESET
only has the R30 pull-up. A core MCU that crashes or hangs during a
press leaves the button pressed. A held PWR button forces the node
off after ~4 s, the same as a power cut, and the node stays off.

The firmware bounds this with the watchdog. The core releases every
press on each boot, before anything else, so a crash holds a button
for ~0.6 s at most. That only works if the watchdog fires. A hang that
keeps the watchdog fed, or a firmware bug in the press timer, holds the
button with nothing in hardware to stop it. Rev C had a hardware
deadline for exactly this case: the expander was held in reset except
during a press. The plane has no equivalent.

## What the line has to do

- No press is possible unless the core MCU is alive and actively
  allows it.
- Within about 1 s of the MCU stopping (crash, hang, reset, core
  unplugged, chain cable pulled) every photoMOS LED goes dark.
- Slot OFF, RUNOFF, DISARM and U_OFF are NOT touched. An operator's
  OFF must survive a firmware hang.
- Default with no core at all: no presses. That is the safe state; the
  button is never needed for the node to run.

## Option A: expander reset line (EXP_RSTN)

One chain pin from a core GPIO (open drain) to every unit's TCA6424A
/RESET (R30 stays as the pull-up on each unit).

- Simple: one pin and one trace per board.
- It resets the whole expander, so a stuck press can only be cleared by
  also dropping every slot's OFF (every slot goes ON on every unit).
- It is an action the firmware must take, not a deadline. A hung MCU
  does not pull it.

Useful as a recovery tool (a wedged expander), not as the deadline.

## Option B: press enable (PRESS_EN), recommended

One chain pin, PRESS_EN, high only while the core MCU keeps it alive.

- **Unit side**: the cathodes of the four photoMOS LEDs (U6, U7, U56,
  U57: pin 3, today on GND) go to a common PRESS_RTN, switched to GND
  by one small N-FET (2N7002 class) whose gate is PRESS_EN, with a
  100k pull-down on the unit. PRESS_EN low or open: no LED current, so
  no press, whatever the expander drives. The photoMOS outputs (pin 4 /
  6) are unchanged. Current through the FET is <= 4 x ~7 mA.
- **Core side**: PRESS_EN comes from a deadline circuit that a static
  GPIO level cannot hold up. Either a GPIO toggling into an AC-coupled
  charge pump (cap + two diodes + RC on the gate, ~0.5-1 s decay), or a
  retriggerable monostable (74LVC1G123 class) clocked by the GPIO. A
  stuck-high or stuck-low pin both end in PRESS_EN low.
- One line for all units is enough. The firmware only toggles while a
  press runs (or always, slot granularity is the expander's job).
- Free core pins since the tile moved out: PC6, PC7 (were TILE_OFF /
  TILE_PGD), PB13 (TILE_AL). A timer channel on one of them can produce
  the toggle in hardware.
- As built (2026-10-30): PRESS_TRIG on PB12, a plain GPIO the press task
  toggles (firmware ADR 0002); a timer would keep toggling through a
  firmware hang. The core pins were reassigned for the fewest crossings
  (tools/sch/blocks/mcu.py CORE_PINS).

## Chain pin

The chain is full: 12 pins = P0..P3 SDA / SCL, 3V3, GND x 3 (ADR 0005
decision 4, chain.py). Two ways to get a pin:

1. Take one of the three GND pins. That leaves two GND, and one SDA /
   SCL pair loses the GND next to it (ADR 0005 risk: "keep the GND pins
   between the pairs"). If PRESS_EN goes between two pairs it is a slow
   static line and works as a guard almost as well as GND. The 3V3
   return then shares two pins (~0.2 A, fine).
2. Go to 14 pins (S14B-PH-K-S class, +4 mm along the edge, LCSC stock
   to check) and keep three GND. That leaves one pin spare for
   EXP_RSTN (option A) too.

## Firmware impact

Small either way. With option B the press path becomes "set PRESS_EN
alive, drive PWRD, wait, release PWRD, stop PRESS_EN", and the boot rule
(release every press first) stays. With option A the firmware gets a
"reset unit expanders" action for a wedged bus. ADR 0002 records the
choice once the hardware takes it.
