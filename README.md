# granite-plane

The 48 V power plane and everything that talks to it: the plane boards,
the plane-core firmware, the slimmed controller (rev D) and the software
shared between them.

Split from granite_controller on 2026-10-29 (git filter-repo, history
kept). granite_controller keeps the rev C controller board and its
firmware for demos; it gets archived later. Code from there is reused
here freely.

## Layout

- power/pp-48: shared plane tools (schematic generator, placement and
  routing scripts), pp.kicad_sym, pp48.pretty. Also the frozen rev1
  single-board plane (tag pp48-rev1).
- power/pp-48-core, power/pp-48-unit: the chained plane (ADR 0005), one
  core board and identical 2-slot unit boards.
- firmware: Rust workspace, copied from granite_controller as of the
  split. granite-core (no_std: dispatcher, commands, MQTT / HTTP / Modbus
  surfaces, rules, config), granite-sim (host simulator), granite-fw
  (ESP32-C6 binary, still the rev C hardware layer), web UI.
- docs: plane ADRs 0001-0005, plane requirements, module interface,
  handoff. Firmware ADRs in firmware/docs/adr.

## Direction (decided 2026-10-29)

- Plane core firmware on the STM32G070 (embassy), I2C slave to the
  controller, master to the units through the TCA9548A.
- Plane firmware updated from the controller over the link: own A/B
  bootloader (embassy-boot) that keeps the plane's strapped address,
  rollback on a failed self-test. ST's ROM bootloader only as a bench
  recovery path.
- The plane core owns node sequencing (press deadlines, power-cycle,
  retry / latch policy, stagger); the controller does policy, rules and
  the network.
- One controller serves several planes over isolated I2C (ISO1640 +
  EXP_INT, daisy chain, plane addresses 0x30-0x37).
- One TLV protocol crate used by the plane firmware, the controller and
  the simulator.
- Controller rev D: rev C minus the relays, expanders, LED sense and
  19 V input; 12 V from the plane's isolated aux.
