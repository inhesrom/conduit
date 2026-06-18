// Hand-written barrel for the ts-rs generated bindings (which are one file
// per type and carry no index). Regenerate the bindings with:
//   TS_RS_EXPORT_DIR=<repo>/web/shared/src/protocol cargo test -p protocol --features ts

export type { AttentionLevel } from "./AttentionLevel";
export type { BranchInfo } from "./BranchInfo";
export type { ChangedFile } from "./ChangedFile";
export type { CheckoutSource } from "./CheckoutSource";
export type { Command } from "./Command";
export type { CommitInfo } from "./CommitInfo";
export type { Event } from "./Event";
export type { GitState } from "./GitState";
export type { RemoteBranchInfo } from "./RemoteBranchInfo";
export type { RepositorySummary } from "./RepositorySummary";
export type { Route } from "./Route";
export type { SavedCommand } from "./SavedCommand";
export type { SshTarget } from "./SshTarget";
export type { TagInfo } from "./TagInfo";
export type { TerminalKind } from "./TerminalKind";
export type { WorkspaceSummary } from "./WorkspaceSummary";

// Rust type aliases (`pub type WorkspaceId = Uuid`) don't survive ts-rs.
export type WorkspaceId = string;
export type RepositoryId = string;
