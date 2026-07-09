import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type {
  PullRequestComment,
  PullRequestDetails,
  PullRequestDiffLine,
  PullRequestDiffSide,
  PullRequestRef,
  PullRequestSummary,
} from "@conduit/shared";
import { navigate } from "../router";
import { git } from "../state/git-actions";
import { prActions } from "../state/pr-actions";
import { repoName } from "../state/selectors";
import { store } from "../state/store";
import { StatusGlyph } from "../components/StatusGlyph";

interface Selection {
  path: string;
  side: PullRequestDiffSide;
  start: number;
  end: number;
}

function lineNumber(line: PullRequestDiffLine): number | null {
  if (line.side === "Left") return line.old_line ?? null;
  if (line.side === "Right") return line.new_line ?? null;
  return null;
}

function sideLabel(side: PullRequestDiffSide): string {
  return side === "Left" ? "left" : "right";
}

function selectionLabel(sel: Selection): string {
  const lo = Math.min(sel.start, sel.end);
  const hi = Math.max(sel.start, sel.end);
  return `${sel.path}:${lo === hi ? lo : `${lo}-${hi}`} ${sideLabel(sel.side)}`;
}

function checksLabel(details: PullRequestDetails): string {
  const c = details.checks;
  if (c.total === 0) return "checks n/a";
  if (c.failed > 0) return `${c.failed}/${c.total} failing`;
  if (c.pending > 0) return `${c.pending}/${c.total} pending`;
  return `${c.passed}/${c.total} passing`;
}

function commentLine(comment: PullRequestComment): number | null {
  if (comment.side === "Left") return comment.line ?? null;
  return comment.line ?? null;
}

function CommentItem(props: {
  wsId: string;
  pr: PullRequestRef;
  comment: PullRequestComment;
  compact?: boolean;
}) {
  const [replying, setReplying] = createSignal(false);
  const [reply, setReply] = createSignal("");
  const [editing, setEditing] = createSignal(false);
  const [editBody, setEditBody] = createSignal(props.comment.body);

  createEffect(() => {
    if (!editing()) setEditBody(props.comment.body);
  });

  const saveReply = () => {
    const body = reply().trim();
    if (!body) return;
    prActions.reply(props.wsId, props.pr, props.comment.target.id, body);
    setReply("");
    setReplying(false);
  };

  const saveEdit = () => {
    const body = editBody().trim();
    if (!body) return;
    prActions.editComment(props.wsId, props.pr, props.comment.target, body);
    setEditing(false);
  };

  const remove = () => {
    if (!window.confirm("Delete this comment?")) return;
    prActions.deleteComment(props.wsId, props.pr, props.comment.target);
  };

  return (
    <article class="pr-comment" classList={{ compact: props.compact }}>
      <header class="pr-comment-head">
        <span class="mono">{props.comment.author}</span>
        <span>{props.comment.target.kind === "Issue" ? "conversation" : "review"}</span>
        <span>{props.comment.updated_at || props.comment.created_at}</span>
        <span class="pr-spacer" />
        <Show when={props.comment.can_edit}>
          <button class="btn xs" onClick={() => setEditing((v) => !v)}>
            {editing() ? "Cancel" : "Edit"}
          </button>
        </Show>
        <Show when={props.comment.can_delete}>
          <button class="btn xs danger" onClick={remove}>
            Delete
          </button>
        </Show>
      </header>
      <Show
        when={editing()}
        fallback={<pre class="pr-comment-body">{props.comment.body}</pre>}
      >
        <textarea
          class="pr-textarea"
          value={editBody()}
          onInput={(e) => setEditBody(e.currentTarget.value)}
        />
        <div class="pr-form-actions">
          <button class="btn xs primary" disabled={!editBody().trim()} onClick={saveEdit}>
            Save
          </button>
        </div>
      </Show>
      <Show when={props.comment.target.kind === "Review"}>
        <div class="pr-form-actions">
          <button class="btn xs" onClick={() => setReplying((v) => !v)}>
            Reply
          </button>
        </div>
        <Show when={replying()}>
          <textarea
            class="pr-textarea sm"
            value={reply()}
            onInput={(e) => setReply(e.currentTarget.value)}
          />
          <div class="pr-form-actions">
            <button class="btn xs primary" disabled={!reply().trim()} onClick={saveReply}>
              Post reply
            </button>
          </div>
        </Show>
      </Show>
    </article>
  );
}

function EmptyPrState(props: {
  wsId: string;
  message?: string;
  setup?: boolean;
  candidates?: PullRequestSummary[];
}) {
  const [query, setQuery] = createSignal("");
  const submit = () => {
    const q = query().trim();
    if (q) prActions.load(props.wsId, null, q);
  };

  return (
    <div class="pr-empty">
      <p>{props.message ?? "No pull request found."}</p>
      <Show when={props.candidates?.length}>
        <div class="pr-candidates">
          <For each={props.candidates}>
            {(candidate) => (
              <div class="pr-candidate-row">
                <div class="pr-candidate-main">
                  <span class="mono">#{candidate.pr.number}</span>
                  <span class="pr-candidate-title">{candidate.title}</span>
                </div>
                <div class="pr-candidate-meta">
                  <span>{candidate.state}</span>
                  <span>
                    {candidate.base_ref} ← {candidate.head_ref}
                  </span>
                </div>
                <button
                  class="btn xs primary"
                  onClick={() => prActions.load(props.wsId, candidate.pr, null)}
                >
                  Load
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
      <div class="pr-lookup">
        <input
          class="modal-input mono"
          placeholder="PR number or URL"
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") submit();
          }}
        />
        <button class="btn primary" disabled={!query().trim()} onClick={submit}>
          Load
        </button>
      </div>
      <Show when={!props.setup}>
        <button class="btn" onClick={() => git.openPullRequest(props.wsId)}>
          Open PR
        </button>
      </Show>
    </div>
  );
}

export function PullRequestScreen(props: { id: string }) {
  const ws = () => store.workspaces.find((w) => w.id === props.id);
  const state = () => store.pullRequestsByWs[props.id] ?? { status: "idle", failureCount: 0 };
  const details = () => state().details;
  const shownDetails = () => (state().status === "candidates" ? undefined : details());
  const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
  const [query, setQuery] = createSignal("");
  const [editingMeta, setEditingMeta] = createSignal(false);
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  const [topComment, setTopComment] = createSignal("");
  const [selection, setSelection] = createSignal<Selection | null>(null);
  const [inlineBody, setInlineBody] = createSignal("");

  createEffect(() => {
    const d = details();
    if (!d) return;
    if (!selectedFile() && d.files.length > 0) setSelectedFile(d.files[0]!.path);
    if (!editingMeta()) {
      setTitle(d.title);
      setBody(d.body);
    }
  });

  let pollTimer: number | undefined;
  const clearPoll = () => {
    if (pollTimer !== undefined) window.clearTimeout(pollTimer);
    pollTimer = undefined;
  };
  const schedulePoll = () => {
    clearPoll();
    const st = state();
    const delay = st.status === "setup" || st.failureCount >= 3 ? 120_000 : 30_000;
    pollTimer = window.setTimeout(() => {
      const d = shownDetails();
      prActions.load(props.id, d?.pr ?? null, null);
      schedulePoll();
    }, delay);
  };

  onMount(() => {
    prActions.load(props.id);
    schedulePoll();
  });
  onCleanup(clearPoll);

  const fileDiff = createMemo(() => {
    const d = details();
    const file = selectedFile();
    if (!d || !file) return [];
    return d.diff.filter((line) => line.path === file);
  });

  const conversationComments = createMemo(() => {
    const d = details();
    if (!d) return [];
    return d.comments.filter((comment) => !comment.path);
  });

  const commentsForLine = (line: PullRequestDiffLine) => {
    const d = details();
    const no = lineNumber(line);
    if (!d || no == null || !line.side) return [];
    return d.comments.filter(
      (comment) =>
        comment.path === line.path &&
        comment.side === line.side &&
        commentLine(comment) === no,
    );
  };

  const commentCount = (path: string) =>
    details()?.comments.filter((comment) => comment.path === path).length ?? 0;

  const selectLine = (line: PullRequestDiffLine) => {
    const no = lineNumber(line);
    if (no == null || !line.side || line.kind === "Hunk" || line.kind === "Meta") return;
    const current = selection();
    if (current && current.path === line.path && current.side === line.side) {
      setSelection({ ...current, end: no });
    } else {
      setSelection({ path: line.path, side: line.side, start: no, end: no });
    }
  };

  const saveMeta = () => {
    const d = details();
    if (!d) return;
    prActions.update(props.id, d.pr, title(), body());
    setEditingMeta(false);
  };

  const saveTopComment = () => {
    const d = details();
    const text = topComment().trim();
    if (!d || !text) return;
    prActions.comment(props.id, d.pr, text);
    setTopComment("");
  };

  const saveInlineComment = () => {
    const d = details();
    const sel = selection();
    const text = inlineBody().trim();
    if (!d || !sel || !text) return;
    const start = Math.min(sel.start, sel.end);
    const end = Math.max(sel.start, sel.end);
    prActions.inlineComment(
      props.id,
      d.pr,
      sel.path,
      text,
      end,
      sel.side,
      start === end ? null : start,
      start === end ? null : sel.side,
    );
    setInlineBody("");
    setSelection(null);
  };

  const loadQuery = () => {
    const q = query().trim();
    if (!q) return;
    prActions.load(props.id, null, q);
  };
  const refresh = () => {
    const d = shownDetails();
    prActions.load(props.id, d?.pr ?? null, null);
  };

  return (
    <div class="ws-screen">
      <Show when={ws()} fallback={<div class="empty">Workspace not found.</div>}>
        <header class="ws-screen-head">
          <button class="back" title="Back to workspace" onClick={() => navigate({ name: "workspace", id: props.id })}>
            ←
          </button>
          <StatusGlyph ws={ws()!} />
          <span class="ws-screen-name">{ws()!.name}</span>
          <span class="ws-screen-repo mono">{repoName(ws()!)}</span>
          <Show when={ws()!.branch}>
            <span class="ws-screen-branch mono">{ws()!.branch}</span>
          </Show>
          <span class="ws-screen-spacer" />
          <button class="btn xs" onClick={refresh}>
            Refresh
          </button>
        </header>

        <Show
          when={shownDetails()}
          fallback={
            <EmptyPrState
              wsId={props.id}
              message={state().message}
              setup={state().status === "setup"}
              candidates={state().candidates}
            />
          }
        >
          {(d) => (
            <section class="pr-screen">
              <div class="pr-head">
                <div class="pr-title-row">
                  <Show
                    when={editingMeta()}
                    fallback={
                      <>
                        <h1 class="pr-title">{d().title}</h1>
                        <button class="btn xs" onClick={() => setEditingMeta(true)}>
                          Edit
                        </button>
                      </>
                    }
                  >
                    <input
                      class="pr-title-input"
                      value={title()}
                      onInput={(e) => setTitle(e.currentTarget.value)}
                    />
                    <button class="btn xs primary" disabled={!title().trim()} onClick={saveMeta}>
                      Save
                    </button>
                    <button class="btn xs" onClick={() => setEditingMeta(false)}>
                      Cancel
                    </button>
                  </Show>
                </div>
                <div class="pr-meta">
                  <span class="mono">#{d().pr.number}</span>
                  <span>{d().state}</span>
                  <span>{d().is_draft ? "draft" : "ready"}</span>
                  <span>{d().base_ref} ← {d().head_ref}</span>
                  <span>{checksLabel(d())}</span>
                  <Show when={d().review_decision}>
                    <span>{d().review_decision}</span>
                  </Show>
                  <Show when={state().status === "loading"}>
                    <span class="spinner" />
                  </Show>
                </div>
                <Show
                  when={editingMeta()}
                  fallback={<pre class="pr-body">{d().body || "No body."}</pre>}
                >
                  <textarea
                    class="pr-textarea pr-body-edit"
                    value={body()}
                    onInput={(e) => setBody(e.currentTarget.value)}
                  />
                </Show>
                <Show when={d().labels.length || d().reviewers.length}>
                  <div class="pr-meta secondary">
                    <For each={d().labels}>{(label) => <span>{label}</span>}</For>
                    <For each={d().reviewers}>{(reviewer) => <span>@{reviewer}</span>}</For>
                  </div>
                </Show>
              </div>

              <div class="pr-toolbar">
                <input
                  class="modal-input mono"
                  placeholder="PR number or URL"
                  value={query()}
                  onInput={(e) => setQuery(e.currentTarget.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") loadQuery();
                  }}
                />
                <button class="btn xs" disabled={!query().trim()} onClick={loadQuery}>
                  Load
                </button>
                <a class="btn xs" href={d().url} target="_blank" rel="noreferrer">
                  GitHub
                </a>
              </div>

              <div class="pr-main">
                <aside class="pr-files">
                  <div class="eyebrow">Files</div>
                  <ul class="flist">
                    <For each={d().files}>
                      {(file) => (
                        <li class="frow" classList={{ selected: selectedFile() === file.path }}>
                          <span class="frow-status">{file.status[0]?.toUpperCase() ?? "M"}</span>
                          <button
                            class="frow-path mono"
                            title={file.path}
                            onClick={() => {
                              setSelectedFile(file.path);
                              setSelection(null);
                            }}
                          >
                            {file.path}
                          </button>
                          <Show when={commentCount(file.path) > 0}>
                            <span class="pr-count mono">{commentCount(file.path)}</span>
                          </Show>
                        </li>
                      )}
                    </For>
                  </ul>
                </aside>

                <section class="pr-diff-pane">
                  <Show when={selection()}>
                    {(sel) => (
                      <div class="pr-inline-composer">
                        <span class="mono">{selectionLabel(sel())}</span>
                        <textarea
                          class="pr-textarea sm"
                          value={inlineBody()}
                          onInput={(e) => setInlineBody(e.currentTarget.value)}
                        />
                        <div class="pr-form-actions">
                          <button class="btn xs primary" disabled={!inlineBody().trim()} onClick={saveInlineComment}>
                            Comment
                          </button>
                          <button class="btn xs" onClick={() => setSelection(null)}>
                            Cancel
                          </button>
                        </div>
                      </div>
                    )}
                  </Show>
                  <div class="pr-diff">
                    <For each={fileDiff()}>
                      {(line) => {
                        const no = () => lineNumber(line);
                        const comments = () => commentsForLine(line);
                        return (
                          <div class="pr-diff-block">
                            <button
                              class="pr-diff-line"
                              classList={{
                                add: line.kind === "Add",
                                del: line.kind === "Delete",
                                hunk: line.kind === "Hunk",
                                meta: line.kind === "Meta",
                              }}
                              disabled={no() == null || !line.side}
                              onClick={() => selectLine(line)}
                              title={line.side ? `${line.path}:${no() ?? ""}` : ""}
                            >
                              <span class="pr-ln old mono">{line.old_line ?? ""}</span>
                              <span class="pr-ln new mono">{line.new_line ?? ""}</span>
                              <span class="pr-mark mono">
                                {line.kind === "Add" ? "+" : line.kind === "Delete" ? "-" : " "}
                              </span>
                              <span class="pr-code mono">{line.text}</span>
                            </button>
                            <Show when={comments().length > 0}>
                              <div class="pr-inline-comments">
                                <For each={comments()}>
                                  {(comment) => (
                                    <CommentItem
                                      wsId={props.id}
                                      pr={d().pr}
                                      comment={comment}
                                      compact
                                    />
                                  )}
                                </For>
                              </div>
                            </Show>
                          </div>
                        );
                      }}
                    </For>
                  </div>
                </section>

                <aside class="pr-conversation">
                  <div class="eyebrow">Conversation</div>
                  <div class="pr-new-comment">
                    <textarea
                      class="pr-textarea sm"
                      value={topComment()}
                      onInput={(e) => setTopComment(e.currentTarget.value)}
                    />
                    <div class="pr-form-actions">
                      <button class="btn xs primary" disabled={!topComment().trim()} onClick={saveTopComment}>
                        Comment
                      </button>
                    </div>
                  </div>
                  <For each={conversationComments()}>
                    {(comment) => <CommentItem wsId={props.id} pr={d().pr} comment={comment} />}
                  </For>
                </aside>
              </div>
            </section>
          )}
        </Show>
      </Show>
    </div>
  );
}
