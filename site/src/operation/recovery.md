# Recovery

Which path applies depends on what you still have. The first half of
this chapter is the controller: everything there works over the
network, and the USB console is the extra option for a controller on
the bench. The second half is the planes: what to do when one stops
answering or latches something off, and how to bring back a plane core
on the bench.

## Lost the admin password

With the recovery token or the fleet key, factory-reset the controller
over the network (below), then set a new password. The configuration
is lost; import the export you kept.

With USB access: `set-password <new>` on the console. The
configuration and the existing API tokens stay (the device salt is
kept, not re-minted).

## Lost the network configuration

Wait for the safety nets first:

- A staged change that was never confirmed reverts by itself after the
  confirm window (default 5 min). A `net` change reverts with a reboot;
  `mqtt` and `sec` revert in place, within about 5 s of the window
  passing.
- A static address whose gateway has not answered a ping, **and** which
  has accepted no connection, for the dead-man interval (default 1 h)
  is replaced by a DHCP address. Look for the MAC in the DHCP leases.
  With an empty gateway the dead-man cannot probe and stays disarmed;
  the boot log says so.

If the controller is still unreachable at any address, a factory reset
over `POST /recover` puts it back on DHCP with no password. That
endpoint answers on HTTPS at whatever address the controller has,
including a 169.254.x.x link-local one.

None of this touches the planes: they keep every node as it is while
the controller reboots or is unreachable.

## Factory reset over the network

Two credentials are accepted; either resets the controller. The call is
rate-limited to one attempt per minute and logged as a security event.

**With the per-device recovery token:**

```sh
curl -sk -X POST -d '{"token":"XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"}' https://<addr>/recover
# -> {"v":1,"id":"api-7","ok":true,"result":"accepted","error":null}
```

**With the fleet recovery key**, from the workstation that holds the
private key:

```sh
cd firmware
cargo run -p granite-sim -- recover <addr>
```

The tool fetches a fresh nonce from `/id`, signs
`device || nonce || "factory-reset"` with the key and posts the
signature; nonces expire after 5 minutes and are single use. The
device certificate is not trusted here on purpose (it is self-signed);
the signature is the authentication.

After either call the controller erases its configuration and secrets,
keeps the `factory` namespace (recovery token, fleet key, device key
and certificate), and reboots into first-setup mode on DHCP. The
recovery token stays the same, so your record stays valid, and the
first setup after a reset shows it again. The HTTPS certificate and
its fingerprint also stay the same, so anything pinning it keeps
working. Reconnect, set a password, import the configuration export,
re-enter the MQTT password, recreate API tokens.

The node settings (names, boot policies) are part of the configuration
and go with it; the planes keep their nodes as they are.

## Factory reset with the password

Maintenance page, type the device id to confirm, or:

```sh
curl -sk -H "Authorization: Bearer $T" -X POST -d '{"confirm":"granite-37adc7"}' https://granite-37adc7.local/api/v1/factory-reset
```

Same result as above. On the console: `factory-reset CONFIRM`.

## A controller that does not boot

If an OTA image is bad, the bootloader already handles it: an image
that cannot validate rolls back to the previous one, then to `factory`.
If `factory` itself is broken the controller needs USB, which means
lifting it out. Keep `factory` on a release you trust, and never
overwrite it except on the bench.

## Replacing a controller

1. Export the old controller's configuration if it is still reachable.
2. Flash and commission the new one on the bench (MAC, token,
   password, fleet key).
3. Import the export; the device id and topic base change with the
   MAC unless you override `sys.device_id`, so update the broker ACLs,
   the Home Assistant topics and the Modbus master.
4. Move the 12 V, plane link, probe and Ethernet cables, check that
   every plane is found again, seal.

The node ids come from the plane address straps and the slot, not from
the controller, so they do not change with the controller.

## What the controller logs

Every security event (login, lockout, password set, token created,
fleet key set, recovery accepted or rejected) goes to the log ring,
to the MQTT `event` topic when connected, and to the `log` topic at
or above the configured level.

## A plane that does not answer

The controller asks every plane for its status once a second. A plane
that misses two transactions in a row is declared lost:

- an event `"kind":"plane","what":"plane_lost"` with its address,
- its bit drops out of the plane bitmap (input register 15), and fault
  bit 0 ("a plane stopped answering") goes up and stays up until that
  address answers again; fault bit 7 is set when no plane answers at
  all. Both bits also light the status LED red at 2 Hz,
- its nodes drop out of the state payload, jobs that were running on
  it fail with `link_lost`, and new commands to its nodes are refused
  as absent.

The plane itself carries on: it runs its slots without a controller
and keeps every node as it was. Nothing is switched because the link
went away.

The controller looks for the address again every 10 s. When the plane
answers, `plane_found` follows and its nodes come back. Things to
check in between: the plane link cable (PH 5p), the plane's own power,
and the address switch on the core. When the controller reports
`address_collision`, two planes are set to the same address; only
fixing the switches helps.

**Current firmware:** the controller's board driver for the plane link
is still a placeholder that reports every address absent, so a rev D
controller today shows no planes at all; see
[Flash and first boot](../commissioning/first-boot.md#2-watch-the-first-boot).

## A plane that rebooted

`plane_rebooted` says the plane came back with a new boot count. After
a commanded reboot, a watchdog reset or any reset that kept its supply,
the plane takes over its intended state from its backup registers and
the unit expanders held their outputs through the reset: nodes do not
move.

After a loss of the core's own power the backup registers are gone.
The hardware default then applies: every slot ON. The plane flags
"intended state lost" (`intended_state_lost` in the plane's state),
and the controller re-applies `power_off` to every node it last
commanded rail-off, which shows up as `intended_off_reapplied` with
the node list. The controller keeps that list in its own flash, so
this also works when both rebooted. A node you turned on since is not
in the list and is left alone.

## Latches

Latches are never cleared automatically. Find the cause first, then
clear the latch by command.

| Event | What happened | Clear with |
|---|---|---|
| `ov_latched` | The slot's overvoltage latch cut the node. | `clear_latch` on the node |
| `power_limit_trip` | The slot's power limit tripped its latch. | `clear_latch` on the node |
| `retry_limit_reached` | The slot's hot-swap kept dropping out; the plane set the slot OFF. | `on` or `power_on` on the node |
| `unit_hot_swap_latched_off` | A unit board's input hot-swap faulted and the plane is holding it off; both of its nodes are dark. `latched_off` is set on the unit in the state payload. | `unit_reset` on the unit |

Over the API, with the node id or the unit as the target:

```sh
B=https://granite-37adc7.local
curl -sk -H "Authorization: Bearer $T" -X POST "$B/api/v1/nodes/5/clear_latch?wait=1"
curl -sk -H "Authorization: Bearer $T" -X POST "$B/api/v1/nodes/plane0.unit1/unit_reset?wait=1"
```

The same over MQTT:

```json
{"v":1,"id":"ur-1","action":"unit_reset","target":"plane0.unit1","args":{}}
```

Units are numbered 0-3 in the order of the chain: `plane0.unit0` is
the first unit board on plane 0.

A unit's input hot-swap restarts on its own after a fault, about once
every two seconds, for ever: the limiter has no latch of its own. The
plane is the latch. When it sees the unit's power-good drop for half a
second while the 48 V bus is good, it calls that a unit fault, holds
the unit off and raises `unit_hot_swap_latched_off`. A power-good that
drops because the *bus* sagged or spiked is not a fault and is not
held: the unit comes back by itself when the bus does.

`unit_reset` releases that hold. There is nothing to pulse, so sending
it to a unit that is running costs its two nodes nothing - but it is
still worth knowing why a unit latched before releasing it. The result
comes back about a second later: `done` if the unit carries power
again, `failed` if the fault is still there and the plane latched it
again.

The plane keeps the hold through its own resets. It drops it only when
the plane loses power: the unit then retries until the plane has booted
and seen the fault again. A unit whose plane is dead or unplugged from
the chain retries for ever and is never held off - by design, since
nothing in the fluid should depend on firmware to carry power.

## Rebooting a plane

```sh
curl -sk -H "Authorization: Bearer $T" -X POST "$B/api/v1/nodes/plane0/plane_reboot?wait=1"
```

The plane core restarts and takes over its intended state from the
backup registers, as above; its nodes keep running. Use it when a plane
answers but misbehaves. Over Modbus the plane command word (holding
register `200 + plane`) carries `plane_reboot` and `unit_reset`.

## When the plane core is dead

No firmware is in the plane's safety chain. When the core is not
running at all:

- the unit expanders keep the outputs they last had, so an OFF slot
  stays OFF and a running node keeps running;
- a button press that was in progress ends within about 1 s, because
  the press enable on the units needs the core to keep retriggering
  it;
- a unit board that loses and regains its supply comes back with its
  expander at hardware defaults: every slot ON and its hot-swap ON,
  and a unit the plane was holding off after a fault starts retrying
  again until the plane is back and latches it;
- the overvoltage cut and the current limits are hardware and keep
  working.

So a dead core leaves the nodes powered, but nothing can switch them
until the core runs again.

## Bench recovery of a plane core

**Status:** the plane core firmware (`plane-fw`, with its bootloader
`plane-boot`) is not written yet; only its hardware-free core,
`plane-logic`, exists in the repository. There are no plane images to
flash today, and the procedure below is the one ADR 0002 lays down for
when there are.

A plane core is recovered out of the fluid, on the bench, over SWD.
The core board has two SWD connectors on the same lines (SWDIO,
SWCLK, NRST, 3V3 reference, GND):

- **J5**, Tag-Connect TC2030 pads, for a TC2030 cable without a
  header on the board;
- **J6**, a keyed ARM Cortex 10-pin 1.27 mm header, for the usual
  ribbon of an SWD probe.

Power the core board itself; do not count on the probe to power it.
SWD writes both images: `plane-boot`, which is only ever written over
SWD and never in the field, and `plane-fw`. The plane's address comes
from the switch on the core, not from the image, so a reflashed core
keeps its address.

The STM32's own ROM bootloader is a fallback for the bench only, never
on a link with other planes: every STM32G070 in its ROM bootloader
answers the same fixed I2C address, so two of them collide and one
stuck there blocks the others, and it has no rollback. Take the plane
off the link before using it. How to enter it on the core board is not
documented yet; prefer SWD.
