# Flash a plane core over SWD

A plane core comes from the factory blank. Flash it once on the bench,
out of the fluid, before it goes into a plane. Later images go the same
way until the bootloader `plane-boot` exists (see
[Firmware updates](../operation/firmware-updates.md)).

Not yet tried end to end on a core board: the steps follow the board
files and `firmware/plane-fd/README.md`. Note anything that differs.

## What you need

| Item | Note |
|------|------|
| ST-LINK/V2 (or V3) | the USB-stick clone works |
| SWD breakout, Adafruit 2743 | 10-pin 1.27 mm on one side, 2.54 mm pins on the other |
| 10-pin 1.27 mm SWD ribbon, Adafruit 1675 | core J6 to the breakout |
| or Tag-Connect TC2030-CTX-NL | held on core J5 instead of the ribbon; its 10-pin end goes into the breakout |
| 4 jumper wires, female both ends | breakout to ST-LINK |
| Bench supply and an XT30 lead | 31-48 V, current limit about 0.1 A |
| PC with the toolchain | `rustup target add thumbv6m-none-eabi`, `probe-rs` (see [Building and flashing](../reference/building.md#plane-core-firmware)) |

On Linux, install the probe-rs udev rules once so the ST-LINK is usable
without root (the Arch package ships them).

## The connectors

Both are on the top side of the core, next to the RUN LED, on the same
lines:

| J6 pin (10-pin Cortex) | J5 pad (TC2030) | Signal |
|------------------------|-----------------|--------|
| 1 | 1 | +3V3, the board's own (reference only) |
| 2 | 2 | SWDIO |
| 3, 5, 9 | 5 | GND |
| 4 | 4 | SWCLK |
| 10 | 3 | NRST |
| 6, 7, 8 | 6 | not connected |

The TC2030-CTX-NL cable maps J5 to the Cortex 10-pin layout, so at the
breakout it is the same as the J6 ribbon.

## Steps

1. Wire the breakout to the ST-LINK: **2 to SWDIO, 4 to SWCLK, 3 to
   GND, 10 to RST.** Leave breakout pin 1 open on a clone ST-LINK (its
   3.3 V pin is an output). On a genuine ST-LINK/V2, pin 1 may go to its
   target-voltage sense.
2. Set the supply to 31-48 V, current limit about 0.1 A, **output
   off**. Plug the XT30 into core J1, then switch the output on. The
   core has no inrush limiting: a live plug sparks, the supply's
   current limit does not stop it.
3. The **3V3** and **12V** LEDs on the underside light. If not, switch
   off and check the supply. Both regulators turn on at about 30.2-30.3
   V typical, up to about 31.8 V worst case; if a core does not start at
   31 V, raise the voltage (bench fallback in `docs/handoff.md`,
   2026-11-23).
4. Connect the probe: the ribbon into J6 and the breakout, pin 1 (red
   stripe) to pin 1 at both ends. Or press the TC2030-CTX-NL onto J5,
   its three pins in the three holes, and hold it flat and still until
   the flash is done.
5. Plug the ST-LINK into the PC. `probe-rs list` must show it.
6. Flash the default build (`link-mcp`, STM32G070CB with the
   MCP2518FD, the first batch):

   ```sh
   cd firmware/plane-fd
   cargo run --release      # = probe-rs run --chip STM32G070CBTx
   ```

   It builds, erases, flashes and then prints the core's log over RTT.
7. The **RUN** LED on top blinks. Stop the log with Ctrl+C: the image
   stays on the core.
8. Switch the supply off, then unplug. Mark the core as flashed with a
   tie-on tag (no adhesive labels, see
   [Immersion constraints](../planning/immersion.md)).

The plane address comes from SW1 on the core, not from the image, so a
reflash keeps it.

## If it does not work

| Sign | Try |
|------|-----|
| `probe-rs list` shows nothing | another USB port or cable; the udev rules |
| Probe found, cannot attach | supply on and both LEDs lit? Check the four wires. Then: `probe-rs run --chip STM32G070CBTx --connect-under-reset target/thumbv6m-none-eabi/release/plane-fd` |
| Flashes, RUN does not blink | keep the log and see [Recovery](../operation/recovery.md#bench-recovery-of-a-plane-core) |

Units have no LEDs. Their logic runs on 3.3 V from the core over the
chain, and their slots only switch on above 41.3 V, so on a 31 V bench
supply a unit shows nothing.
