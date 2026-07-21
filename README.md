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

A *surface* (`tui`, `web`, `desktop`) is an interchangeable view onto a
*session* (a named daemon). Create sessions explicitly with `conduit new
<name>`, attach surfaces with `conduit <surface> attach <name>`, and open bare
surfaces to choose from registered sessions.

```
conduit                       Open the default surface chooser
conduit tui                   Open the TUI session chooser
conduit new <name>            Create a missing session or revive stale state
conduit attach <name>         Attach the default surface to a registered session
conduit a <name>              Alias for attach
conduit tui attach <name>     Attach the TUI to a registered session
conduit tui a <name>          Alias for tui attach
conduit web                   Serve the browser session chooser
conduit web attach <name>     Serve the web UI pinned to one session
conduit desktop               Open the native desktop window (session chooser)
conduit desktop attach <name> Open the desktop window pinned to one session
conduit delete <name>         Delete a session after confirmation
conduit delete <name> --yes   Delete without prompting
conduit d <name>              Alias for delete
conduit list                  List registered sessions      (alias: l)
conduit version               Print version                 (alias: -v)
conduit help                  Print help                    (alias: -h)
```

Session names are non-empty slugs: ASCII letters, digits, `-`, and `_`.
`attach` refuses unknown sessions and suggests `conduit new <name>`. `new`
refuses already-running sessions and suggests `conduit attach <name>`.
`web` also takes `status`, `shutdown`, and `set-password` subcommands.

## Key Bindings

The bordered footer shows the most useful controls for the current screen,
focused pane, selection, or modal. It adapts to the available width and keeps
whole key/action hints together.

Press `?` on an application-controlled screen to open shortcut help. **Current**
shows actions available in the captured context; press `Tab` for **All**, the
complete catalog grouped by surface and mode. Use arrows or `j`/`k` to scroll,
Page Up/Down for larger jumps, and `Esc` or `?` to close help. Help is
intentionally unavailable while typing into an application text field or while
normal input is being sent directly to a terminal.

The focused terminal normally receives keys literally. Use the configured
**Command mode key** (default `Ctrl+G`) to let Conduit handle pane navigation,
fullscreen, workspace commands, and other workspace shortcuts; use it again to
return to terminal input. Configured previous/next-workspace keys remain active
in normal terminal mode and are advertised in its footer.

`Ctrl+B` cycles the sidebar through expanded, repository rail, and hidden modes
from Conduit-controlled screens. In terminal-tab focus, `a` and `s` are aliases
for starting the active terminal; `A` and `S` are aliases for stopping it.
Previous/next workspace, command mode, scroll-to-bottom, and fullscreen bindings
can all be changed in Settings, and the footer and help show their effective
values.

## Configuration

### Environment Variables

| Variable | Description | Default |
|---|---|---|
| `CONDUIT_WEB_PORT` | Standalone web server port (see `web/README.md`) | `3001` |
| `CONDUIT_WEB_BIND` | Address the web server binds to | `127.0.0.1` |
| `CONDUIT_WEB_TLS` | Force TLS on localhost (`on`); a self-signed cert is generated | — |
| `CONDUIT_WEB_CERT` / `CONDUIT_WEB_KEY` | Use a specific TLS cert/key (PEM) instead of self-signed | — |
| `CONDUIT_SESSION_NAME` | Internal daemon/session context marker | — |
| `SHELL` | Shell used for terminal sessions | `zsh` |

### Web UI

`conduit web` runs a standalone web server (`web/`) that lists registered
sessions and attaches the browser to whichever you pick. It can revive stale
registered sessions, but it cannot create or delete sessions from the browser
surface. It connects to sessions over their existing daemon sockets, so it
drives already-running sessions and their live agents without restarting them.
Run it once; browse `http://localhost:3001`.

```sh
conduit new work             # create or revive a session daemon
conduit web                  # serve the browser chooser
conduit web attach work      # or pin it to a registered session
```

For remote access, set a password and bind beyond localhost — both are
required, and TLS is used automatically (self-signed if no cert is provided):

```sh
conduit web set-password
CONDUIT_WEB_BIND=0.0.0.0 conduit web   # refused unless a password is set
```

### Desktop app

A plain `conduit` (no arguments) opens the web UI in a native OS window (system
webview via `wry`/`tao` — no Electron, no bundled browser). It runs a trusted
local web server on a private loopback port and shows the same session chooser.
The desktop surface can create and delete sessions from the chooser. `conduit
desktop` does the same thing explicitly. The desktop UI is on by default, so it
links GTK/WebKit on Linux:

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
GTK/WebKit link. There, a bare `conduit` opens the TUI session chooser and
`conduit web` runs without any desktop dependencies:

```sh
cargo build --release -p tui --no-default-features
```

### Config Paths

Conduit stores configuration under `~/.config/conduit/` (respects `XDG_CONFIG_HOME`):

- `sessions.json` — session registry
- `workspaces.json` — default workspace persistence
- `workspaces.<session-name>.json` — per-session workspace state
- `repositories.<session-name>.json` — per-session repository registry
- `foreground_commands.<session-name>.json` — per-session foreground command resurrection state

## License

MIT
