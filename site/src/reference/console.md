# Console commands

The USB-C port is an interactive console (USB Serial/JTAG, shows up as
`/dev/ttyACM0`). It asks for no password: physical access is full
access. It is not reachable over the network. Use `espflash monitor`
interactively, `picocom`, `screen` or any serial terminal; the baud
rate is irrelevant on USB.

```
granite> help
commands:
  id                    device id, mac, firmware, cert fingerprint
  status                net, ota slot and state, uptime, free heap
  net                   network detail
  set-password <pw>     set the admin password (first setup or reset)
  recovery-token        print the per-device recovery token again
  fleet-key             fleet recovery key fingerprint
  factory-reset CONFIRM erase config and secrets, keep factory, reboot
  ota-mark-valid        confirm the running image
  reboot                restart through the planned-reboot path
  log [n]               last n lines of the log ring (default 40)
  help                  this list
```

| Command | Output |
|---|---|
| `id` | `device`, `mac`, `fw`, `built`, `cert sha256` |
| `status` | `net link up/down <mode> <ip> gw <gw>`, `ota running X (state), next Y`, `signing ...`, `uptime`, `heap free / low water`, `boot` reason, `time` source |
| `net` | link, mode, ip, netmask, gateway, IPv6 link-local, hostname, mDNS, time (`sntp` or `build stamp`); plus `deadman fired: dhcp was started as a fallback` and `confirm pending, N s left before a revert reboot` when applicable |
| `set-password <pw>` | `ok: admin password set`, or an error below 8 characters |
| `recovery-token` | `recovery <token>` and a reminder to keep it with the MAC |
| `fleet-key` | `fleet-key sha256 <fingerprint>` or `fleet-key none configured` |
| `factory-reset CONFIRM` | Erases config and secrets, keeps the `factory` namespace (recovery token, fleet key), reboots. Without the literal `CONFIRM` it refuses. |
| `ota-mark-valid` | Ends probation of the running image |
| `reboot` | Planned reboot; node power untouched |
| `log [n]` | Last `n` lines of the 16 KB log ring, then `-- N lines kept` |

`?` is an alias for `help`. Lines are limited to 512 characters;
backspace and Ctrl-C work.

The log ring is also what the Maintenance page and
`GET /api/v1/log/tail` show, and it captures the first boot lines
before the network is up, which is where an expander or W5500 problem
shows.
