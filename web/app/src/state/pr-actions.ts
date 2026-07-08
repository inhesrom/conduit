import type {
  PullRequestCommentTarget,
  PullRequestDiffSide,
  PullRequestRef,
} from "@conduit/shared";
import { client } from "../client";
import { setStore, store } from "./store";

function markLoading(id: string): void {
  const prev = store.pullRequestsByWs[id];
  setStore("pullRequestsByWs", id, {
    status: "loading",
    details: prev?.details,
    candidates: prev?.candidates,
    message: prev?.message,
    failureCount: prev?.failureCount ?? 0,
    updatedAt: prev?.updatedAt,
  });
}

export const prActions = {
  load: (id: string, pr?: PullRequestRef | null, query?: string | null) => {
    markLoading(id);
    return client.send({ LoadPullRequest: { id, pr: pr ?? null, query: query ?? null } });
  },

  update: (id: string, pr: PullRequestRef, title: string | null, body: string | null) =>
    client.send({ UpdatePullRequest: { id, pr, title, body } }),

  comment: (id: string, pr: PullRequestRef, body: string) =>
    client.send({ CreatePullRequestComment: { id, pr, body } }),

  inlineComment: (
    id: string,
    pr: PullRequestRef,
    path: string,
    body: string,
    line: number,
    side: PullRequestDiffSide,
    startLine: number | null,
    startSide: PullRequestDiffSide | null,
  ) =>
    client.send({
      CreatePullRequestInlineComment: {
        id,
        pr,
        path,
        body,
        line,
        side,
        start_line: startLine,
        start_side: startSide,
      },
    }),

  reply: (id: string, pr: PullRequestRef, commentId: string, body: string) =>
    client.send({ ReplyPullRequestComment: { id, pr, comment_id: commentId, body } }),

  editComment: (id: string, pr: PullRequestRef, target: PullRequestCommentTarget, body: string) =>
    client.send({ EditPullRequestComment: { id, pr, target, body } }),

  deleteComment: (id: string, pr: PullRequestRef, target: PullRequestCommentTarget) =>
    client.send({ DeletePullRequestComment: { id, pr, target } }),
};
