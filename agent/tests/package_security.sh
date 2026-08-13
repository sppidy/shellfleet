#!/bin/sh
# Literal grep patterns intentionally contain shell-variable syntax.
# shellcheck disable=SC2016
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
unit="$root/agent/debian/shellfleet-agent.service"
restricted="$root/agent/debian/shellfleet-agent-restricted.conf"
gate="$root/agent/debian/shellfleet-approval-gate.service"
proxy="$root/agent/debian/shellfleet-docker-proxy.service"
proxy_socket="$root/agent/debian/shellfleet-docker-proxy.socket"
proxy_helper="$root/agent/debian/shellfleet-docker-proxy"
pair_helper="$root/agent/debian/shellfleet-agent-pair"
mode_helper="$root/agent/debian/shellfleet-agent-mode"
prerm="$root/agent/debian/prerm"
postinst="$root/agent/debian/postinst"
postrm="$root/agent/debian/postrm"

# Fresh native installs are functional host-management agents. Root is accepted
# only with the explicit mode contract in the packaged unit; the restrictions
# that intentionally disable mutations live in a selectable drop-in.
grep -qx 'User=root' "$unit"
grep -qx 'Group=root' "$unit"
grep -qx 'Environment=SHELLFLEET_AGENT_MODE=managed' "$unit"
grep -qx 'NoNewPrivileges=true' "$unit"
grep -qx 'RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX' "$unit"
grep -qx 'EnvironmentFile=-/etc/shellfleet/docker-proxy.env' "$unit"
if grep -q '^CapabilityBoundingSet=$' "$unit"; then
    echo 'managed unit unexpectedly clears its capability set' >&2
    exit 1
fi
if grep -q '^ProtectSystem=strict$' "$unit"; then
    echo 'managed unit unexpectedly makes the host filesystem read-only' >&2
    exit 1
fi
if grep -q '^AppArmorProfile=' "$unit"; then
    echo 'managed unit unexpectedly applies the restricted agent profile' >&2
    exit 1
fi

grep -qx 'User=shellfleet' "$restricted"
grep -qx 'Group=shellfleet' "$restricted"
grep -qx 'Environment=SHELLFLEET_AGENT_MODE=restricted' "$restricted"
grep -qx 'CapabilityBoundingSet=' "$restricted"
grep -qx 'AmbientCapabilities=' "$restricted"
grep -qx 'ProtectSystem=strict' "$restricted"
grep -qx 'ProtectHome=yes' "$restricted"
grep -qx 'ReadWritePaths=/var/lib/shellfleet-agent' "$restricted"
grep -qx 'AppArmorProfile=-shellfleet-agent' "$restricted"
if grep -Eq 'SupplementaryGroups=.*docker|Group=docker' "$restricted"; then
    echo 'restricted mode unexpectedly grants docker-group membership' >&2
    exit 1
fi

grep -qx 'User=root' "$gate"
grep -qx 'RestrictAddressFamilies=AF_UNIX' "$gate"
grep -qx 'NoNewPrivileges=true' "$gate"
grep -qx 'RuntimeDirectoryPreserve=restart' "$gate"
grep -q 'systemctl try-restart shellfleet-approval-gate.service' "$postinst"
test -f "$root/agent/debian/apparmor/shellfleet-agent"
test -f "$root/agent/debian/apparmor/shellfleet-approval-gate"
test -f "$root/agent/debian/apparmor/shellfleet-docker-proxy"
grep -Fqx -- 'profile shellfleet-agent flags=(attach_disconnected,mediate_deleted) {' \
    "$root/agent/debian/apparmor/shellfleet-agent"
if grep -Eq '^profile shellfleet-agent[[:space:]]+/usr/bin/shellfleet-agent' \
    "$root/agent/debian/apparmor/shellfleet-agent"; then
    echo 'restricted AppArmor profile unexpectedly auto-attaches in managed mode' >&2
    exit 1
fi
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
grep -Fqx -- '  /var/lib/shellfleet-agent/ rw,' "$root/agent/debian/apparmor/shellfleet-agent"
grep -Fqx -- '  /var/lib/shellfleet-agent/** rwk,' "$root/agent/debian/apparmor/shellfleet-agent"

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
if grep -q 'enable .*shellfleet-docker-proxy.socket' "$root/agent/debian/postinst"; then
    echo 'package install unexpectedly enables the restricted-mode Docker proxy' >&2
    exit 1
fi
grep -q 'disable --now shellfleet-docker-proxy.socket' "$prerm"

# Pairing is initiated by an administrator but the network agent must still
# execute as the unprivileged service account so credentials remain portable
# between managed and restricted modes. The helper owns the service transition
# and loads only the packaged root-controlled environment file.
test -x "$pair_helper"
grep -q 'runuser -u shellfleet' "$pair_helper"
grep -q '\. /etc/shellfleet/env' "$pair_helper"
grep -q 'SHELLFLEET_AGENT_MODE=restricted' "$pair_helper"
grep -q "root:shellfleet 640" "$pair_helper"
grep -q 'exec /usr/bin/shellfleet-agent --pair' "$pair_helper"
grep -q 'systemctl stop shellfleet-agent.service' "$pair_helper"
grep -q 'systemctl restart shellfleet-agent.service' "$pair_helper"

# Runtime-mode selection is a supported, state-preserving transition. Managed
# is the default whenever no explicit operator choice has been recorded.
test -x "$mode_helper"
grep -q 'shellfleet-agent-mode {managed|restricted|status}' "$mode_helper"
grep -q '20-runtime-mode.conf' "$mode_helper"
grep -q 'normalize_restricted_credentials' "$mode_helper"
grep -q 'agent-token.txt agent-refresh.txt agent-token-expiry.txt' "$mode_helper"
grep -q 'agent-token.txt agent-refresh.txt agent-token-expiry.txt' "$postinst"
grep -q 'chown shellfleet:shellfleet' "$mode_helper"
grep -q 'systemctl stop "$UNIT"' "$mode_helper"
grep -q 'if \[ "$was_active" = true \]' "$mode_helper"
grep -q "printf '%s\\\\n' managed" "$postinst"
if grep -q 'migrated_restricted' "$postinst"; then
    echo 'an unrecorded legacy upgrade unexpectedly defaults to restricted' >&2
    exit 1
fi
grep -Fq 'AGENT_STATE_DIR=/var/lib/shellfleet-agent' "$postrm"
grep -Fq 'rm -rf -- "$AGENT_STATE_DIR"' "$postrm"
grep -Fq '"$CONF_DIR/agent-mode" "$MODE_DROPIN"' "$postrm"
grep -Fq 'depends = "$auto, systemd, ca-certificates"' "$root/agent/Cargo.toml"
grep -Fq 'debian/shellfleet-agent-mode' "$root/agent/Cargo.toml"
grep -Fq 'debian/shellfleet-agent-restricted.conf' "$root/agent/Cargo.toml"

grep -q 'SHELLFLEET_AGENT_MODE' "$root/agent/src/main.rs"
grep -q 'managed mode requires root' "$root/agent/src/main.rs"
grep -q 'restricted mode refuses root' "$root/agent/src/main.rs"
grep -qx 'USER 10001' "$root/Dockerfile.agent"
grep -qx 'USER 10001' "$root/Dockerfile.agent.k8s"
grep -qx 'ENV SHELLFLEET_AGENT_MODE=restricted' "$root/Dockerfile.agent"
grep -qx 'ENV SHELLFLEET_AGENT_MODE=restricted' "$root/Dockerfile.agent.k8s"
if grep -q 'SHELLFLEET_AGENT_MODE=managed' \
    "$root/Dockerfile.agent" "$root/Dockerfile.agent.k8s"; then
    echo 'a container image unexpectedly enables native managed mode' >&2
    exit 1
fi

echo 'package privilege boundary: ok'
