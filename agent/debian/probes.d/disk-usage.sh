#!/bin/sh
# Root filesystem usage. Red when use% exceeds THRESHOLD (default 90).
# Env: THRESHOLD (integer percent), MOUNT (default /)
set -e
THRESHOLD="${THRESHOLD:-90}"
MOUNT="${MOUNT:-/}"
USED=$(df -P "$MOUNT" | awk 'NR==2 {gsub("%",""); print $5}')
if [ -z "$USED" ]; then
    echo "df failed for $MOUNT"
    exit 1
fi
if [ "$USED" -ge "$THRESHOLD" ]; then
    echo "$MOUNT at ${USED}% (threshold ${THRESHOLD}%)"
    exit 1
fi
echo "$MOUNT at ${USED}%"
exit 0
