# MQTT

MQTT is the intended day-to-day control path: the controller publishes
retained status and state, streams events and logs, and takes commands.
It is off by default. The MQTT page edits the `mqtt` section, which is
**staged** like the network section and, like it, applied live: the
controller connects with the staged settings within about 5 s, so you
can see whether they work before you commit to them. Confirm within the
window (or tick auto-confirm on connect) and the section is stored;
let the window pass and the stored settings come back, also within
about 5 s. Nothing here reboots the controller, and nothing here
touches node power.

## Broker side

Before touching the board:

1. Create a user (or a client certificate) per board, with publish
   rights on `<topic_root>/<site>/<device>/#` and subscribe rights on
   `<topic_root>/<site>/<device>/cmd`. Your automation needs the
   inverse.
2. Have the broker's CA certificate in PEM form. The controller
   verifies the broker with it; there is no "accept any certificate"
   switch. Until SNTP has synced the clock runs from the firmware build
   time, which is what makes the certificate's not-before check pass.
3. Make port 8883 reachable from the controller's subnet.

For a bench, `firmware/tools/mqtt-smoke.sh` starts a throwaway
anonymous mosquitto on 1883 and prints every `granite/#` message.

## Board side

| Field | Default | Notes |
|---|---|---|
| Enabled | off | |
| Host | | Required when enabled. Name or IP. |
| Port | 8883 | 1883 for plaintext. |
| TLS | on | |
| Broker CA (PEM) | | The CA that signed the broker certificate. |
| Username | | Password and client certificate are entered separately and stored as secrets. |
| Client id | `granite-xxxxxx` | Must be unique on the broker. |
| Site | `default` | Second topic level. Use one per pod or room. |
| Topic root | `granite` | First topic level. |
| QoS | 1 | 0..2 |
| Keepalive | 30 s | 5..600 |
| State republish | 60 s | Retained state is re-sent at this interval even without a change. |
| Auto-confirm on connect | off | When set, a session with the staged broker confirms the `mqtt` section by itself. A `net` or `sec` change staged at the same time still needs its own confirm. |

The password or client certificate and key go through a separate
credentials call so they never appear in an export:

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT \
  -d '{"username":"granite-37adc7","password":"..."}' \
  https://granite-37adc7.local/api/v1/security/mqtt-credentials
```

Then stage the section and confirm once the broker session is up:

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT \
  -d '{"enabled":true,"host":"broker.site.example","port":8883,"tls":true,"ca_pem":"-----BEGIN CERTIFICATE-----\n...","username":"granite-37adc7","site":"pod1"}' \
  https://granite-37adc7.local/api/v1/config/mqtt
curl -sk -H "Authorization: Bearer $T" -X POST https://granite-37adc7.local/api/v1/config/confirm
```

Between those two calls the Status page's MQTT box should go to
connected; if it does not, `last_error` in that box and the Maintenance
log say why (CA mismatch and a clock before the certificate's
not-before are the usual ones). Fix the section and PUT it again - the
window restarts - or let it pass and the board goes back to what was
stored.

Nothing on this page needs a reboot. The one commit-confirm change that
still does is the device certificate on the Security page: the HTTPS
server can only pick a new one up at start-up.

## Check it

```sh
mosquitto_sub -h broker.site.example -p 8883 --cafile ca.pem -u ops -P ... -v -t 'granite/pod1/granite-37adc7/#'
```

You should see the retained `status` (`"online":true`) and `state`
messages at once (`state` lists every plane that answers and every
node that is present), then `event` lines as nodes change. Send a
command and watch the ack:

```sh
mosquitto_pub ... -t granite/pod1/granite-37adc7/cmd \
  -m '{"v":1,"id":"t1","action":"on","target":3}'
# -> granite/pod1/granite-37adc7/event  {"v":1,"ts":..,"kind":"accepted","id":"t1","action":"on","target":3,"job":12}
# -> granite/pod1/granite-37adc7/ack/t1 {"v":1,"id":"t1","ok":true,"result":"done","error":null}
```

The command becomes a job on the plane that holds node 3, and the ack
is published when the plane reports the job **complete**: an `on` acks
once the host LED is lit (or the plane gave up waiting), an `off` once
the host shut down or the escalation finished, which can take the
node's whole `t_soft_off_s`. A job that has not finished after 300 s is
acked with `ok: false` and `the action did not finish in time`; the
plane carries on, and the outcome still lands in the node's `last_job`.
A command the controller refuses (an absent node, a wrong target) is
acked at once with `ok: false`.

## When the broker goes away

The controller reconnects on an exponential ladder from 1 s to 60 s,
keeps running its rules and boot policy, and its `mqtt_connected` flag
is visible to the rules, in the state payload and as Modbus discrete
input 13. The last will sets `status` to `"online":false`. Nothing
about node power changes because the broker is gone: the planes hold
their nodes as they are, and the controller keeps serving the HTTP API
and Modbus.

The topic and payload reference is in
[MQTT topics and commands](../reference/mqtt.md); a ready-made Home
Assistant configuration is in [Home Assistant](../operation/home-assistant.md).
