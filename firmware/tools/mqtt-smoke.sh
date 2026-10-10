#!/bin/sh
# Local broker for firmware tests: mosquitto on port 1883, no auth, no TLS,
# and a subscriber printing every granite topic. Point the controller's
# MQTT page at mqtt://<this host's IP>:1883 (plain mqtt:// is allowed; the
# firmware logs a warning because the default is mqtts://).
#
# Usage: firmware/tools/mqtt-smoke.sh [port]
set -eu
PORT="${1:-1883}"
DIR="$(mktemp -d)"
cat > "$DIR/mosquitto.conf" <<CONF
listener $PORT
allow_anonymous true
persistence false
log_type error
log_type warning
CONF
echo "broker on port $PORT, config in $DIR; ctrl-c stops both"
mosquitto -c "$DIR/mosquitto.conf" &
BROKER=$!
trap 'kill $BROKER 2>/dev/null; rm -rf "$DIR"' EXIT INT TERM
sleep 0.5
exec mosquitto_sub -h 127.0.0.1 -p "$PORT" -v -t 'granite/#'
