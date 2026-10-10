# MQTT

MQTT is the intended day-to-day control path: the controller publishes
retained status and state, streams events and logs, and takes commands.
It is off by default. The MQTT page edits the `mqtt` section, which is
**staged** like the network section, but unlike the network section it
is not applied live: the sequence is stage, confirm, **reboot**, then
check the broker session. The MQTT worker is started at boot only when
the saved section has it enabled, and a changed broker is only logged
until the next restart.

## Broker side

Before touching the board:

1. Create a user (or a client certificate) per board, with publish
   rights on `<topic_root>/<site>/<device>/#` and subscribe rights on
   `<topic_root>/<site>/<device>/cmd`. Your automation needs the
   inverse.
2. Have the broker's CA certificate in PEM form. The controller
   verifies the broker with it; there is no "accept any certificate"
   switch other than the time check.
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
| Auto-confirm on connect | off | Shown in the UI; **not yet wired**. Confirm by hand. |
| Skip TLS time check | off | Shown in the UI; **ignored** by the current firmware. |

The password or client certificate and key go through a separate
credentials call so they never appear in an export:

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT \
  -d '{"username":"granite-37adc7","password":"..."}' \
  https://granite-37adc7.local/api/v1/security/mqtt-credentials
```

Then stage the section, confirm, and reboot:

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT \
  -d '{"enabled":true,"host":"broker.site.example","port":8883,"tls":true,"ca_pem":"-----BEGIN CERTIFICATE-----\n...","username":"granite-37adc7","site":"pod1"}' \
  https://granite-37adc7.local/api/v1/config/mqtt
curl -sk -H "Authorization: Bearer $T" -X POST https://granite-37adc7.local/api/v1/config/confirm
curl -sk -H "Authorization: Bearer $T" -X POST https://granite-37adc7.local/api/v1/reboot
```

A reboot does not touch node power. After it, the Status page's MQTT
box should read connected; if not, `last_error` in that box and the
Maintenance log say why (CA mismatch and a clock before the
certificate's not-before are the usual ones).

## Check it

```sh
mosquitto_sub -h broker.site.example -p 8883 --cafile ca.pem -u ops -P ... -v -t 'granite/pod1/granite-37adc7/#'
```

You should see the retained `status` (`"online":true`) and `state`
messages at once, then `event` lines as nodes change. Send a command
and watch the ack:

```sh
mosquitto_pub ... -t granite/pod1/granite-37adc7/cmd \
  -m '{"v":1,"id":"t1","action":"on","target":3}'
# -> granite/pod1/granite-37adc7/ack/t1 {"v":1,"id":"t1","ok":true,"result":"ok","error":null}
```

The ack is published when the action **completes**, so an `on` acks
after the LED comes up (up to `t_on`), an `off` after the OS shut down
(up to `t_soft_off`) or after the escalation to a forced power-off.

## When the broker goes away

The controller reconnects on an exponential ladder from 1 s to 60 s,
keeps running its rules and boot policy, and its `mqtt_connected` flag
is visible to the rules, in the state payload and as Modbus discrete
input 13. The last will sets `status` to `"online":false`. Nothing
about node power changes because the broker is gone.

The topic and payload reference is in
[MQTT topics and commands](../reference/mqtt.md); a ready-made Home
Assistant configuration is in [Home Assistant](../operation/home-assistant.md).
