# PR Viewer uses GitHub CLI as the auth/API boundary

The **PR Viewer** is a web/desktop Workspace sub-route for hosted pull request review. It loads GitHub PR metadata, status/check summaries, files, conversation comments, inline review comments, and supports immediate title/body edits plus top-level, inline, reply, edit, and delete comment mutations.

## Decision

- V1 supports GitHub only, behind generic protocol types (`PullRequestRef`, `PullRequestDetails`, `PullRequestComment`, etc.) so other providers do not leak into the web route shape.
- Core uses `gh` as the API and authentication boundary. Local Workspaces run local `gh`; SSH-backed Workspaces run `gh` on the Workspace host through `ssh::build_command`, matching Conduit's SSH-first behavior.
- GitHub's PR diff is the source of truth for inline comment coordinates. The viewer renders the GitHub PR diff and validates inline comment targets against that diff before posting.
- The PR identity is detected on route entry with `gh pr view` and is not persisted onto the Workspace.
- The Review tab remains the local branch-diff/preflight view. PR Viewer is not a new **Surface**; it is a route in the web/desktop Surface.

## Consequences

- Users authenticate once with `gh auth login` in the environment where the Workspace runs. SSH Workspaces require `gh` to be installed and authenticated on the remote host.
- Conduit avoids storing GitHub tokens and avoids implementing a parallel provider auth stack in v1.
- TUI parity is deferred. Existing TUI Open PR behavior remains separate from the web/desktop PR Viewer.

## Future Work

- Full PR controls: labels, reviewers, assignees, draft/ready, merge, close/reopen, update branch.
- Chronological timeline and review state transitions.
- Draft review, approve, and request-changes flows.
- Repository PR picker for loading PRs not associated with the current branch.
- TUI parity.
- Rendered markdown for PR bodies/comments.
- Non-GitHub providers.
