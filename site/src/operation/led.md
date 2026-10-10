# Status LED

One LED on the board (GPIO1) and the two LEDs in the Ethernet jack.
None of them is a reliable indicator through fluid and a lid; they are
for the bench and for a board lifted out for service. The same
information is on the Status page, in the `status` and `state` MQTT
payloads and in the Modbus fault word.

| Pattern | Meaning |
|---|---|
| 1 Hz blink | Healthy heartbeat. |
| 0.25 Hz blink (slow) | No Ethernet link. |
| 4 Hz blink (fast) | A firmware image is on probation, waiting to validate itself. |
| Solid on | A button press is in progress. |
| Off | No power, or the firmware has not started. |

On the Ethernet jack, green is link and yellow is activity, driven by
the W5500 directly.

Development boards built with the `rgb-led` feature add a colour LED
that mirrors the same patterns (green heartbeat, amber no link, blue
probation, white press) plus red at 2 Hz for an expander or sense
fault. Controller boards have no such LED; a fault there is visible in
the fault word and the log.
