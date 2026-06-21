# Unified surface + session CLI grammar

The CLI is organized around two concepts: a **Surface** (`tui`, `web`, `desktop`) is a view onto a **Session** (a named daemon). The grammar is `conduit <surface> [attach <name>]` for views and top-level verbs (`list`, `remove`, `version`, `help`, `update`, `reinstall`) for session/meta management, each with a short-flag twin (`-l`, `-r`, `-v`, `-h`, `-u`). Parsing moved from a hand-rolled `std::env::args()` loop to `clap` (derive).

## Why

The old CLI mixed top-level session flags (`-s`/`-a`/`-l`/`-r`) with ad-hoc subcommands (`tui`, `desktop`, `web serve …`) — inconsistent across launch surfaces and hard to extend. Naming the surface explicitly and making `attach` work uniformly across all three surfaces gives one mental model: pick a surface, optionally attach it to a session. `clap` generates help/usage/version and validates the nested tree, replacing ~180 lines of manual parsing.

## Decisions

- **`attach` is create-if-missing.** `conduit <surface> attach <name>` is idempotent: it spawns the session daemon when absent and reattaches when present (via `ensure_session_running`). There is no separate `new`/`create` verb — one verb covers the common case.
- **Bare `conduit tui` stays Sessionless.** It runs an in-process core with no daemon (the TUI has no session picker). Bare `conduit web`/`conduit desktop` keep the all-sessions picker. This surface-level asymmetry is intentional.
- **Clean break.** The old bare `-s`/`-a` flags and `web serve` subcommand were removed (no deprecation aliases) — pre-1.0, and the cohesion is worth the break.
- **Each surface accepts `-a <name>` as shorthand** for the `attach <name>` subcommand, so both `conduit tui attach work` and `conduit tui -a work` resolve to the same session.
- **Version is `-v`** (with `-V`/`--version`/`version` as aliases), overriding clap's default `-V`-only convention, because the project had no verbose flag to conflict with.

## Consequences

- The internal daemon re-exec moved from the `--run-daemon --session-name <n>` flag form to a hidden `run-daemon` subcommand; `spawn_daemon_process` and the `is_expected_daemon_process` kill-guard cmdline check were updated to match (the guard tolerates both old and new forms).
- `web attach <name>` and `desktop attach <name>` now `ensure_session_running` before serving, so pinning a surface to an absent session creates it — consistent with the TUI.
- `serve_desktop` gained a `pinned_session` parameter and `conduit_desktop::run` a `session` argument, threaded from `conduit desktop attach <name>`.
