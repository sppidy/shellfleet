#!/bin/sh
# Counts upgradable apt packages. Green only when 0; red emits the count.
# No env vars.
set -e
COUNT=$(apt list --upgradable 2>/dev/null | grep -cE 'upgradable from:' || true)
if [ "$COUNT" -eq 0 ]; then
    echo "no pending updates"
    exit 0
fi
echo "$COUNT package(s) pending upgrade"
exit 1
