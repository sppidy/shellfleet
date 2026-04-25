#!/bin/sh
# Red when there are exited containers in the last LOOKBACK seconds.
# Skips silently (green) if docker isn't installed/available.
# Env: LOOKBACK (seconds, default 3600 = 1h)
set -e
if ! command -v docker >/dev/null 2>&1; then
    echo "docker not installed"
    exit 0
fi
if ! docker info >/dev/null 2>&1; then
    echo "docker daemon unreachable"
    exit 1
fi
LOOKBACK="${LOOKBACK:-3600}"
SINCE=$(date -u -d "@$(( $(date -u +%s) - LOOKBACK ))" +%FT%TZ 2>/dev/null || echo "")
NAMES=$(docker ps -a --filter "status=exited" --filter "since=${SINCE}" --format '{{.Names}}' 2>/dev/null | tr '\n' ' ' | sed 's/ $//')
COUNT=$(echo "$NAMES" | wc -w)
if [ "$COUNT" -eq 0 ]; then
    RUNNING=$(docker ps -q | wc -l)
    echo "${RUNNING} running, no recent exits"
    exit 0
fi
echo "${COUNT} exited in last ${LOOKBACK}s: $NAMES"
exit 1
