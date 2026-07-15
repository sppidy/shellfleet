#!/bin/sh
# Red when sustained swap activity (si+so per second) exceeds THRESHOLD_KB.
# Samples vmstat for 2 seconds.
# Env: THRESHOLD_KB (default 256)
set -e
THRESHOLD_KB="${THRESHOLD_KB:-256}"
# vmstat 1 2 prints two samples; we want the last (post-warmup) row's si + so.
LAST=$(vmstat 1 2 2>/dev/null | tail -n 1)
SI=$(echo "$LAST" | awk '{print $7}')
SO=$(echo "$LAST" | awk '{print $8}')
RATE=$((SI + SO))
if [ "$RATE" -gt "$THRESHOLD_KB" ]; then
    echo "swap rate ${RATE} kB/s (threshold ${THRESHOLD_KB})"
    exit 1
fi
echo "swap rate ${RATE} kB/s"
exit 0
