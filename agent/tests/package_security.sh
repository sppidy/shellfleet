#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
unit="$root/agent/debian/shellfleet-agent.service"
gate="$root/agent/debian/shellfleet-approval-gate.service"
proxy="$root/agent/debian/shellfleet-docker-proxy.service"
proxy_socket="$root/agent/debian/shellfleet-docker-proxy.socket"
proxy_helper="$root/agent/debian/shellfleet-docker-proxy"
pair_helper="$root/agent/debian/shellfleet-agent-pair"
prerm="$root/agent/debian/prerm"
postinst="$root/agent/debian/postinst"

grep -qx 'User=shellfleet' "$unit"
grep -qx 'Group=shellfleet' "$unit"
grep -qx 'CapabilityBoundingSet=' "$unit"
grep -qx 'AmbientCapabilities=' "$unit"
grep -qx 'NoNewPrivileges=true' "$unit"
grep -qx 'ProtectSystem=strict' "$unit"
grep -qx 'RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX' "$unit"
! grep -Eq 'SupplementaryGroups=.*docker|Group=docker' "$unit"
grep -qx 'EnvironmentFile=-/etc/shellfleet/docker-proxy.env' "$unit"

grep -qx 'User=root' "$gate"
grep -qx 'RestrictAddressFamilies=AF_UNIX' "$gate"
grep -qx 'NoNewPrivileges=true' "$gate"
grep -qx 'RuntimeDirectoryPreserve=restart' "$gate"
grep -q 'systemctl try-restart shellfleet-approval-gate.service' "$postinst"
test -f "$root/agent/debian/apparmor/shellfleet-agent"
test -f "$root/agent/debian/apparmor/shellfleet-approval-gate"
test -f "$root/agent/debian/apparmor/shellfleet-docker-proxy"
grep -q 'deny /run/docker.sock' "$root/agent/debian/apparmor/shellfleet-agent"
grep -q '/run/shellfleet/docker.sock rw,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/lib/docker/cli-plugins/ r,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/lib/docker/cli-plugins/** r,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/libexec/docker/cli-plugins/ r,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/libexec/docker/cli-plugins/** r,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/lib/docker/cli-plugins/docker-compose rix,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/lib/docker/cli-plugins/docker-buildx rix,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/libexec/docker/cli-plugins/docker-compose rix,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /usr/libexec/docker/cli-plugins/docker-buildx rix,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -q 'dbus (send, receive)' "$root/agent/debian/apparmor/shellfleet-agent"

# The proxy stays root-owned and is reachable only via a socket owned by the
# unprivileged service account. Package installation must never enable it.
grep -qx 'User=root' "$proxy"
grep -qx 'NoNewPrivileges=true' "$proxy"
grep -qx 'RestrictAddressFamilies=AF_UNIX' "$proxy"
grep -qx 'AppArmorProfile=-shellfleet-docker-proxy' "$proxy"
grep -qx 'SocketUser=shellfleet' "$proxy_socket"
grep -qx 'SocketGroup=shellfleet' "$proxy_socket"
grep -qx 'SocketMode=0660' "$proxy_socket"
grep -qx 'ExecStart=/lib/systemd/systemd-socket-proxyd /run/docker.sock' "$proxy"
grep -qx '    systemctl enable --now "$SOCKET_UNIT"' "$proxy_helper"
! grep -q 'enable .*shellfleet-docker-proxy.socket' "$root/agent/debian/postinst"
grep -q 'disable --now shellfleet-docker-proxy.socket' "$prerm"

# Pairing is initiated by an administrator but the network agent must still
# execute as the unprivileged service account. The helper owns the service
# transition and loads only the packaged root-controlled environment file.
test -x "$pair_helper"
grep -q 'runuser -u shellfleet' "$pair_helper"
grep -q '\. /etc/shellfleet/env' "$pair_helper"
grep -q "root:shellfleet 640" "$pair_helper"
grep -q 'exec /usr/bin/shellfleet-agent --pair' "$pair_helper"
grep -q 'systemctl stop shellfleet-agent.service' "$pair_helper"
grep -q 'systemctl restart shellfleet-agent.service' "$pair_helper"

grep -q 'refuses to run as root' "$root/agent/src/main.rs"

echo 'package privilege boundary: ok'
