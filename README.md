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
- [bun](https://bun.sh/) (to build the embedded web UI)
- **Linux desktop builds:** `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `librsvg2-dev`
- **Windows desktop builds:** the [MSVC build tools](https://visualstudio.microsoft.com/visual-studio-build-tools/)
  ("Desktop development with C++" — provides the linker + Windows SDK) and the
  [WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
  (preinstalled on Windows 11; install the Evergreen bootstrapper on Windows 10).
  Use the default `x86_64-pc-windows-msvc` target — `windows-rs`/`wry`/WebView2
  are best supported there.

#### Build

```sh
cd web && bun install && bun run build   # build the web UI first
cd .. && cargo build --release
```

#### Run

```sh
cargo run
# or
./target/release/conduit          # conduit.exe on Windows
```

## Usage

A *surface* (`tui`, `web`, `desktop`) is an interchangeable view onto a
*session* (a named daemon). Attach a surface to a session — creating it if it
doesn't exist — with `conduit <surface> attach <name>` (or the `-a <name>`
shorthand). Session management lives at the top level.

```
conduit                       Open the default surface (desktop, or TUI on headless builds)
conduit tui                   Terminal UI on an in-process core (no session)
conduit tui attach <name>     Attach the TUI to a session (created if missing)
conduit tui attach <name> -d  Start the session daemon in the background, don't open the UI
conduit web                   Serve the web UI for all running sessions (picker)
conduit web attach <name>     Serve the web UI pinned to one session
conduit desktop               Open the native desktop window (session picker)
conduit desktop attach <name> Open the desktop window pinned to one session
conduit list                  List active sessions          (alias: -l)
conduit remove <name>         Remove a session              (alias: -r <name>)
conduit version               Print version                 (alias: -v)
conduit help                  Print help                    (alias: -h)
```

Each surface accepts `-a <name>` as shorthand for `attach <name>`
(e.g. `conduit tui -a work`). `web` also takes `status`, `shutdown`, and
`set-password` subcommands.

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

`conduit web` runs a standalone web server (`web/`) that lists your running
sessions and attaches the browser to whichever you pick — `conduit tui attach`
for the browser. It connects to sessions over their existing daemon sockets, so
it drives already-running sessions and their live agents without restarting
them. Run it once; browse `http://localhost:3001`.

```sh
conduit tui attach work -d   # start your session(s) in the background
conduit web                  # then serve the web UI for all of them
conduit web attach work      # …or pin it to one session (created if missing)
```

For remote access, set a password and bind beyond localhost — both are
required, and TLS is used automatically (self-signed if no cert is provided):

```sh
conduit web set-password
CONDUIT_WEB_BIND=0.0.0.0 conduit web   # refused unless a password is set
```

### Desktop app

A plain `conduit` (no arguments) opens the web UI in a native OS window (system
webview via `wry`/`tao` — no Electron, no bundled browser). It runs the core and
web server in-process on a private loopback port, so no separate session daemon
is needed. `conduit desktop` does the same thing explicitly. The desktop UI is on
by default, so it links GTK/WebKit on Linux and the WebView2 runtime on Windows:

```sh
cd web && bun install && bun run build   # build the web UI first
cd .. && cargo run                       # then run from source
```

(From-source debug builds read the web UI from `web/app/dist/`; if it isn't
built you'll see a "web UI hasn't been built yet" placeholder. Released bundles
embed it at compile time, so this step is only needed for local dev.)

Releases ship double-clickable bundles — a macOS `.dmg`/`.app` and a Linux
`.deb`/`.AppImage` — that launch straight into the window.

**Headless / server builds:** compile with `--no-default-features` to drop the
GTK/WebKit link. There, a bare `conduit` falls back to the terminal UI and
`conduit web` runs without any desktop dependencies:

```sh
cargo build --release -p tui --no-default-features
```

### Config Paths

Conduit stores configuration under `~/.config/conduit/` (respects
`XDG_CONFIG_HOME`; on Windows, `%APPDATA%\conduit\`):

- `sessions.json` — session registry
- `workspaces.json` — default workspace persistence
- `workspaces.<session-name>.json` — per-session workspace state
- `sessions/<name>.sock` — per-session daemon socket (Unix). On Windows, named
  pipes (`\\.\pipe\conduit-session-<name>`) are used instead — no socket files.

## License

MIT
