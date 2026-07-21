#!/usr/bin/env bash
set -euo pipefail

channel="${MULLVAD_CHANNEL:-stable}"
case "$channel" in
  stable|beta) ;;
  *)
    echo "MULLVAD_CHANNEL must be stable or beta" >&2
    exit 2
    ;;
esac

curl --fail --location --silent --show-error \
  https://repository.mullvad.net/deb/mullvad-keyring.asc \
  --output /usr/share/keyrings/mullvad-keyring.asc
architecture="$(dpkg --print-architecture)"
printf '%s\n' \
  "deb [signed-by=/usr/share/keyrings/mullvad-keyring.asc arch=$architecture] https://repository.mullvad.net/deb/$channel $channel main" \
  > /etc/apt/sources.list.d/mullvad.list

apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install --yes --no-install-recommends mullvad-vpn
dpkg-query --show --showformat='Testing Mullvad package ${Version}\n' mullvad-vpn

if [[ ! -c /dev/net/tun ]]; then
  mkdir -p /dev/net
  mknod /dev/net/tun c 10 200
fi

/usr/bin/mullvad-daemon -v > /tmp/mullvad-daemon.log 2>&1 &
daemon_pid=$!
trap 'kill "$daemon_pid" 2>/dev/null || true; cat /tmp/mullvad-daemon.log' EXIT

for _ in $(seq 1 60); do
  [[ -S /var/run/mullvad-vpn ]] && break
  kill -0 "$daemon_pid"
  sleep 1
done
if [[ ! -S /var/run/mullvad-vpn ]]; then
  echo "Mullvad daemon did not create /var/run/mullvad-vpn within 60 seconds" >&2
  exit 1
fi

cargo test --locked --no-default-features --test controller_integration \
  -- --ignored --test-threads=1
