import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Emit a self-contained server bundle so Dockerfile.web can ship a
  // ~100-200 MB runtime image instead of vendoring the full node_modules
  // (1+ GB). The build copies node_modules into .next/standalone
  // automatically; the runner stage only needs that + .next/static + public.
  output: "standalone",
  async headers() {
    return [
      {
        source: "/(.*)",
        headers: [
          {
            key: "Content-Security-Policy",
            // Next's production bootstrap uses small inline scripts/styles;
            // keep those while constraining every fetch, frame, object, and
            // navigation target to the dashboard's intended boundaries.
            value:
              "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self' https: wss:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'; form-action 'self' https://github.com",
          },
        ],
      },
    ];
  },
};

export default nextConfig;
