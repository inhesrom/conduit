# ReadyForReview is orthogonal to AttentionLevel

The "agent is done and ready for PR review" signal is modeled as a separate per-Workspace boolean (`ready_for_review`) plus a manual override, NOT as a new variant of the existing `AttentionLevel` enum (which has an unused `Notice` slot that was tempting to repurpose).

## Why

Attention means "the user must look *now*" — it flashes orange/red and drives focus. Readiness means "a unit of work completed" — it should be a calm, steady indicator and a filter, not a flash. Conflating them would poison both the attention-flash logic and the review filter, and the two transition on different signals.

## Detection

Heuristic, computed in the core event loop: the agent terminal goes idle past the settle window AND the worktree has uncommitted-or-ahead changes AND it isn't `NeedsInput`. New agent output or input flips it back to working. A manual toggle (`Space`) sets it explicitly and sticks until the agent is active again.

## Consequences

- Rendered as a steady magenta `◆` in the sidebar, deliberately distinct from the attention flash.
- A `ReviewSource`-style seam is reserved so a future explicit agent-emitted marker (an OSC/sentinel parsed by the `AttentionDetector`) can supersede the heuristic without a protocol break.
