import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: { target: "esnext" },
  clearScreen: false,
  // With `drift-server` running locally, `npm run dev` shows the remote UI against it.
  server: { port: 1420, strictPort: true, proxy: { "/api": "http://127.0.0.1:8793" } },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
