# Checklist before sealing

Go through this with the controller and the planes in their final
positions, cabled, on the 48 V bus, and the lid still open. Every item
is something you cannot fix from the network afterwards, or something
that is much cheaper to find now.

## Recorded

- [ ] MAC address and device id of the controller.
- [ ] Recovery token, stored with the MAC (console `recovery-token`
      prints it again while USB is still available).
- [ ] Admin password in your password store.
- [ ] HTTPS certificate fingerprint, if you pin it in scripts.
- [ ] Which frame, which pod, which switch port the controller is on.
- [ ] Per plane: its address (SW1), its position in the frame, how
      many units it has, and which link chain (J13 or J14) it is on.
- [ ] Per node: its number and the motherboard it powers.
- [ ] A configuration export (Maintenance page or
      `GET /api/v1/config/export`). It contains no secrets; keep the
      MQTT password and API tokens with the admin password.

## Hardware verified

- [ ] Controller powers from J11 alone and from J12 alone (pull the
      other 12 V cable), with USB unplugged.
- [ ] `vin_v` reads about 12 V.
- [ ] Link LED on, `net` shows the expected address and mode.
- [ ] Every plane is found at its address, with the number of units
      you fitted, and no address collision is reported.
- [ ] Every chain cable fully seated and tied; every unit reports its
      input voltage.
- [ ] Every module screwed down with lock washers and reported present.
- [ ] Every node number matches the motherboard you expect: power each
      node off and on in turn from the Status page and watch which one
      reacts.
- [ ] Every node's power LED reading matches reality, on and off.
- [ ] `reset` visibly restarts a running node.
- [ ] Every probe appears in the probe table with a sensible reading
      and has a name.
- [ ] Dry contacts read closed when their float or switch is operated.
- [ ] No fault shown for any plane, unit or node, or only the MQTT one
      while MQTT is still being set up.
- [ ] Bus ground and controller ground are still separate: nothing in
      the tank connects the controller, the Ethernet shield or a probe
      to the bus.

## Configuration verified

- [ ] Network: DHCP reservation in place, or static address confirmed
      and the dead-man left enabled.
- [ ] Rebooting the controller (Maintenance page) moved no node.
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

- [ ] Controller: running image is the release you intend, `factory`
      and `ota_0` hold it, nothing is `pending_verify`.
- [ ] Planes: every core runs the plane firmware release you intend.
      (**Current firmware**: the plane core firmware is not written yet
      at the time of writing; until it is, this item cannot be done and
      the planes run on their hardware defaults.)
- [ ] Optional but strongly advised once per site: push a controller
      update over the air and watch it validate, then push a
      deliberately broken image and watch the board come back on the
      previous one. See [Firmware updates](../operation/firmware-updates.md).

## Physical

- [ ] No adhesive label on any board; identification is on the frame
      or cable tags outside the fluid.
- [ ] Cables are PVC-free and either solid-conductor or potted at the
      gland.
- [ ] Nothing is plugged into the controller's USB-C port or a plane
      core's SWD connector.
- [ ] The controller's antenna edge and RJ45 opening are not pressed
      against metal.
- [ ] The branch breaker for the bus is outside the fluid and labelled
      with the planes it feeds.

When the lid closes, the paths into the system are HTTPS on 443, MQTT
through the broker, Modbus if enabled, and `POST /recover`, all on the
controller. The planes are reached only through it.
