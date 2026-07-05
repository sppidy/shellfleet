#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
unit="$root/agent/debian/shellfleet-agent.service"
gate="$root/agent/debian/shellfleet-approval-gate.service"

grep -qx 'User=shellfleet' "$unit"
grep -qx 'Group=shellfleet' "$unit"
grep -qx 'CapabilityBoundingSet=' "$unit"
grep -qx 'AmbientCapabilities=' "$unit"
grep -qx 'NoNewPrivileges=true' "$unit"
grep -qx 'ProtectSystem=strict' "$unit"
grep -qx 'RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX' "$unit"
! grep -Eq 'SupplementaryGroups=.*docker|Group=docker' "$unit"

grep -qx 'User=root' "$gate"
grep -qx 'RestrictAddressFamilies=AF_UNIX' "$gate"
grep -qx 'NoNewPrivileges=true' "$gate"
test -f "$root/agent/debian/apparmor/shellfleet-agent"
test -f "$root/agent/debian/apparmor/shellfleet-approval-gate"
grep -q 'deny /run/docker.sock' "$root/agent/debian/apparmor/shellfleet-agent"
grep -q 'deny /run/dbus/system_bus_socket' "$root/agent/debian/apparmor/shellfleet-agent"
grep -q 'refuses to run as root' "$root/agent/src/main.rs"

echo 'package privilege boundary: ok'
