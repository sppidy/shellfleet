//! Defence-in-depth response headers.
//!
//! Cloudflare in front of the live deploy already injects most of
//! these, but this layer guarantees they're present on every response
//! including in dev / direct-Tailscale access where Cloudflare isn't
//! in the path. Only set headers we own — never overwrite an upstream
//! value (e.g. CSP) without thinking.

use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, Response, header},
    middleware::Next,
};

pub async fn middleware(req: Request, next: Next) -> Response<Body> {
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();

    // 1 year HSTS, include subdomains, preload-eligible. The live
    // deploy is HTTPS-only behind Cloudflare; an in-cluster client
    // on Tailscale also gets HTTPS via the cert pinned at the edge.
    h.entry(header::STRICT_TRANSPORT_SECURITY)
        .or_insert_with(|| HeaderValue::from_static("max-age=31536000; includeSubDomains"));
    // No legacy MIME sniffing.
    h.entry(header::X_CONTENT_TYPE_OPTIONS)
        .or_insert_with(|| HeaderValue::from_static("nosniff"));
    // No clickjacking. The dashboard isn't intended to be embedded.
    h.entry(header::X_FRAME_OPTIONS)
        .or_insert_with(|| HeaderValue::from_static("DENY"));
    // Don't leak the dashboard URL via the Referer header on outbound
    // links (e.g. to GitHub during the OAuth dance).
    h.entry(header::REFERRER_POLICY)
        .or_insert_with(|| HeaderValue::from_static("strict-origin-when-cross-origin"));
    // Conservative permissions: deny every powerful API by default.
    // The dashboard doesn't use camera, mic, geolocation, etc.
    h.entry(header::HeaderName::from_static("permissions-policy"))
        .or_insert_with(|| {
            HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=()")
        });
    // Content-Security-Policy. The dashboard self-hosts its JS/CSS
    // (Next.js standalone build) and only talks to its own origin
    // (HTTPS for /api/*, WSS for /ui/ws). Inline <style> tags ship
    // from styled-jsx and Next, so 'unsafe-inline' is required for
    // styles. xterm.js + Monaco both bundle into the same origin so
    // no extra script-src entries are needed.
    h.entry(header::CONTENT_SECURITY_POLICY).or_insert_with(|| {
        HeaderValue::from_static(
            "default-src 'self'; \
             script-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: https:; \
             font-src 'self' data:; \
             connect-src 'self' wss:; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self' https://github.com",
        )
    });

    resp
}
