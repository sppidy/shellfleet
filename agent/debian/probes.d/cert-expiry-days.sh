#!/bin/sh
# Red when any letsencrypt fullchain.pem expires in fewer than THRESHOLD days.
# Env: THRESHOLD (days, default 14), CERT_GLOB (default /etc/letsencrypt/live/*/fullchain.pem)
set -e
THRESHOLD="${THRESHOLD:-14}"
CERT_GLOB="${CERT_GLOB:-/etc/letsencrypt/live/*/fullchain.pem}"
NOW=$(date -u +%s)
WORST_DAYS=99999
WORST_NAME=""
FOUND=0
for cert in $CERT_GLOB; do
    [ -f "$cert" ] || continue
    FOUND=1
    END_DATE=$(openssl x509 -enddate -noout -in "$cert" 2>/dev/null | cut -d= -f2)
    [ -n "$END_DATE" ] || continue
    END_TS=$(date -u -d "$END_DATE" +%s 2>/dev/null || echo 0)
    [ "$END_TS" -gt 0 ] || continue
    DAYS=$(( (END_TS - NOW) / 86400 ))
    if [ "$DAYS" -lt "$WORST_DAYS" ]; then
        WORST_DAYS=$DAYS
        WORST_NAME=$(basename "$(dirname "$cert")")
    fi
done
if [ "$FOUND" -eq 0 ]; then
    echo "no certs found at $CERT_GLOB"
    exit 0
fi
if [ "$WORST_DAYS" -lt "$THRESHOLD" ]; then
    echo "${WORST_NAME} expires in ${WORST_DAYS} day(s) (threshold ${THRESHOLD})"
    exit 1
fi
echo "${WORST_NAME} expires in ${WORST_DAYS} day(s)"
exit 0
