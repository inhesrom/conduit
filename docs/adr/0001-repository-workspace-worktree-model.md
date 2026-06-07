# Repository (source-only) / Workspace (worktree) domain model

To make Conduit work like conductor.build, we renamed the single existing entity — `Workspace`, which meant "an arbitrary directory git runs in" — to **Repository**, and introduced a new **Workspace** = one git worktree + branch + agent + task belonging to a Repository. Repositories are *source-only* (never worked in directly); all work happens in worktree-backed Workspaces created via `ctrl+n`.

## Considered Options

- **Keep `Workspace` = repo, add a child `Worktree` entity.** Less churn (the `WorkspaceId` type threads through ~40 protocol variants), but the nouns would permanently diverge from conductor.build and collide with the existing `worktree_status` porcelain field.
- **Rename to Repository + Workspace (chosen).** Maximum rename churn now, but the vocabulary matches conductor.build 1:1 so docs and user intuition transfer, and the user's own phrasing already mapped to this two-tier model.

## Consequences

- `RepositoryId` added; `WorkspaceId` now identifies a worktree-workspace.
- Repositories persist globally (`repositories.json`); Workspaces stay per-session with a `repository_id` FK.
- First launch migrates legacy `workspaces.json` entries to Repositories (git roots only); no Workspaces are auto-created.
- The base repo checkout is intentionally not surfaced as a workspace — "every task is a worktree."
