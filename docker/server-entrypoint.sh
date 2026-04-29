#!/bin/sh
# Drop-privileges entrypoint for the server container.
#
# When this container previously ran as root (pre-LOW-1) the named
# `server_data` volume's contents were chowned to 0:0. Docker won't
# re-chown on volume reuse, so a fresh non-root image inheriting the
# old volume couldn't write the SQLite DB. Run a best-effort chown
# at startup so deployments self-heal across the root → app
# transition.
#
# This script is the entrypoint; gosu re-execs as the app user
# without staying root. Running as root for these few syscalls then
# dropping is standard practice and keeps the dropped-privs window
# tight.

set -eu

if [ -d /data ]; then
  chown -R app:app /data 2>/dev/null || true
fi

# Hand off to the requested CMD as the unprivileged user.
exec gosu app:app "$@"
