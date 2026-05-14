# Cloudflare configuration for ShellFleet

The live deploy fronts the origin with Cloudflare. The server already has
defence-in-depth headers, per-actor + per-real-IP rate limiting, and
CSRF/RBAC, but Cloudflare is the first line of defence. The edge sees the
real attacker IP before TLS terminates on your origin -- that's where
IP-keyed throttling belongs.

This is the minimum config for ShellFleet behind Cloudflare. Assumes:

- The proxied A/AAAA record is **orange-clouded** (proxy enabled).
- `JWT_SECRET` on the origin is a 64-char random hex (enforced by the
  server's startup check).
- WebSocket proxying is enabled (it is, by default, on Free).

## Headers

The origin's IP-keyed throttler reads client IP from `CF-Connecting-IP`.
Cloudflare sets this on every proxied request -- nothing to configure.
But: **never expose the origin's port directly without Cloudflare in
front**, because anyone who reaches the origin directly can spoof
`CF-Connecting-IP` to dodge the throttle. The deploy uses Tailscale ACLs
to keep the origin off the public internet. (If you turn that off, set up
an `allowed_ips` check at the origin.)

## Rate Limiting Rules

Free plan gives you **5 rate-limiting rules**. Configure them in
**Security → WAF → Rate limiting rules**.

### Rule 1 -- Auth flood (login + callback + MFA verify)

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

Covers the OAuth dance, post-OAuth MFA challenge, and `/auth/logout`.
10/minute is plenty for a human to log in and out several times; an
attacker brute-forcing TOTP codes gets stopped here first.

### Rule 2 -- Session probe flood

```
If incoming requests match:
  (http.request.uri.path eq "/api/me")
Then:
  Action: Managed Challenge
  When the rate exceeds: 60 requests
  Per: 1 minute
  Counting: by client IP
```

`/api/me` is hit on every page load by the SPA's SessionProvider.
60/min is generous for a real user. Flooding here keeps DB connections
busy and is a precursor to session-fixation probes.

### Rule 3 -- Device approve

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

Even though only authed admins reach this endpoint, a stolen admin
session shouldn't get to brute-force the 8-char user_code at line speed.
The origin already has a per-actor throttle on top.

### Rule 4 -- General API ceiling

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

Global ceiling that catches anonymous abuse against the API plane. Set
high so authed power users (kicking off many container ops) don't trip
it. Excludes paths covered by rules 1-3 to avoid double-counting.

### Rule 5 -- WebSocket flood

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

A real client opens at most one WS per page load and reconnects on
network blips. 10 upgrades/minute leaves headroom for flapping networks;
anything beyond is reconnect storms or scripted abuse. Doesn't affect
bytes on an established connection -- that's Cloudflare's per-connection
limits.

## Page Rules / Cache

`/api/*`, `/auth/*`, `/ui/ws`, and `/agent/ws` should be **Cache-Control:
bypass** (or "Cache Level: Bypass" via a Cache Rule). Without this,
Cloudflare may cache `/api/me` responses, which would leak a session
across users or break the SPA.

Static dashboard assets (`/_next/...`) can be cached aggressively;
Next.js gives them content-hashed filenames so cache busting is automatic.

## Bot Fight Mode

Enable under **Security → Bots**. Low-cost defence against scripted
attackers using common bot signatures, zero latency for real users.
Don't pay for "Super Bot Fight Mode" yet -- only worth it with customer
traffic.

## Origin certificates

Use a Cloudflare Origin Certificate on the server side (not a public
cert from Let's Encrypt). The origin cert is signed by Cloudflare's
private CA and is only valid through the Cloudflare edge -- if someone
connects to the origin directly, the TLS handshake fails. This is
"authenticated origin pulls" or an origin cert on the server side;
either is meaningful defence-in-depth.

## Verifying the rules

After applying, hammer a limited path from `curl` and check that you get
429 / Cloudflare's challenge page after the configured threshold:

```bash
for i in {1..15}; do
  curl -s -o /dev/null -w "%{http_code}\n" \
    https://<your-dashboard>/api/auth/mfa/verify \
    -X POST -H 'content-type: application/json' -d '{"code":"000000"}'
done
```

Expect: ~10x `401` then a flip to `429` (Cloudflare) and ultimately
`403` (Cloudflare's "you're banned for 10 minutes" page).

## What's not configured here

- **WAF custom rules for SQLi / XSS** -- Cloudflare's managed rules
  already cover these. Don't add overlapping rules.
- **Country blocking** -- ShellFleet has legitimate users so geo-blocking
  is too blunt. Revisit if the deploy is ever single-country.
- **mTLS on /api/agent/ws** -- agents auth via per-agent token +
  Tailscale ACL. mTLS would be belt-and-suspenders but the operational
  cost (cert distribution to every agent) doesn't match the threat model
  yet.
