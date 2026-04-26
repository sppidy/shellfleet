import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Emit a self-contained server bundle so Dockerfile.web can ship a
  // ~100-200 MB runtime image instead of vendoring the full node_modules
  // (1+ GB). The build copies node_modules into .next/standalone
  // automatically; the runner stage only needs that + .next/static + public.
  output: "standalone",
};

export default nextConfig;
