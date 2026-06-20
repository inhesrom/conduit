# Conduit

<video src="https://github.com/user-attachments/assets/fbc7da6f-0f79-4cad-ab20-b997c3dd119f" controls width="600"></video>

A terminal-based multi-workspace manager built with Rust.

## Features

- **Multi-workspace management** with Git integration — branch tracking, status monitoring, and inline diffs
- **Embedded terminal sessions** (agent + shell tabs) via PTY, with full input passthrough
- **Attention system** that detects prompts, errors, and activity in terminal output
- **Session persistence** with a daemon/attach model for long-running workspaces
- **Web UI** with real-time WebSocket updates, served from an embedded HTTP server
- **Mouse support** and terminal scrollback via mouse wheel
- **Vim-style navigation** throughout the interface

## Architecture

Conduit is organized as a Cargo workspace with three crates:

| Crate | Description |
|---|---|
| `protocol` | Serializable types for IPC — workspace routing, attention levels, terminal kinds, and command/event enums |
| `core` | Application state management — workspaces, Git, terminal PTY spawning, attention detection, SSH, and the async event loop |
| `tui` | Terminal UI built with Ratatui — renders home/workspace screens, handles input, and manages sessions |

See [docs/repo-diagram.md](/home/ianhersom/repo/conduit/docs/repo-diagram.md) for a rendered repo diagram and runtime overview.

## Getting Started

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/inhesrom/conduit/master/install.sh | bash
```

Prebuilt binaries are available for:
- macOS (Apple Silicon)
- Linux (x86_64)

The installer places the `conduit` binary in `~/.local/bin`. Override with `CONDUIT_INSTALL_DIR`.

### Build from source

#### Prerequisites

- [Rust toolchain](https://rustup.rs/) (stable)

#### Build

```sh
cargo build --release
```

#### Run

```sh
cargo run
# or
./target/release/conduit
```

## Usage

```
conduit                    Local mode (no session)
conduit -s <name>          Create and start a named session
conduit -a <name>          Attach to an existing session
conduit -l                 List sessions
conduit -r <name>          Remove a session
conduit -d                 Detach (use with -s or -a)
```

## Key Bindings

### Global

| Key | Action |
|---|---|
| `q` | Quit |
| `Tab` / `Shift+Tab` | Cycle focus between sections |
| `Esc` | Exit focused section / go back |

### Home Screen

| Key | Action |
|---|---|
| `h` `j` `k` `l` / Arrow keys | Navigate workspaces |
| `Enter` | Open selected workspace |
| `n` | New workspace |
| `D` | Delete workspace |
| `!` | Toggle attention level |
| `g` | Refresh git status |

### Workspace Screen

| Key | Action |
|---|---|
| `1` `2` / `h` `l` | Switch terminal tabs |
| `n` | New shell tab |
| `x` | Close active tab |
| `r` | Rename tab |
| `a` / `A` | Start / stop agent terminal |
| `s` / `S` | Start / stop shell terminal |
| `g` | Refresh git |
| `j` `k` / Arrow keys | Navigate file list |
| `Enter` | Show diff for selected file |
| Mouse wheel | Scroll terminal output |

## Configuration

### Environment Variables

| Variable | Description | Default |
|---|---|---|
| `CONDUIT_WEB_PORT` | Embedded web server port (session/daemon mode; see `web/README.md`) | `3001` |
| `CONDUIT_WEB_BIND` | Address the web server binds to | `127.0.0.1` |
| `CONDUIT_WEB_TLS` | Force TLS on localhost (`on`); a self-signed cert is generated | — |
| `CONDUIT_WEB_CERT` / `CONDUIT_WEB_KEY` | Use a specific TLS cert/key (PEM) instead of self-signed | — |
| `SHELL` | Shell used for terminal sessions | `zsh` |

### Web UI

`conduit web serve` runs a standalone web server (`web/`) that lists your
running sessions and attaches the browser to whichever you pick — `conduit -a`
for the browser. It connects to sessions over their existing daemon sockets, so
it drives already-running sessions and their live agents without restarting
them. Run it once; browse `http://localhost:3001`.

```sh
conduit -s work                      # start your session(s) as usual
conduit web serve                    # then serve the web UI for all of them
conduit web serve --session work     # …or pin it to one session
```

For remote access, set a password and bind beyond localhost — both are
required, and TLS is used automatically (self-signed if no cert is provided):

```sh
conduit web set-password
CONDUIT_WEB_BIND=0.0.0.0 conduit web serve   # refused unless a password is set
```

### Desktop app

`conduit desktop` opens the same web UI in a native OS window (system webview
via `wry`/`tao` — no Electron, no bundled browser). It runs the core and web
server in-process on a private loopback port, so no separate session daemon is
needed. The desktop UI is compiled in behind a Cargo feature, keeping headless
server builds free of GTK/WebKit:

```sh
cd web && bun install && bun run build           # build the web UI first
cd .. && cargo run -p tui --features desktop -- desktop   # then run from source
```

(From-source debug builds read the web UI from `web/app/dist/`; if it isn't
built you'll see a "web UI hasn't been built yet" placeholder. Released bundles
embed it at compile time, so this step is only needed for local dev.)

Releases ship double-clickable bundles — a macOS `.dmg`/`.app` and a Linux
`.deb`/`.AppImage` — that launch straight into the window. Headless installs
(the `install.sh` tarball) are unaffected and never link GTK/WebKit.

### Config Paths

Conduit stores configuration under `~/.config/conduit/` (respects `XDG_CONFIG_HOME`):

- `sessions.json` — session registry
- `workspaces.json` — default workspace persistence
- `workspaces.<session-name>.json` — per-session workspace state

## License

MIT
