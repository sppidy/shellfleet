#!/bin/sh
# Manager-only. Red when any service has 0/N replicas or is converging.
# On non-manager hosts, returns green with a "not a manager" detail.
set -e
if ! command -v docker >/dev/null 2>&1; then
    echo "docker not installed"
    exit 0
fi
ROLE=$(docker info --format '{{.Swarm.ControlAvailable}}' 2>/dev/null || echo "false")
if [ "$ROLE" != "true" ]; then
    echo "not a swarm manager"
    exit 0
fi
PENDING=$(docker service ls --format '{{.Name}} {{.Replicas}}' 2>/dev/null | awk '{
    split($2, parts, "/");
    if (parts[1] != parts[2]) { print $1 " " $2 }
}')
if [ -z "$PENDING" ]; then
    COUNT=$(docker service ls -q | wc -l)
    echo "${COUNT} services all converged"
    exit 0
fi
LIST=$(echo "$PENDING" | tr '\n' ',' | sed 's/,$//')
echo "pending: $LIST"
exit 1
