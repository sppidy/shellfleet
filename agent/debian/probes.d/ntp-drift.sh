#!/bin/sh
# Red when system clock offset (chrony or timedatectl) exceeds THRESHOLD_MS.
# Env: THRESHOLD_MS (default 1000)
set -e
THRESHOLD_MS="${THRESHOLD_MS:-1000}"
OFFSET_MS=""
if command -v chronyc >/dev/null 2>&1; then
    # chronyc tracking emits "System time : 0.000123456 seconds slow of NTP time"
    OFFSET_S=$(chronyc tracking 2>/dev/null | awk -F: '/System time/ {print $2}' | awk '{print $1}')
    if [ -n "$OFFSET_S" ]; then
        OFFSET_MS=$(awk -v s="$OFFSET_S" 'BEGIN { printf "%d", (s<0?-s:s) * 1000 }')
    fi
fi
if [ -z "$OFFSET_MS" ] && command -v timedatectl >/dev/null 2>&1; then
    SYNC=$(timedatectl show -p NTPSynchronized --value 2>/dev/null || echo no)
    if [ "$SYNC" = "yes" ]; then
        OFFSET_MS=0
    else
        echo "NTP not synchronized (timedatectl)"
        exit 1
    fi
fi
if [ -z "$OFFSET_MS" ]; then
    echo "no chronyc or timedatectl available"
    exit 0
fi
if [ "$OFFSET_MS" -gt "$THRESHOLD_MS" ]; then
    echo "clock off by ${OFFSET_MS}ms (threshold ${THRESHOLD_MS})"
    exit 1
fi
echo "clock within ${OFFSET_MS}ms"
exit 0
