# Checklist before sealing

Go through this with the board in its final position, cabled, on 19 V,
and the lid still open. Every item is something you cannot fix from the
network afterwards, or something that is much cheaper to find now.

## Recorded

- [ ] MAC address and device id of the board.
- [ ] Recovery token, stored with the MAC (console `recovery-token`
      prints it again while USB is still available).
- [ ] Admin password in your password store.
- [ ] HTTPS certificate fingerprint, if you pin it in scripts.
- [ ] Which frame, which pod, which switch port this board is on.
- [ ] A configuration export (Maintenance page or
      `GET /api/v1/config/export`). It contains no secrets; keep the
      MQTT password and API tokens with the admin password.

## Hardware verified

- [ ] Board powers from J3 alone, with USB unplugged.
- [ ] Link LED on, `net` shows the expected address and mode.
- [ ] All eight node cables plugged into the right node connector
      (node 1 on J11 ... node 8 on J18).
- [ ] Every node's LED reading matches reality, on and off.
- [ ] Every node powers on and off from the Status page; `reset`
      visibly restarts a running node.
- [ ] Every probe appears in the probe table with a sensible reading
      and has a name.
- [ ] Dry contacts read closed when their float or switch is operated.
- [ ] `vin_v` matches a meter on the 19 V bus (set the trim if not).
- [ ] Fault word (Modbus input register 13) is 0, or only bit 6 while
      MQTT is still being set up.

## Configuration verified

- [ ] Network: DHCP reservation in place, or static address confirmed
      and the dead-man left enabled.
- [ ] Rebooting the controller (Maintenance page) moved no node. If a
      firmware with the boot policy applied is running, nodes with `on`
      came up staggered.
- [ ] MQTT confirmed, controller rebooted, connected (Status page box,
      retained `status` on the broker), and a command round-trips with
      an ack.
- [ ] Modbus enabled only if needed, with a tight allow-list, and
      readable from the master.
- [ ] Rules reviewed; the leak and over-temperature rules enabled with
      thresholds for your fluid and your probe placement.
- [ ] Fleet recovery key installed, or a deliberate decision not to.
- [ ] An API token created for your automation; the password is not
      embedded in scripts.

## Firmware verified

- [ ] Running image is the release you intend, `factory` and `ota_0`
      hold it, nothing is `pending_verify`.
- [ ] Optional but strongly advised once per site: push an update over
      the air and watch it validate, then push a deliberately broken
      image and watch the board come back on the previous one. See
      [Firmware updates](../operation/firmware-updates.md).

## Physical

- [ ] No adhesive label on the board; identification is on the frame
      or cable tags outside the fluid.
- [ ] Cables are PVC-free and either solid-conductor or potted at the
      gland.
- [ ] Nothing is plugged into the USB-C port.
- [ ] The antenna edge and the RJ45 opening are not pressed against
      metal.

When the lid closes, the paths into the board are HTTPS on 443, MQTT
through the broker, Modbus if enabled, and `POST /recover`.
