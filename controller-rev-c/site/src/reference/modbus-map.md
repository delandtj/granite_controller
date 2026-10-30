# Modbus register map

Generated from the firmware (`firmware/docs/modbus_map.md`, map
version 1). Addresses are zero-based protocol addresses; a client that
counts from 1 (as `mbpoll` does by default) adds one. Enable the server
and its allow-list as described in [Modbus TCP](../commissioning/modbus.md).

All seven fault bits below are maintained by the firmware. Bit 0 is set
when the expanders fail to initialise and, at runtime, by a blown press
deadline or an expander readback mismatch; it clears again on the next
clean one-second readback. Bit 2 needs a probe to be *configured*: a
probe found by the boot scan and then unplugged does not set it, a ROM
id in the `nodes` section that stops answering does. Bit 3 uses the
window `nodes.vin_min_mv`..`nodes.vin_max_mv` (16 V to 22 V by default,
either bound at `0` to disable that side). Bit 5 stays set until the
section that fell back at boot is written again.

{{#include ../../../firmware/docs/modbus_map.md:3:}}
