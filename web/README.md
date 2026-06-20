# Conduit Web UI

TypeScript web frontend for conduit. A running daemon embeds a web server
(`crates/server`) that bridges browser WebSocket clients onto the same
Command/Event protocol the TUI speaks — same core, same session, same
history replay.

## Stack

**SolidJS** (chosen after a three-framework evaluation against React and
Svelte). Solid's `createStore` + `reconcile` maps cleanly onto conduit's
full-snapshot event protocol, and its `onMount`/`onCleanup` lifecycle suits
the imperative xterm.js terminals. The reusable, framework-agnostic core
lives in `@conduit/shared`; the app is a thin Solid layer over it.

## Layout

```
web/
  shared/          @conduit/shared — framework-agnostic core
    src/protocol/  generated TS bindings (bun run gen:protocol)
    src/client.ts  ConduitClient: WS + reconnect + terminal ring buffers
    src/terminal.ts  xterm.js wiring (fit/webgl/unicode11, resize debounce)
    src/events.ts  externally-tagged JSON -> discriminated union
    src/diff.ts    unified-diff parser
    src/theme.css  base design tokens
  app/             the SolidJS application (full TUI parity)
```

## Running

```sh
# 1. Start a conduit session — its daemon serves ws://127.0.0.1:3001/ws
conduit -s dev

# 2. Run the app
cd web
bun install
bun run dev
```

From another machine on a trusted network (Tailscale), tunnel the daemon
and dev-server ports over SSH, or wait for the embedded production build
(`bun run build` → served by the daemon directly with password + TLS).

Set `CONDUIT_WEB_PORT` for a non-default daemon port and
`VITE_CONDUIT_WS_URL` to match. `CONDUIT_DISABLE_EMBEDDED_WEB=1` turns the
embedded server off.

## Protocol types

Generated from `crates/protocol` via ts-rs. Regenerate after protocol
changes:

```sh
bun run gen:protocol   # cargo test -p protocol --features ts with TS_RS_EXPORT_DIR
```

## Architecture notes

- Terminal bytes (`TerminalOutput`) never enter framework state: the shared
  client decodes them into a per-terminal ring buffer and writes straight to
  xterm.js. App state (workspaces, git, attention) flows through the store.
- The server replays a full snapshot on every connect; snapshot events are
  full replacements, so reconnect needs no special handling in app code.
- The web client never sends `SetRoute` — that steers the TUI's screen. Web
  routing is client-side.
- PTYs spawn at the browser terminal's real size: fit xterm first, then send
  `StartTerminal` with the fitted cols/rows.
- Some TUI client responsibilities fall away on web because xterm.js is a
  real terminal emulator: cursor-position-report handling, the
  passthrough-key multiplexing, and Alacritty/Vt100 core-switching.
