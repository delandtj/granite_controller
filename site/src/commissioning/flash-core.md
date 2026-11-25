# Flash a plane core over SWD

A plane core comes from the factory blank. Flash it once on the bench,
out of the fluid, before it goes into a plane. Later images go the same
way until the bootloader `plane-boot` exists (see
[Firmware updates](../operation/firmware-updates.md)).

First done end to end on 2026-11-25: an EEmaker STLink-V2.1 on J5
through the cut TC2030-CTX-NL, core on 47.8 V. The core booted, found
its MCP2518FD, and the controller saw the plane over the link at once.

## What you need

| Item | Note |
|------|------|
| ST-LINK/V2, V2.1 or V3 | the USB-stick clone and the EEmaker STLink-V2.1 work; check its pin labels (below) |
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
   target-voltage sense. The labels differ per probe:

   | Signal | USB-stick clone | EEmaker STLink-V2.1 (left column) |
   |--------|-----------------|-----------------------------------|
   | SWDIO | SWDIO | SDO |
   | SWCLK | SWCLK | SCK |
   | GND | GND | GND |
   | NRST | RST | RST |
   | 3.3 V | 3.3V, output: leave open | 3V3, output: leave open |

   The EEmaker's right column (5V, 3V3, GND, RX, TX) is a power and
   serial port; nothing goes there. Its jumpers next to the header, if
   fitted, connect the probe to its own target: take them off.
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
5. Plug the ST-LINK into the PC, straight into a port on the PC rather
   than a hub. `probe-rs list` must show it. Note its
   `VID:PID:serial` and pass it with `--probe` from then on, so
   probe-rs does not scan every USB device (on one PC the scan reset a
   USB keyboard).
6. Flash the default build (`link-mcp`, STM32G070CB with the
   MCP2518FD, the first batch):

   ```sh
   cd firmware/plane-fd
   cargo run --release      # = probe-rs run --chip STM32G070CBTx
   ```

   It builds, erases, flashes and then prints the core's log over RTT:

   ```text
   plane-fd at 0x37 (CAN FD, MCP2518FD on SPI1), reset cause 4
   booted, units 0b0000, boot count 2
   MCP2518FD up, node 7
   ```

   The address is SW1's; `units 0b0000` means no unit on the chain.
   With a probe named: `probe-rs run --probe <VID:PID:serial> --chip
   STM32G070CBTx target/thumbv6m-none-eabi/release/plane-fd`.
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
| Probe found, cannot attach (`JtagGetIdcodeError`, "No target responded") | supply on and both LEDs lit? Check the four wires against the probe's own labels, SWDIO and SWCLK not swapped, each one beeped through to J5 / J6. Then: `probe-rs run --chip STM32G070CBTx --connect-under-reset target/thumbv6m-none-eabi/release/plane-fd` |
| Core LEDs light with only the probe attached, no supply | the probe's 3.3 V pin is an output and back-feeds the core's 3V3. Take that wire off, power the core from its XT30 |
| A Nordic nRF DK (on-board J-Link, Debug out) as the probe | not usable: no response from the STM32, and connecting the core reset the kit's J-Link and hung the PC's USB controller. Its Debug out is meant for nRF targets at the kit's I/O voltage |
| Flashes, RUN does not blink | keep the log and see [Recovery](../operation/recovery.md#bench-recovery-of-a-plane-core) |

Units have no LEDs. Their logic runs on 3.3 V from the core over the
chain, and their slots only switch on above 41.3 V, so on a 31 V bench
supply a unit shows nothing.
