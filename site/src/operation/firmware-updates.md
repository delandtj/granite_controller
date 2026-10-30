# Firmware updates

There are two firmwares in a frame: the controller's and each plane
core's. The controller is updated over the network into one of two OTA
slots; the image flashed over USB stays in the `factory` slot as the
last resort. A plane core is updated by the controller over the plane
link. Both run a new image on probation, and an image that does not
prove itself is rolled back.

## Controller: two ways to deliver an image

**Push** from the Firmware page or a script: upload the application
image (a `.bin`, not the ELF) over HTTPS. It streams straight into the
inactive slot.

```sh
curl -sk -H "Authorization: Bearer $T" -X POST \
  --data-binary @granite-fw-0.2.0.bin https://granite-37adc7.local/api/v1/firmware/upload
# -> {"ok":true,"slot":"ota_1","bytes":1719488,"reboot_required":true}
curl -sk -H "Authorization: Bearer $T" -X POST https://granite-37adc7.local/api/v1/reboot
```

**Pull** by command: the controller fetches the image itself over
HTTPS, verifying the server with the MQTT CA, and checks the SHA-256
you give it. This is the way to update a site full of controllers from
one message each.

```json
{"v":1,"id":"ota-1","action":"ota","target":"all",
 "args":{"url":"https://fw.site.example/granite-fw-0.2.0.bin","sha256":"<64 hex>"}}
```

Progress arrives as `ota` events: `started`, then `downloading` with a
percentage every 10 % of a known image size (every 256 KiB when the
server sends no Content-Length), `verified` or `failed` when the image
has been written, then `pending_verify`, `valid` or `rolled_back` from
the new image.

Making the `.bin` from a build is in
[Building and flashing](../reference/building.md#producing-an-ota-image).

## Controller: what the reboot does to the nodes

Nothing by itself. The planes run their slots on their own and keep
every node as it is while the controller restarts. The only node
commands a controller boot sends are the boot policies: a node set to
`on` or `off` gets that command about three seconds after every
controller boot, a node on the default `leave` is not touched. See
[Nodes and boot policy](../commissioning/nodes.md).

## Controller: probation

After the reboot the new image runs as `pending_verify`. It has
`sys.t_validate_s` (default 600 s) to:

1. get an Ethernet link and an IP address, and
2. connect to the configured MQTT broker, or serve one authenticated
   HTTPS request.

Then it marks itself valid. The planes are deliberately not part of
this: a controller whose planes are unplugged must still be able to
take an update. If the image fails, panics, trips the task watchdog or
browns out before that, it is marked invalid and the board reboots,
and the bootloader starts the previous slot. If that one is also
invalid, `factory` runs. An image that hangs too early to roll itself
back is reset by the bootloader's RTC watchdog, which stays armed for
an image on probation, and the bootloader rolls it back the same way.

While pending, the status LED blinks at 4 Hz, fault bit 4 is set, the
Firmware page shows the seconds left, and you can confirm early with
"Mark running image valid" (`POST /api/v1/firmware/mark-valid`) or the
console's `ota-mark-valid`.

Practical consequence for a site with MQTT disabled: after pushing an
update, log in to the page once. That request is the proof.

## Controller: rolling back on purpose

"Roll back to the previous image" (`POST /api/v1/firmware/rollback`)
marks the current image invalid and reboots into the other slot. The
Firmware page lists both slots with their state and version.

## Controller: signed images

Builds can enable application signature verification (ECDSA P-256).
The running firmware then refuses any OTA image that is not signed with
the matching key; nothing is burned into eFuses, so a board can still
be reflashed over USB with anything. The Firmware page and the
console's `status` show whether signatures are required and the key id
of the running image, which matches the ELF SHA-256 printed at flash
time.

The signing key lives outside the boards and outside the repository.
Losing it means no more updates for the controllers built with its
public key, and a submerged controller cannot be reflashed over USB.
Back it up before the first signed build goes into a tank.

## Plane firmware updates

**Status: not usable in the field yet.** The controller side of the
update is written and tested against the simulated plane, but three
pieces around it do not exist:

- the plane core firmware itself (`plane-fw` and its bootloader
  `plane-boot`); only its hardware-free core, `plane-logic`, is
  written. Plane cores are flashed over SWD on the bench until then
  (see [Recovery](recovery.md#bench-recovery-of-a-plane-core));
- a way to hand the controller a plane image: the controller's OTA
  image does not carry one yet, and there is no API route or command
  that uploads a plane image or starts a plane update;
- the board driver for the plane link (the current controller image
  finds no planes at all, see
  [Flash and first boot](../commissioning/first-boot.md#2-watch-the-first-boot)).

What follows is how the update is built, so that it behaves this way
once those pieces arrive.

### The policy

`nodes.plane_update` in the configuration, `manual` (the default) or
`auto`. `GET /api/v1/planes` reports it as `update_policy`.

- `manual`: an update starts only when something asks for it.
- `auto`: the controller offers its plane image to every plane whose
  firmware version differs from it, one plane at a time, after the
  controller's own image is confirmed. In the current firmware the
  setting is stored and handed to the plane client, but nothing acts on
  it, because there is no image to offer.

### One update, step by step

The controller updates one plane at a time and refuses to start
(`busy`, `plane_absent`, `sequence_running`, `commands_in_flight`,
`bad_image`, `not_capable`) when another update runs, the plane is not
there, a node sequence runs on that plane, commands to it are still in
flight, the image is empty, or the plane does not report the firmware
update capability.

1. **Begin**: size, CRC-32 and version go to the plane, which erases
   its update slot.
2. **Data**: 128-byte records, paced by the offset the plane asks for
   next. A plane image is around 55 KB, about 10 s at the link's
   100 kHz. The transfer gives up after 5 minutes.
3. **Commit**: the plane checks the CRC-32 over the slot and marks the
   swap (30 s allowed).
4. **Reboot**: the plane's bootloader swaps the images. The unit
   expanders hold their outputs through the reset, so the nodes do not
   move.
5. **Back**: the controller waits up to 30 s for the plane to answer
   INFO with a new boot count.
6. **Confirm**: only when the plane shows the same unit boards it had
   before the update does the controller send the confirm, then waits
   up to 10 s for the plane to report its image confirmed.

Anything that goes wrong ends the run without a confirm. That is the
safe direction: a plane image that is not confirmed within 120 s of
booting resets and its bootloader swaps the previous image back, so a
controller that dies in the middle of an update still leaves a working
plane.

### What you see

- `.../event` with `"kind":"plane"` and `what` set to
  `update_confirmed` (detail: the new version) or `update_failed`
  (detail: `<version> at <reason>`, the reason one of `image`,
  `transfer`, `commit`, `no_return`, `units_changed`, `rolled_back`,
  `not_confirmed`, `aborted`).
- The plane's own events in the same form: `update_state_changed`,
  `image_confirmed`, `rolled_back`.
- In `state.planes[]` and `GET /api/v1/planes`: `firmware` (the running
  version) and `image_state`, which is `confirmed`, `probation` or
  `rolled_back`.

## Recommended procedure for a site

1. Update one controller, wait for `valid`, exercise a node action on
   it.
2. Update the rest, a few at a time, watching the `ota` events on the
   broker.
3. Keep the previous `.bin` at hand; a pull of the old image with its
   SHA-256 is the cleanest way back if a problem shows up later than
   the probation window.
