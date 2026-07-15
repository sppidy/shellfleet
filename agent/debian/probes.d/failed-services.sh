#!/bin/sh
# Fails red when systemd has any --failed units.
set -e
NAMES=$(systemctl list-units --failed --no-legend --plain 2>/dev/null | awk '{print $1}' | tr '\n' ' ' | sed 's/ $//')
COUNT=$(echo "$NAMES" | wc -w)
if [ "$COUNT" -eq 0 ]; then
    echo "no failed units"
    exit 0
fi
echo "$COUNT failed: $NAMES"
exit 1
