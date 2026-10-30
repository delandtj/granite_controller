# First setup over the network

With the firmware flashed, connect Ethernet and 19 V (or keep USB
power on the bench) and finish setup from a browser.

## 1. Find the board

The board asks for a DHCP lease under the hostname `granite-xxxxxx`
and advertises itself over mDNS. Any of these works:

- `http://granite-xxxxxx.local/id` on the same subnet;
- `http://<ip>/id` with the address from your DHCP server's lease
  table (filter by the MAC you wrote down);
- the console's `net` command if USB is still connected.

`/id` is unauthenticated and read-only. It returns the device id, MAC,
firmware version, certificate fingerprint, whether a password is set
and the OTA state, so you can confirm you are on the right board before
typing a password into it:

```json
{"device":"granite-37adc7","mac":"..","fw":"0.1.0",
 "cert_sha256":"..","password_set":false,"uptime_s":42,
 "ota":{"running":"factory","state":"valid","pending_verify":false},
 "modbus_map_version":1,"nonce":"..","v":1}
```

If no DHCP server answers within about 30 s the board adds a
169.254.x.x address; a laptop on the same switch with link-local
addressing reaches it through `granite-xxxxxx.local`.

## 2. Accept the certificate

Open `https://granite-xxxxxx.local/` (or the IP). The certificate is
self-signed per device, so the browser warns. Compare the fingerprint
with the one `/id` or the console printed, then accept it. You can
upload your own certificate and key on the Security page later; the
page then says "reconnect", but the HTTPS server only loads the new
certificate on the next reboot, so reboot the controller afterwards.

## 3. Set the admin password

The page opens on the first-setup panel. Until a password exists the
board refuses every other request with 403; login attempts get 409.
Enter a password of at least 8 characters, twice.

The response card shows the **recovery token once** (only on this
first setup from the page; a board that had its password set over USB
or that was factory-reset does not show it again). Tick "I have
written it down" only after you have, and keep it next to the MAC.

Over the API the same step is:

```sh
B=https://granite-37adc7.local
curl -sk -X POST -d '{"password":"correct horse battery"}' $B/api/v1/security/password
# -> {"ok":true,"first_boot":true,"recovery_token":"XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX"}
```

Setting the password does not log you in. Log in next.

## 4. Log in

The login panel takes the password and sets a session cookie valid for
12 hours. Five wrong passwords lock the login for 60 s, and the lockout
is logged as a security event.

For scripts, create an API token on the Security page (or with
`POST /api/v1/security/tokens`) and send it as
`Authorization: Bearer <token>`. The token is shown once; the board
stores only a hash. Tokens are the right tool for automation; the
cookie is for the browser.

```sh
curl -sk -c jar -X POST -d '{"password":"correct horse battery"}' $B/api/v1/session
curl -sk -b jar $B/api/v1/status
curl -sk -b jar -X POST -d '{"name":"ansible"}' $B/api/v1/security/tokens
# -> {"ok":true,"name":"ansible","token":"<64 hex>"}
```

## 5. Install the fleet recovery key

Optional but recommended for any site with more than a couple of
boards. One ECDSA P-256 key pair per site lets you factory-reset any
board from the network without its individual token.

On a workstation, once:

```sh
cd firmware
cargo run -p granite-sim -- keygen        # writes ~/.config/granite/fleet-recovery.key
```

The command prints the public key in PEM form. Paste it into "Fleet
recovery key" on the Security page (or `PUT /api/v1/security/fleet-key`
with `{"pem":"..."}`). The private key stays on the workstation, never
on a board. Replacing the key later needs the admin password.

Boards can also be built with the public key baked in through the
`FLEET_RECOVERY_PUBKEY` environment variable at build time, so a fresh
board trusts it from the first boot.

## 6. Look at the Status page

The Status page shows the eight nodes with their state and LED reading,
the sensors, the network summary, the MQTT summary and the running
firmware slot. On a board with nothing cabled yet every node reads
`unknown` or `off` depending on whether the sense expander answered.
That is expected; the next chapters fill it in.
