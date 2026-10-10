# Modbus register map

Generated from the firmware (`firmware/docs/modbus_map.md`, map
version 1). Addresses are zero-based protocol addresses; a client that
counts from 1 (as `mbpoll` does by default) adds one. Enable the server
and its allow-list as described in [Modbus TCP](../commissioning/modbus.md).

**Current firmware:** of the fault bits below, bit 0 and bit 1 are set
when the expanders fail to initialise, bit 1 when a sense read fails
at runtime, bit 4 while an OTA image is pending and bit 6 while MQTT is
enabled but not connected. Bits 2, 3 and 5 are defined but not yet
set by anything.

{{#include ../../../firmware/docs/modbus_map.md:3:}}
