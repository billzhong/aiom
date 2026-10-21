import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { searchForWorkspaceRoot } from "vite"
import { defineConfig } from "vitest/config"

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { tsconfigPaths: true },
  server: {
    // `pnpm dev` and `pnpm preview` hand the dashboard WebSocket on to a local server.
    proxy: { "/ws": { target: "ws://localhost:3000", ws: true } },
    // The protocol test reads the node list shared with the Rust protocol crate.
    fs: { allow: [searchForWorkspaceRoot(process.cwd()), "../crates/protocol/fixtures"] },
  },
  build: {
    rolldownOptions: {
      output: {
        codeSplitting: {
          groups: [
            {
              debugName: "vendor",
              name(id) {
                if (/\/node_modules\/(react|react-dom|scheduler)\//.test(id)) return "vendor-react"
                if (/\/node_modules\/(recharts|d3-[^/]+)\//.test(id)) return "vendor-recharts"
                if (id.includes("/node_modules/country-flag-icons/")) return "vendor-flags"
                return null
              },
            },
          ],
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    // Independent of the URL in a local `.env`.
    env: { VITE_WS_URL: "ws://monitor.test/ws" },
    setupFiles: ["./src/test/setup.ts"],
    restoreMocks: true,
  },
})
