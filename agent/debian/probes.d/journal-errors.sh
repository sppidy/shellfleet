#!/bin/sh
# Red when journalctl reports more than THRESHOLD priority<=err lines
# in the last LOOKBACK seconds.
# Env: THRESHOLD (default 0), LOOKBACK (default 900 seconds = 15min)
set -e
THRESHOLD="${THRESHOLD:-0}"
LOOKBACK="${LOOKBACK:-900}"
COUNT=$(journalctl --since "${LOOKBACK} seconds ago" -p err --no-pager 2>/dev/null | wc -l)
# journalctl includes a header line ("-- Logs begin at ... --") that
# we want to ignore. Subtract 1 if we have any output at all.
if [ "$COUNT" -gt 0 ]; then
    COUNT=$((COUNT - 1))
fi
if [ "$COUNT" -gt "$THRESHOLD" ]; then
    echo "${COUNT} error-priority lines in last ${LOOKBACK}s (threshold ${THRESHOLD})"
    exit 1
fi
echo "${COUNT} error-priority lines in last ${LOOKBACK}s"
exit 0
