# Network configuration

The Network page edits the `net` section. Every change here is
**staged**: it is applied immediately, but it is only saved once you
confirm it through the new address within the confirm window. If you
cannot reach the board after the change, do nothing: when the window
expires the board reboots into the previous settings.

## DHCP with a reservation (recommended)

Leave the mode on DHCP and reserve the address on the DHCP server by
MAC. The board keeps its name resolvable through the DHCP hostname and
mDNS, you keep a single place to renumber, and the dead-man fallback is
irrelevant because there is no static address to get wrong.

## Static address

Fill in:

| Field | Meaning |
|---|---|
| Address | IPv4 in CIDR form, for example `10.20.0.31/24`. Required. |
| Gateway | Default router. The dead-man uses its reachability. |
| DNS | One or more servers. Needed to resolve the broker, the NTP server or OTA URLs by name. |
| Hostname | Empty keeps `granite-xxxxxx`. |
| SNTP server | Empty keeps the server from DHCP option 42; a static setup should name one. |
| Confirm window | Seconds you have to confirm a staged change. 30 to 3600, default 300. |
| Static dead-man | Seconds without a ping answer from the gateway before the board drops the static address and starts DHCP. Default 3600, 0 disables it. The gateway must answer ICMP echo, or set 0. |
| mDNS | Advertise `granite-xxxxxx.local`. Keep on unless your policy forbids multicast. |
| VLAN id | Present in the UI; VLAN tagging is **not implemented** in the current firmware. Leave empty and do tagging on the switch port. |

Press "Stage and apply". The board switches to the new address at
once. Open the page again at the **new** address, log in, and press
"Confirm" in the banner (or `POST /api/v1/config/confirm`). The Status
page shows which sections are staged and the seconds left.

Over the API:

```sh
curl -sk -H "Authorization: Bearer $T" -X PUT \
  -d '{"ip_mode":"static","address":"10.20.0.31/24","gateway":"10.20.0.1","dns":["10.20.0.1"],"sntp":"10.20.0.1"}' \
  https://granite-37adc7.local/api/v1/config/net
# -> {"ok":true,"section":"net","staged":true,"confirm_s":300}
curl -sk -H "Authorization: Bearer $T" -X POST https://10.20.0.31/api/v1/config/confirm
```

Fields you leave out of a section PUT take their defaults, so send the
whole section; `GET /api/v1/config/net` gives you the current one to
edit.

## What happens on a mistake

- Wrong address or gateway: you lose the session, wait out the window,
  the board reboots into the old configuration and answers again at the
  old address.
- Wrong VLAN on the switch side: the firmware cannot recover that;
  fix the switch port.
- A static address that was correct but whose network was renumbered
  later, with the board sealed: once the gateway has not answered a
  ping for the dead-man interval the board starts DHCP instead, and
  `net` on the console or the status page reports that the dead-man
  fired. Reserve the MAC in the new network's DHCP server and the
  board reappears.
- Everything else: `POST /recover` with the recovery token or the fleet
  key resets the board to DHCP and no password. See
  [Recovery](../operation/recovery.md).

## Time

Until the first NTP sync the clock is the firmware build time. The
status page and `net` on the console show whether SNTP has synced. TLS
to the broker works either way as long as the broker certificate's
not-before date is earlier than the firmware build time. (The MQTT
page shows a "skip TLS time check" box; the current firmware ignores
it because the ESP-IDF MQTT client has no such option.)

## Hostname and device id

The device id (`sys.device_id`) and hostname default to
`granite-<mac6>` and can be overridden. The device id is part of the
MQTT topic base and of the factory-reset confirmation, so change it
only with a naming scheme in mind, and record the mapping to the MAC.
