import { defineConfig } from 'vitest/config';

// W0 safety-net: unit tests for the dashboard's pure logic (capability
// gating, CSRF/cookie handling). jsdom gives us `document.cookie` for the
// api.ts tests. No React plugin needed — these tests import plain modules,
// not rendered components; component/integration tests can add it later.
export default defineConfig({
  resolve: {
    alias: {
      '@': new URL('./src', import.meta.url).pathname,
    },
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.{test,spec}.{ts,tsx}'],
  },
});
