# Explicit session lifecycle CLI

The CLI is organized around two concepts: a **Surface** (`tui`, `web`, `desktop`) is a view onto a registered **Session** (a named daemon). Session lifecycle is explicit:

- `conduit new <name>` creates a missing session or revives stale registered state.
- `conduit attach <name>` / `conduit a <name>` attaches the default surface.
- `conduit <surface> attach <name>` / `conduit <surface> a <name>` attaches a specific surface.
- `conduit delete <name>` / `conduit d <name>` deletes a session after confirmation.
- `conduit delete <name> --yes` is the automation path.
- `conduit list` / `conduit l` lists registered sessions as `running` or `stale`.

Bare surfaces open choosers: `conduit` opens the default surface chooser (desktop when built, TUI in headless builds), while `conduit tui`, `conduit desktop`, and `conduit web` open that surface's chooser.

## Why

The previous grammar made `attach` create missing sessions. That was convenient, but it blurred two different operations: selecting existing state versus minting a new daemon and state namespace. It also left the TUI as the odd surface because bare `conduit tui` ran a sessionless in-process core while web/desktop showed pickers.

Making `new`, `attach`, and `delete` explicit gives users one model across CLI, TUI, desktop, and web:

- Creation is intentional and validates a public session slug.
- Attachment never surprises users by creating a typo-named session.
- Stale registered state can be revived without losing persisted workspaces, repositories, or foreground command state.
- Deletion has one destructive verb and one automation escape hatch.

## Decisions

- **Session names are strict slugs.** Valid names are non-empty ASCII letters, digits, `-`, and `_`. Invalid names are rejected rather than sanitized so two visible names cannot collide on one socket or state-file stem.
- **`new` owns creation.** Missing sessions are created. Stale registered sessions are revived with their state preserved. Running sessions are rejected with a suggestion to attach.
- **`attach` requires registration.** Missing sessions are rejected with a suggestion to run `conduit new <name>`. Stale registered sessions are revived before attaching. Revival failure leaves the registry and persisted state intact.
- **Bare surfaces open choosers.** There is no public `picker` command. The chooser is the default behavior of each unpinned surface.
- **The browser web chooser is attach-only.** `conduit web` can attach to and revive registered sessions, but it cannot create or delete sessions, even on localhost. The trusted desktop server can create/delete sessions.
- **Deletion is bounded to Conduit state.** Delete stops the daemon only when the recorded pid still looks like the expected `run-daemon --session-name <name>` process, removes the socket and registry entry, and removes per-session Conduit state files. It never deletes git repos, worktrees, branches, or user files.
- **Short aliases are subcommands, not flags.** `a` aliases `attach`, `d` aliases `delete`, and `l` aliases `list`. The old `-a`, `-l`, and `-r` flags are not retained.

## Rejected alternatives

- **Keep implicit-create `attach`.** Rejected because typos created durable sessions and the semantics did not line up with deletion, stale revival, or web's restricted authority.
- **Add `picker` as a public command.** Rejected because the chooser is a surface behavior, not a fourth concept users should learn.
- **Keep `create`/`remove` aliases.** Rejected to keep the public grammar small and to use one vocabulary in CLI, TUI, desktop, web, and docs.
- **Fallback from desktop to TUI based on display availability.** Rejected for this change. The default surface is build-time: desktop builds open desktop; headless builds open TUI.

## Consequences

- `conduit tui` no longer starts a sessionless in-process core; it opens the TUI chooser.
- `conduit web` never auto-attaches just because exactly one session exists.
- Pinned surface attach (`web attach`, `desktop attach`, `tui attach`) refuses missing sessions instead of creating them.
- Existing persisted per-session state remains keyed by the validated session slug.
