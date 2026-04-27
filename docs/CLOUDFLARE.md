# Cloudflare configuration for sys-manager

The live deploy fronts the origin with Cloudflare. The server already
has defence-in-depth security headers, per-actor + per-real-IP rate
limiting, and CSRF/RBAC, but Cloudflare is the **first** line of
defence and the right place for IP-keyed throttling: the edge sees the
real attacker IP before TLS terminates on your origin.

This document is the minimum config needed for sys-manager to be safe
behind Cloudflare. It assumes:

- The proxied A/AAAA record for `dashboard.example.com` (or whatever you
  use) is **orange-clouded** (proxy enabled).
- `JWT_SECRET` on the origin is a 64-char random hex (enforced by the
  server's startup check).
- WebSocket proxying is enabled (it is, by default, on Free).

## Headers

The origin's IP-keyed throttler reads the real client IP from
`CF-Connecting-IP`. Cloudflare sets this on every proxied request, so
you don't have to do anything here — but be aware of the implication:
**never expose the origin's port directly without Cloudflare in
front**, because anyone who reaches the origin directly can spoof
`CF-Connecting-IP` to dodge the throttle. The deploy uses Tailscale
ACLs to keep the origin off the public internet, which closes that
gap. (If you ever turn that off, set up an `allowed_ips` check at the
origin.)

## Rate Limiting Rules

Free plan gives you **5 rate-limiting rules**, which is enough.
Configure them in **Security → WAF → Rate limiting rules**.

### Rule 1 — Auth flood (login + callback + MFA verify)

```
If incoming requests match:
  (http.request.uri.path contains "/auth/")
  or (http.request.uri.path eq "/api/auth/mfa/verify")
Then:
  Action: Block
  When the rate exceeds: 10 requests
  Per: 1 minute
  Counting: by client IP
  Mitigation timeout: 10 minutes
```

Why: covers the OAuth dance, the post-OAuth MFA challenge, and the
`/auth/logout`. 10/minute is enough for any human to log in and out
several times; an attacker brute-forcing TOTP codes or trying to
exhaust the MFA throttle locks gets stopped here first.

### Rule 2 — Session probe flood

```
If incoming requests match:
  (http.request.uri.path eq "/api/me")
Then:
  Action: Managed Challenge
  When the rate exceeds: 60 requests
  Per: 1 minute
  Counting: by client IP
```

Why: `/api/me` is hit on every page load by the SPA's
SessionProvider. 60/min is generous for a real user. An attacker
flooding here keeps DB connections busy and is also a precursor to
session-fixation probes.

### Rule 3 — Device approve

```
If incoming requests match:
  (http.request.uri.path eq "/api/device/approve")
Then:
  Action: Block
  When the rate exceeds: 5 requests
  Per: 5 minutes
  Counting: by client IP
  Mitigation timeout: 30 minutes
```

Why: even though only authed admins reach this endpoint, an attacker
who steals an admin session shouldn't get to brute-force the 8-char
user_code at line speed. The origin already has a per-actor throttle
on top of this.

### Rule 4 — General API ceiling

```
If incoming requests match:
  starts_with(http.request.uri.path, "/api/")
  and not (http.request.uri.path eq "/api/me")
  and not starts_with(http.request.uri.path, "/api/auth/")
Then:
  Action: Managed Challenge
  When the rate exceeds: 300 requests
  Per: 1 minute
  Counting: by client IP
```

Why: a global ceiling that catches any anonymous abuse against the
API plane. Set high so authed power users (kicking off many container
ops) don't trip it. Excludes the paths covered by rules 1-3 so the
counters don't double-count.

### Rule 5 — WebSocket flood

```
If incoming requests match:
  (http.request.uri.path eq "/ui/ws")
  or (http.request.uri.path eq "/agent/ws")
Then:
  Action: Block
  When the rate exceeds: 10 requests
  Per: 1 minute
  Counting: by client IP
  Mitigation timeout: 10 minutes
```

Why: a real client opens at most one WS per page load and reconnects
on network blips. 10 upgrades/minute leaves headroom for flapping
networks; anything beyond is reconnect storms or scripted abuse.
Doesn't affect *bytes* on an established connection — that's
controlled by Cloudflare's per-connection limits already.

## Page Rules / Cache

`/api/*` and `/auth/*` and `/ui/ws` and `/agent/ws` should be
**Cache-Control: bypass** (or set "Cache Level: Bypass" via a Cache
Rule). Without this, Cloudflare may try to cache `/api/me` responses,
which would either leak a session across users or break the SPA.

The static dashboard assets (`/_next/...`) can be cached aggressively;
Next.js gives them content-hashed filenames so cache busting is
automatic.

## Bot Fight Mode

Free plan has **Bot Fight Mode**. Enable it under
**Security → Bots**. It's a low-cost defence against scripted
attackers using common bot signatures and adds zero latency to
real users. Don't pay for the upgraded "Super Bot Fight Mode" yet —
it's only worth it once you have customer traffic.

## Origin certificates

Use a Cloudflare Origin Certificate on the server side (not a public
cert from Let's Encrypt). The origin cert is signed by Cloudflare's
private CA and is only valid for connections through the Cloudflare
edge — meaning if someone ever finds and connects to the origin
directly without going through Cloudflare, the TLS handshake fails
loudly. This is "authenticated origin pulls" or just an origin cert
on the server side; either is a meaningful defence-in-depth.

## Verifying the rules are working

After applying, hammer one of the limited paths from `curl` and check
that you get 429 / Cloudflare's challenge page after the configured
threshold:

```bash
for i in {1..15}; do
  curl -s -o /dev/null -w "%{http_code}\n" \
    https://dashboard.example.com/api/auth/mfa/verify \
    -X POST -H 'content-type: application/json' -d '{"code":"000000"}'
done
```

Expect: ~10x `401` then a flip to `429` (Cloudflare) and ultimately
`403` (Cloudflare's "you're banned for 10 minutes" page).

## Things deliberately not configured here

- **WAF custom rules for SQL injection / XSS** — Cloudflare's
  managed rules already cover these. Don't add overlapping rules.
- **Country blocking** — sys-manager has legitimate users (the user
  themself) so geo-blocking is too blunt. If the deploy is ever
  single-country, revisit.
- **mTLS on /api/agent/ws** — agents currently auth via a
  per-agent token + Tailscale ACL. mTLS would be belt-and-suspenders
  but the operational cost (cert distribution to every agent) doesn't
  match the threat model yet.
