# Conduit

A terminal multi-repository agent manager. You register base **Repositories**, then spawn worktree-backed **Workspaces** — each one task, on its own branch, with its own agent — and review the resulting diff before opening a PR. Modeled on conductor.build, in the terminal.

## Language

**Repository**:
A registered base git repo, registered once per machine. Source-only — it is never worked in directly; it exists to branch Workspaces from. Has a detected default branch and an optional per-repo worktree root + default agent.
_Avoid_: project, base workspace, clone.

**Workspace**:
One unit of work: a git **Worktree** on its own branch, owned by exactly one **Repository**, with one agent terminal (and optional shell tabs) and a review state.
_Avoid_: tab, checkout, session.

**Worktree**:
The on-disk `git worktree` backing a Workspace, created under `<repo_parent>/.conduit-worktrees/<repo>/<slug>` by default. Distinct from `worktree_status`, the porcelain unstaged-state char.
_Avoid_: clone.

**ReadyForReview**:
A Workspace state, orthogonal to **AttentionLevel**: the agent has gone idle while the worktree has uncommitted-or-ahead changes and isn't awaiting input. Set by that heuristic or a manual toggle. Rendered as a steady magenta `◆`, never the attention flash.
_Avoid_: done, finished, notice.

**AttentionLevel**:
The existing per-Workspace flash state (`None`/`NeedsInput`/`Error`) signalling the user must look *now*. Separate from ReadyForReview — attention is "look now", readiness is "a unit of work completed".
_Avoid_: status, state.

**Session**:
Unchanged infrastructure concept — a named daemon process bound to a Unix socket. A process/isolation boundary, NOT a domain grouping. Repositories are global (above sessions); Workspaces are per-session.
_Avoid_: workspace, project.

## Relationships

- A **Repository** has many **Workspaces** (reference by `repository_id`).
- A **Workspace** has exactly one **Worktree**, one branch, one agent terminal, and an optional **ReadyForReview** state.
- **Repositories** live in a global `repositories.json`; **Workspaces** live per-session with a `repository_id` foreign key.
- A **Session** owns many **Workspaces** but does not own **Repositories**.

## Example dialogue

> **Dev:** "When I press ctrl+n on a Repository, what gets created?"
> **Domain expert:** "A new **Workspace** — Conduit fetches the repo's default branch, adds a **Worktree** on a fresh branch named from your task, and starts the agent there."
> **Dev:** "And when is it **ReadyForReview**?"
> **Domain expert:** "When the agent goes quiet with changes on disk and isn't waiting on you. That's different from **AttentionLevel** `NeedsInput`, which means it's blocked asking *you* something right now."

## Flagged ambiguities

- "Workspace" previously meant *the repo directory itself*. It was renamed: today's repo-directory concept is now **Repository**, and **Workspace** means a per-task worktree. (See ADR 0001.)
- "done"/"ready" was used loosely — resolved to **ReadyForReview**, an explicit state kept orthogonal to **AttentionLevel**. (See ADR 0002.)
