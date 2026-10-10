# Firmware updates

Updates go over the network into one of two OTA slots; the image
flashed over USB stays in the `factory` slot as the last resort. A new
image must prove itself within a probation window or the bootloader
returns to the previous one. Node power is never touched by an update.

## Two ways to deliver an image

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
you give it. This is the way to update a pod full of boards from one
message each.

```json
{"v":1,"id":"ota-1","action":"ota","target":"all",
 "args":{"url":"https://fw.site.example/granite-fw-0.2.0.bin","sha256":"<64 hex>"}}
```

Progress arrives as `ota` events: `verified` or `failed` when the
image has been written, then `pending_verify`, `valid` or
`rolled_back` from the new image.

## Probation

After the reboot the new image runs as `pending_verify`. It has
`sys.t_validate_s` (default 600 s) to:

1. initialise the relay and sense expanders,
2. get an Ethernet link and an IP address, and
3. connect to the configured MQTT broker, or, if MQTT is disabled,
   serve one authenticated HTTPS request.

Then it marks itself valid. If it fails, panics, trips the task
watchdog or browns out before that, it marks itself invalid and
reboots, and the bootloader starts the previous slot. If that one is
also invalid, `factory` runs.

While pending, the status LED blinks at 4 Hz, fault bit 4 is set, the
Firmware page shows the seconds left, and you can confirm early with
"Mark running image valid" (`POST /api/v1/firmware/mark-valid`) or the
console's `ota-mark-valid`.

Practical consequence for a site with MQTT disabled: after pushing an
update, log in to the page once. That request is the proof.

## Rolling back on purpose

"Roll back to the previous image" (`POST /api/v1/firmware/rollback`)
marks the current image invalid and reboots into the other slot. The
Firmware page lists both slots with their state and version.

## Signed images

Builds can enable application signature verification (ECDSA P-256).
The running firmware then refuses any OTA image that is not signed with
the matching key; nothing is burned into eFuses, so a board can still
be reflashed over USB with anything. The Firmware page and the
console's `status` show whether signatures are required and the key id
of the running image, which matches the ELF SHA-256 printed at flash
time.

The signing key lives outside the boards and outside the repository.
Losing it means no more updates for the boards built with its public
key, and a submerged board cannot be reflashed over USB. Back it up
before the first signed build goes into a tank.

## Recommended procedure for a pod

1. Update one board, wait for `valid`, exercise a node action on it.
2. Update the rest, a few at a time, watching the `ota` events on the
   broker.
3. Keep the previous `.bin` at hand; a pull of the old image with its
   SHA-256 is the cleanest way back if a problem shows up later than
   the probation window.
