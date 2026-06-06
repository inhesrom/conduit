# Conduit native-GUI spike — GPUI vs Iced

Throwaway spike (branch `worktree-gui-spike`) to judge **GPUI** vs **Iced** as a
native desktop front-end for Conduit, against Conduit's real needs: an embedded
terminal fed by `core`'s PTY, the workspace list, and live event/state flow.
Both fronts were **built and run** on this machine (Wayland + Vulkan).

## What this spike is

Its own Cargo workspace under `experiments/gui-spike/`, deliberately **not** a
member of the Conduit workspace:

| Crate | Role |
|---|---|
| `bridge` | Owns a tokio runtime + Conduit's real `core` (`spawn_core()`); exposes a thread-safe `Command` sink and `Event` subscription. **Zero changes to `core`.** (~75 LOC) |
| `termgrid` | Framework-neutral terminal model wrapping Conduit's **own vendored `vt100`** (same parser the TUI uses). PTY bytes in → styled cell snapshot out, plus a key→bytes encoder + xterm-256 palette. (~230 LOC) |
| `smoke` | Headless proof that `core` is fully reusable with no UI. |
| `iced-front` | Iced front-end: workspace list + live terminal + input. (~290 LOC) |
| `gpui-front` | GPUI front-end, **its own isolated workspace** (heavy git-only dep). (~310 LOC) |

Both fronts share `bridge` + `termgrid` verbatim. The only per-framework code is:
translate `termgrid::CellSnap` colors into the framework's primitives, map native
key events onto `termgrid::Key`, and bridge the `Event` stream into the UI loop.

## Proven facts (with running code)

1. **Backend reuse is ~total.** `bridge` depends on `conduit_core` + `protocol`
   by path with **no patches and no edits to core**. `core` pulls in no
   vt100/ratatui, so it drops straight into a new front-end.
2. **`smoke`** starts core, adds a workspace, starts a PTY shell, streams output
   back via `Event::TerminalOutput` — no TUI involved.
3. **Both fronts work end-to-end through the GUI.** Each boots → receives
   `WorkspaceList` → auto-starts a shell → renders the vt100 grid → a scripted
   `SendTerminalInput` round-trips (terminal echoes `SPIKE_INPUT_OK ✓`,
   confirming key → core → PTY → `TerminalOutput` → grid). Both opened a window
   on Wayland with no GPU/surface error.
4. **Sandboxing:** the bridge sets `CONDUIT_SESSION_NAME=gui-spike` and clears
   that state file on start, so persistence stays in `workspaces.gui-spike.json`
   — the real workspace list is never touched.

## Key architectural finding

Conduit **inverts** the usual terminal-widget model. Off-the-shelf widgets
(`iced_term`, `gpui_terminal`) spawn and own their *own* PTY via
`alacritty_terminal`. Conduit's `core` already owns the PTY and streams bytes via
events, so neither widget fits — the right approach (what this spike does, and
what Zed itself does) is to **render a grid fed by external bytes**. Hence
`termgrid`, and hence the widget crates are unused. This is the single biggest
thing to know before picking a framework: you're building a grid renderer either
way, so "does it ship a terminal widget" is a non-criterion.

## Comparison (measured in this spike)

| Axis | Iced | GPUI |
|---|---|---|
| Source | crates.io `iced = "0.13"` | git-only `zed-industries/zed` (rev `5c1f18b`) + `gpui_platform` |
| First build | clean, ~minutes (wgpu/winit/cosmic-text) | heavy — **exceeded a 10-min build window once** mid-compile, resumed incrementally; iteration after that is ~1.5s |
| Linux setup gotchas | enable `tokio` feature for `time::every` | must add **both** `gpui` + `gpui_platform`, and enable `gpui_platform`'s `wayland`/`x11` features (default = none) |
| API churn hit | `Text` has no `underline` in 0.13 (rich-text only) | `Application::new()` removed → `gpui_platform::application()`; `flex_grow()` now takes an arg |
| Event bridge | `Subscription::run` over a stream of broadcast `Event`s | `cx.spawn(async \|cx: &mut AsyncApp\| …)` awaiting the broadcast receiver, `weak.update` into the view |
| Input | `keyboard::on_key_press` → `termgrid::Key` | `observe_keystrokes` (`Keystroke.key`/`key_char`/`modifiers`) → `termgrid::Key` |
| Grid render | coalesced colored `text` runs in a `column` | coalesced `div`s with `bg`/`text_color`/`font_weight` |
| Stability in spike | rock-solid across many runs | **pre-1.0 quirks**: root-view state reinitialized once (duplicate auto-select even with a flag), and the app self-exited early in this virtual-display session |
| Aesthetics | clean, themeable, slightly generic | top-tier (Zed-class) once styled |

## Honest read

- **Integration cost is basically equal and small.** Because `bridge` +
  `termgrid` are shared, each front is ~300 LOC of glue. Both bridge async cleanly
  (Iced via subscriptions, GPUI via `cx.spawn`). Neither was hard to wire to core.
- **The real differentiator is friction vs. polish.** Iced: stable, on crates.io,
  trivial to build, behaved perfectly — at some cost to "wow". GPUI: best-looking
  and the closest precedent for Conduit (Zed's own terminal, Paneflow), but
  git-only, pre-1.0, a heavy first build, platform feature flags, real API churn
  between revs, and I hit two lifecycle quirks even in this tiny app.

## Recommendation

- **Ship-soon / low-risk → Iced.** It was the dependable one here: crates.io,
  fast build, stable runtime. The grid renderer is the only real work and it's done.
- **Max polish, can absorb churn → GPUI.** The integration is genuinely small once
  it builds, and it's the natural fit for a Zed-adjacent dev tool. But budget for:
  **pinning a specific gpui git rev** (vendor/patch as needed), tracking breaking
  changes, and chasing pre-1.0 behaviors like the ones above.
- **Either way:** keep the `bridge` + `termgrid` split. It's what made this a
  ~300-LOC-per-front comparison instead of two rewrites, and it's the right shape
  for the real thing too.

## How to run

```sh
cd experiments/gui-spike
# headless proof of backend reuse:
SHELL=/bin/bash cargo run -p gui-spike-smoke
# Iced front (opens a window; auto-starts a shell in a demo workspace):
SHELL=/bin/bash cargo run -p iced-front
# GPUI front (separate workspace). Logs are line-buffered; if piping, redirect
# to a file since the window event loop may be SIGTERM'd before a pipe flushes:
cd gpui-front && SHELL=/bin/bash cargo run > /tmp/gpui.log 2>&1
```

Each front prints its event flow (`[iced]` / `[gpui]`): boot → `WorkspaceList`
→ `select`/`StartTerminal` → `TerminalStarted` → first `TerminalOutput` →
`SPIKE_INPUT_OK ✓`.
