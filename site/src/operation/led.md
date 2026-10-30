# Status LED

## Controller

One LED on the board (GPIO1) and the two LEDs in the Ethernet jack.
None of them is a reliable indicator through fluid and a lid; they are
for the bench and for a controller lifted out for service. The same
information is on the Status page, in the `status` and `state` MQTT
payloads and in the Modbus fault word.

| Pattern | Meaning |
|---|---|
| 1 Hz blink | Healthy heartbeat. |
| 0.25 Hz blink (slow) | No Ethernet link. |
| 4 Hz blink (fast) | A firmware image is on probation, waiting to validate itself. |
| Off | No power, or the firmware has not started. |

Probation wins over no link: an image on probation blinks at 4 Hz
whether or not the cable is in. The LED says nothing about the planes;
a plane that stopped answering or a node in fault shows in the fault
word (bits 0, 1 and 7) and as an event.

On the Ethernet jack, green is link and yellow is activity, driven by
the W5500 directly.

Development boards built with the `rgb-led` feature add a colour LED
that mirrors the same patterns (green heartbeat, amber no link, blue
probation) plus red at 2 Hz when a plane stopped answering, a node is
in fault or no plane answers at all. Controller boards have no such
LED. On a bare devkit, which has no planes, the colour LED therefore
sits on red; that is the honest reading.

## Plane core

The plane core has one LED, LED_RUN (PB14). The plane firmware logic
defines three patterns for it:

| Pattern | Meaning |
|---|---|
| 1 Hz blink | The plane is running. |
| Fast blink | A firmware update is being received. |
| Double blink | The running image is unconfirmed (on probation after an update). |

The plane core firmware that drives the pin is not written yet (see
[Recovery](recovery.md#bench-recovery-of-a-plane-core)), so the exact
rates of the fast and double blink are not fixed, and a core flashed
with nothing shows nothing.
