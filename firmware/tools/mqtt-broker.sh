#!/bin/sh
# Persistent bench broker for controller tests: mosquitto as a systemd
# user service, the way a board talks to a real broker.
#
#   8883  TLS (local test CA) + username/password, on all interfaces
#   1883  plain, anonymous, 127.0.0.1 only (for mosquitto_sub on this host)
#
# State (CA, server key, password file, config) lives outside the repo in
# $GRANITE_MQTT_DIR (default ~/.local/share/granite-mqtt). Re-running the
# script keeps the CA and the existing users, re-issues the server cert
# for the given IP, and restarts the service.
#
# Usage: firmware/tools/mqtt-broker.sh <broker-ip> [user ...]
#   e.g. firmware/tools/mqtt-broker.sh 192.168.64.99 granite-cb374b
# Each new user gets a random password, printed once and stored in
# $GRANITE_MQTT_DIR/users/<user> (mode 600).
#
# Watch everything:  mosquitto_sub -h 127.0.0.1 -v -t 'granite/#'
# The CA for the board's MQTT page: $GRANITE_MQTT_DIR/ca.crt
set -eu

IP="${1:?usage: mqtt-broker.sh <broker-ip> [user ...]}"
shift
DIR="${GRANITE_MQTT_DIR:-$HOME/.local/share/granite-mqtt}"
UNIT="$HOME/.config/systemd/user/granite-mqtt.service"

mkdir -p "$DIR/users"
chmod 700 "$DIR" "$DIR/users"
cd "$DIR"

if [ ! -f ca.key ]; then
    openssl ecparam -name prime256v1 -genkey -noout -out ca.key
    openssl req -x509 -new -key ca.key -sha256 -days 3650 \
        -subj "/CN=granite bench CA" -out ca.crt
    chmod 600 ca.key
fi

# Server cert: ECDSA P-256, SAN = the broker IP (boards connect by IP).
openssl ecparam -name prime256v1 -genkey -noout -out server.key
chmod 600 server.key
openssl req -new -key server.key -subj "/CN=$IP" -out server.csr
printf 'subjectAltName=IP:%s\nextendedKeyUsage=serverAuth\n' "$IP" > server.ext
openssl x509 -req -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
    -sha256 -days 825 -extfile server.ext -out server.crt
rm -f server.csr server.ext

touch passwd
chmod 600 passwd
for user in "$@"; do
    if [ -f "users/$user" ]; then
        echo "user $user exists, password unchanged"
        continue
    fi
    pw="$(openssl rand -hex 12)"
    mosquitto_passwd -b passwd "$user" "$pw"
    printf '%s\n' "$pw" > "users/$user"
    chmod 600 "users/$user"
    echo "user $user password $pw"
done

cat > mosquitto.conf <<CONF
per_listener_settings true
persistence false
log_dest stdout
log_type error
log_type warning
log_type notice

listener 1883 127.0.0.1
allow_anonymous true

listener 8883
allow_anonymous false
password_file $DIR/passwd
cafile $DIR/ca.crt
certfile $DIR/server.crt
keyfile $DIR/server.key
CONF

mkdir -p "$(dirname "$UNIT")"
cat > "$UNIT" <<UNITEOF
[Unit]
Description=Granite bench MQTT broker (firmware/tools/mqtt-broker.sh)

[Service]
ExecStart=/usr/bin/mosquitto -c $DIR/mosquitto.conf
Restart=on-failure

[Install]
WantedBy=default.target
UNITEOF

systemctl --user daemon-reload
systemctl --user enable granite-mqtt.service >/dev/null 2>&1
systemctl --user restart granite-mqtt.service
echo "broker up: mqtts://$IP:8883 (CA $DIR/ca.crt), local mqtt://127.0.0.1:1883"
