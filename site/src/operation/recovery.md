# Recovery

Which path applies depends on what you still have. Everything here
works over the network; the USB console is the extra option for a
board on the bench.

## Lost the admin password

With the recovery token or the fleet key, factory-reset the board over
the network (below), then set a new password. The configuration is
lost; import the export you kept.

With USB access: `set-password <new>` on the console. The
configuration stays, but every API token is invalidated (the console
path mints a new device salt); recreate them.

## Lost the network configuration

Wait for the safety nets first:

- A staged change that was never confirmed reverts by itself after the
  confirm window (default 5 min) with a reboot.
- A static address whose gateway has not answered a ping for the
  dead-man interval (default 1 h) is dropped for DHCP. Look for the MAC
  in the DHCP leases.

If the board is still unreachable at any address, a factory reset over
`POST /recover` puts it back on DHCP with no password. That endpoint
answers on HTTPS at whatever address the board has, including a
169.254.x.x link-local one.

## Factory reset over the network

Two credentials are accepted; either resets the board. The call is
rate-limited to one attempt per minute per board and logged as a
security event.

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

After either call the board erases its configuration and secrets,
keeps the `factory` namespace (recovery token and fleet key), and
reboots into first-setup mode on DHCP. The recovery token stays the
same, so your record stays valid. The HTTPS certificate and key are
secrets and are erased too: the board generates a new certificate on
the next boot, so update any pinned fingerprint. Reconnect, set a
password, import the configuration export, re-enter the MQTT
password, recreate API tokens.

## Factory reset with the password

Maintenance page, type the device id to confirm, or:

```sh
curl -sk -H "Authorization: Bearer $T" -X POST -d '{"confirm":"granite-37adc7"}' https://granite-37adc7.local/api/v1/factory-reset
```

Same result as above. On the console: `factory-reset CONFIRM`.

## A board that does not boot

If an OTA image is bad, the bootloader already handles it: an image
that cannot validate rolls back to the previous one, then to `factory`.
If `factory` itself is broken the board needs USB, which means lifting
it out. Keep `factory` on a release you trust, and never overwrite it
except on the bench.

## Replacing a board

1. Export the old board's configuration if it is still reachable.
2. Flash and commission the new board on the bench (MAC, token,
   password, fleet key).
3. Import the export; the device id and topic base change with the
   MAC unless you override `sys.device_id`, so update the broker ACLs,
   the Home Assistant topics and the Modbus master.
4. Move the cables, verify every LED sense, seal.

## What the controller logs

Every security event (login, lockout, password set, token created,
fleet key set, recovery accepted or rejected) goes to the log ring,
to the MQTT `event` topic when connected, and to the `log` topic at
or above the configured level.
