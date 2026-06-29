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

**AgentActive**:
A transient Workspace UI state: the agent terminal produced content and has not yet reached the 500 ms settle window. Separate from **AttentionLevel** (look now), **ReadyForReview** (reviewable work), and `agent_running` (process presence). Rendered as a light-blue spinner, suppressed when AttentionLevel is `NeedsInput` or `Error`.
_Avoid_: working, busy, running.

**Session**:
Unchanged infrastructure concept — a named daemon process bound to a Unix socket. A process/isolation boundary, NOT a domain grouping. Repositories are global (above sessions); Workspaces are per-session.
_Avoid_: workspace, project.

**Surface**:
An interchangeable view onto a **Session** — the terminal UI (`tui`), the browser UI (`web`), or the native window (`desktop`). Surfaces are orthogonal to Sessions: any surface can attach to any session. Opened with `conduit <surface>`; pinned to one session with `conduit <surface> attach <name>`. The CLI surface keyword is the first word after `conduit`.
_Avoid_: launch type, app version, mode.

**Attach**:
Connecting a **Surface** to a **Session**, creating the session (spawning its daemon) if it doesn't exist. `conduit <surface> attach <name>` is idempotent — it starts the daemon when absent and reattaches when present. Bare `conduit tui` is the exception: a Sessionless **Local** view on an in-process core. (See ADR 0003.)
_Avoid_: open, connect, launch.

**Diff Selection**:
A contiguous range of lines selected within one file's diff in the diff viewer (click a line-number gutter; shift-click to extend). The unit a **Diff Question** is built from. Bounded to a single file.
_Avoid_: highlight, region.

**Diff Question**:
A prompt composed from a **Diff Selection** — file path + new-file line range + the quoted lines + the user's question — injected into an agent as a bracketed paste. By default (Enter) it starts a **Secondary Agent**; Alt+Enter sends it to the existing agent terminal to continue that agent's context. (See ADR 0004.)
_Avoid_: comment, review note, annotation.

**Secondary Agent**:
A fresh agent process launched in a **Shell** tab (the Workspace's configured agent command, no shared history), distinct from the singleton agent terminal. Does not participate in **AttentionLevel**/**AgentActive**/**ReadyForReview** signals. (See ADR 0004.)
_Avoid_: second agent terminal, sub-agent.

## Relationships

- A **Repository** has many **Workspaces** (reference by `repository_id`).
- A **Workspace** has exactly one **Worktree**, one branch, one agent terminal, and optional **AgentActive** and **ReadyForReview** states.
- **Repositories** live in a global `repositories.json`; **Workspaces** live per-session with a `repository_id` foreign key.
- A **Session** owns many **Workspaces** but does not own **Repositories**.
- A **Surface** attaches to at most one **Session** at a time (the `web`/`desktop` picker switches between them); bare `conduit tui` runs Sessionless on an in-process core.
- A **Diff Question** is built from a **Diff Selection** in one Workspace and delivered to that Workspace's agent terminal, or to a **Secondary Agent** in a new Shell tab.

## Example dialogue

> **Dev:** "When I press ctrl+n on a Repository, what gets created?"
> **Domain expert:** "A new **Workspace** — Conduit fetches the repo's default branch, adds a **Worktree** on a fresh branch named from your task, and starts the agent there."
> **Dev:** "And when is it **ReadyForReview**?"
> **Domain expert:** "When the agent goes quiet with changes on disk and isn't waiting on you. That's different from **AttentionLevel** `NeedsInput`, which means it's blocked asking *you* something right now."
> **Dev:** "So the spinner means the agent is running?"
> **Domain expert:** "No. **AgentActive** means it has produced visible output inside the settle window. A quiet but still-running agent is not active."

## Flagged ambiguities

- "Workspace" previously meant *the repo directory itself*. It was renamed: today's repo-directory concept is now **Repository**, and **Workspace** means a per-task worktree. (See ADR 0001.)
- "done"/"ready" was used loosely — resolved to **ReadyForReview**, an explicit state kept orthogonal to **AttentionLevel**. (See ADR 0002.)
- "working"/"busy" is ambiguous — resolved to **AgentActive**, a transient output-within-settle-window signal rather than process liveness.
- "launch type"/"app version" for tui/web/desktop — resolved to **Surface**, with the unified `conduit <surface> [attach <name>]` CLI grammar. The old top-level `-s`/`-a` session flags were removed. (See ADR 0003.)
- "ask an agent" was ambiguous — Conduit has no in-app LLM; "agent" means the agent process in a PTY. A **Diff Question** injects a prompt into the existing agent terminal, or spawns a **Secondary Agent** (a Shell tab), never a direct API call. (See ADR 0004.)
