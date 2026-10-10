# Network requirements

The controller needs one 10/100 Ethernet port and an IPv4 address.
Everything else is optional and depends on which northbound interface
you use.

## Addressing

- **DHCP** is the default. The controller sends its hostname
  (`granite-xxxxxx`) in the request, so a DHCP server that registers
  names in DNS makes it resolvable by name.
- **AutoIP fallback**: after four failed DHCP attempts (about 30 s) the
  controller takes a 169.254.x.x link-local address while DHCP keeps
  retrying. On a bench with no DHCP server you can still reach it
  through mDNS or a link-local address on your laptop.
- **Static** addresses are configured on the Network page. A static
  change is applied with a commit-confirm step: if you do not confirm
  within the confirm window the board reverts, so a typo cannot lock
  you out. See [Network configuration](../commissioning/network.md).
- **Dead-man**: a static address whose configured gateway has not
  answered a ping for a configurable time (default one hour, 0
  disables it) makes the controller drop the static address and start
  DHCP. This is the safety net for a submerged board whose static
  network was renumbered under it. **Current firmware:** only the
  gateway ping counts; accepted connections do not reset the timer.
  A static configuration therefore needs a gateway that answers ICMP
  echo, or the dead-man set to 0.
- **mDNS**: the board advertises `granite-xxxxxx.local` with the
  services `_https._tcp` and `_granite._tcp` on port 443 (the TXT record carries `id=<device id>`). On a flat
  subnet `http://granite-xxxxxx.local/id` finds it without any server.

Reserve the MAC address in your DHCP server, or at least record it:
the device id, the hostname and the recovery procedure all key on it.

## Ports

Inbound, towards the controller:

| Port | Protocol | Service | Default | Notes |
|---|---|---|---|---|
| 443 | TCP | HTTPS setup page and `/api/v1` | on | Self-signed device certificate; pin its fingerprint |
| 80 | TCP | `/id` and a redirect to HTTPS | on | Unauthenticated, read only |
| 502 | TCP | Modbus TCP | **off** | Port configurable; listens only with an IP allow-list |
| 5353 | UDP | mDNS | on | Multicast 224.0.0.251, same subnet only |

Outbound, from the controller:

| Port | Protocol | Service | When |
|---|---|---|---|
| 67/68 | UDP | DHCP | Always, unless static |
| 53 | UDP | DNS | Resolving the broker, the NTP server and OTA URLs |
| 123 | UDP | NTP | Always; the server comes from DHCP option 42 or configuration |
| 8883 (or 1883) | TCP | MQTT to the broker | When MQTT is enabled |
| 443 | TCP | HTTPS to an update server | Only while a pull update (`ota` command with a URL) runs |

Nothing else. The radio is off in the shipped firmware; there is no
Wi-Fi, no Bluetooth and no cloud dependency.

## Placing the controller in your network

A frame controller is a management device. Treat it like a BMC:

- Put it on a management VLAN or an isolated subnet with the MQTT
  broker and the Modbus master, not on the nodes' production network.
- The HTTPS certificate is per device and self-signed. Browsers warn
  once; compare the fingerprint the page shows with the one the USB
  console printed (`id` command) the first time, then accept it.
  Scripts should pin the fingerprint rather than disable verification.
- Modbus has no authentication at all. The controller refuses to
  listen until you enter an allow-list of client addresses or CIDRs,
  and that list is the only protection. Keep Modbus inside the
  management subnet.
- MQTT should run over TLS with the broker's CA loaded on the
  controller, and with per-device credentials or a client certificate.
  A plaintext broker on 1883 works for a bench.

## Time

The controller boots with its clock set to the firmware build time and
then synchronises with NTP. Until it has synchronised, TLS still works
(certificates are checked against the build time) but log and event
timestamps are wrong. Provide NTP through DHCP option 42 if you can; it
needs no configuration on the board.

## Several frames

Each board is `granite-<mac>` and publishes under
`<topic_root>/<site>/<device>`. Set the same `site` on every board of a
pod and give every broker user access only to its own `site` prefix if
you need isolation. Nothing on the boards needs to know about each
other.
