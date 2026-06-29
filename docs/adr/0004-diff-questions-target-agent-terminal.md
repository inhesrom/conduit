# Diff Questions target the agent terminal; the secondary agent is a Shell tab

A **Diff Question** — selecting lines in a Workspace's diff and asking about them — is delivered by injecting a composed prompt into an agent process via `SendTerminalInput`. The default (Enter) **starts a fresh agent** (a **Secondary Agent**: the Workspace's configured agent command in a new **Shell** tab) and delivers there once it's ready — deterministic, and never the current agent tab (which may hold a shell). Alt+Enter sends to the existing agent terminal to continue its context. The prompt is wrapped in **bracketed-paste** escapes (`\x1b[200~…\x1b[201~`, matching `crates/tui/src/main.rs`) so a multi-line prompt arrives as one atomic paste and a trailing CR submits — without it, a shell executes each line. This keeps Conduit's "no in-app LLM client — the agent is a process in a PTY" architecture and the ADR 0001 one-agent-terminal invariant intact.

## Considered Options

- **Call an LLM API directly from Conduit.** Rejected: Conduit has no HTTP client, no API-key handling, and no chat surface; the agent process already runs in the worktree and can edit files. A direct call would be architecturally foreign and far larger.
- **Add a true second agent terminal** (extend `WorkspaceTerminals`/`TerminalKind` to N agents, wired into attention/AgentActive/ReadyForReview). Rejected for v1: breaks the singleton `agent: Option<TerminalSession>` invariant from ADR 0001 and touches the attention/readiness heuristics.
- **Reuse the agent terminal + a Shell-tab secondary agent (chosen).** No protocol/core change; reuses `SendTerminalInput`, `agentCmdFor`, and the pending-prompt delivery path.

## Consequences

- The Secondary Agent runs in a Shell tab, so it does **not** participate in attention/`AgentActive`/`ReadyForReview` signals (those key off the singleton agent terminal). Acceptable for a throwaway parallel agent; revisit if secondary agents need first-class status.
- Web/desktop only — the selection UI lives in the web diff pane. The TUI diff pane is unaffected; a TUI version can reuse the same `SendTerminalInput` plumbing later.
