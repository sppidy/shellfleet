#!/bin/sh
# Red when 1-minute load average per CPU exceeds THRESHOLD (default 1.0).
# Env: THRESHOLD (float, default 1.0)
set -e
THRESHOLD="${THRESHOLD:-1.0}"
LOAD1=$(awk '{print $1}' /proc/loadavg)
CPUS=$(nproc 2>/dev/null || echo 1)
# Compute load1 / cpus and compare to threshold using awk (float math).
RATIO=$(awk -v l="$LOAD1" -v c="$CPUS" 'BEGIN { if (c==0) c=1; printf "%.2f", l/c }')
OVER=$(awk -v r="$RATIO" -v t="$THRESHOLD" 'BEGIN { print (r > t) ? "1" : "0" }')
if [ "$OVER" = "1" ]; then
    echo "load1 ${LOAD1} on ${CPUS} CPU(s) → ${RATIO}/CPU (threshold ${THRESHOLD})"
    exit 1
fi
echo "load1 ${LOAD1} on ${CPUS} CPU(s) → ${RATIO}/CPU"
exit 0
