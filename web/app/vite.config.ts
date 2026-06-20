import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Same-origin model: the app always talks to `/ws` and `/api` on its own
// host. In production the daemon serves both the assets and those routes; in
// dev, Vite proxies them to the daemon so the browser still sees one origin
// (cookies and wss upgrade just work, including from another machine hitting
// this dev server over Tailscale).
export default defineConfig({
  plugins: [solid()],
  server: {
    port: 5180,
    host: true,
    proxy: {
      "/ws": { target: "ws://127.0.0.1:3001", ws: true },
      "/api": { target: "http://127.0.0.1:3001" },
    },
  },
});
