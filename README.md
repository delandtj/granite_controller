# granite

The Granite immersion frame electronics: the 48 V power plane, the
plane-core firmware, the controller and the software shared between them.

granite_controller and granite-plane merged on 2026-10-30. granite-plane
was split from granite_controller on 2026-10-29; its commits were replayed
on top of granite_controller's history, so both histories are here.

## Layout

- controller: controller rev D (KiCad project, tools), the live design.
- controller-rev-c: the rev C controller board (relays, expanders, 19 V
  input), kept as demoware. Tag rev-c-demo is the last rev C state of
  the whole repo, firmware included.
- power/pp-48: shared plane tools (schematic generator, placement and
  routing scripts), pp.kicad_sym, pp48.pretty, pp48.3dshapes.
- power/pp-48-core, power/pp-48-unit: the chained plane (ADR 0005), one
  core board and identical 2-slot unit boards.
- power/pm-19v-250w: the port module. power/pm-bench-carrier: bench
  carrier for one module. power/tools: module mount geometry.
- site: installer / operator manual (mdBook). Still describes rev C
  (relays, expanders, 19 V input); to be reworked for rev D and the plane.
- firmware: Rust workspace. granite-core (no_std: dispatcher, commands,
  MQTT / HTTP / Modbus surfaces, rules, config), granite-sim (host
  simulator), granite-fw (ESP32-C6 binary, still the rev C hardware
  layer), web UI.
- docs: ADRs 0001-0005, plane requirements, module interface, controller
  specs, handoff. Firmware ADRs in firmware/docs/adr.
- attic: superseded designs, kept for reference, not maintained.
  - attic/power/pp-48-2s: the first 2-slot plane (ADR 0001 / 0002).
  - attic/power/pm-19v-200w: the first port module (obsolete since
    2026-10-27, replaced by pm-19v-250w).
  - attic/power/pp-48-rev1: the single-board plane (ADR 0004, tag
    pp48-rev1); uses the libraries in power/pp-48.
  Older docs and the handoff name these at their old paths
  (power/pp-48-2s, power/pm-19v-200w, power/pp-48/pp-48.kicad_*).

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
