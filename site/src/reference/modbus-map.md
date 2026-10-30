# Modbus register map

Generated from the firmware (`firmware/docs/modbus_map.md`, map
version 2). Addresses are zero-based protocol addresses; a client that
counts from 1 (as `mbpoll` does by default) adds one. Enable the server
and its allow-list as described in [Modbus TCP](../commissioning/modbus.md).

Version 2 is the map of the rev D controller with power planes. It is
sparse: the controller registers at the bottom (dry inputs, link, MQTT,
probes, board temperature, input voltage, uptime, firmware, fault word,
map version at input register 14) kept their addresses, the per-node
and per-plane blocks are new, and every address not listed answers
exception 2. That includes the version 1 node registers of the rev C
controller, so a client written for version 1 fails loudly instead of
reading a plausible wrong node. Read input register 14 first.

Notes on the fault word (input register 13):

- Bit 0 is set while a plane the controller had **found** is not
  answering, and clears when that address answers again. A controller
  that has never seen a plane does not set it: there is nothing to
  miss. Input register 15 (a bit per plane that answers) says which
  plane went away.
- Bit 7 is set while no plane answers at all.
- Bit 1 is set while any present node has a fault flag set (offset 10
  of its block).
- Bit 2 needs a probe to be *configured*: a probe found by the boot
  scan and then unplugged does not set it, a ROM id in the `nodes`
  section that stops answering does.
- Bit 3 uses the window `nodes.vin_min_mv`..`nodes.vin_max_mv` (10 V to
  14 V by default, either bound at `0` to disable that side); the
  shipped rule 3 watches the lower bound as well.
- Bit 5 stays set until the section that fell back at boot is written
  again.

The plane flags (offset 0 of a plane block) are: bit 0 the plane
answers, bit 1 a sequence runs on one of its slots, bit 2 it holds
events the controller has not read yet, bit 3 it says its intended
state is lost (after a power loss; see
[Controlling nodes](../operation/nodes.md)).

A node's energy (offsets 6..9) is in joules since the plane's first
boot, with the boot count it belongs to, so a reader can tell a reset
counter from a real drop.

{{#include ../../../firmware/docs/modbus_map.md:3:}}
