use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type WorkspaceId = Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SshTarget {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Route {
    Home,
    Workspace { id: WorkspaceId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AttentionLevel {
    None,
    Notice,
    NeedsInput,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TerminalKind {
    Agent,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedCommand {
    pub argv: Vec<String>,
    pub cwd: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub name: String,
    pub path: String,
    pub branch: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub dirty_files: usize,
    pub attention: AttentionLevel,
    pub agent_running: bool,
    pub shell_running: bool,
    pub last_activity_unix_ms: u64,
    #[serde(default)]
    pub ssh_host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub index_status: char,
    pub worktree_status: char,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub message: String,
    pub author: String,
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub is_head: bool,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteBranchInfo {
    pub full_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub hash: String,
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GitState {
    pub branch: Option<String>,
    pub upstream: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub changed: Vec<ChangedFile>,
    pub recent_commits: Vec<CommitInfo>,
    pub local_branches: Vec<BranchInfo>,
    pub remote_branches: Vec<RemoteBranchInfo>,
    #[serde(default)]
    pub tags: Vec<TagInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    SetRoute(Route),
    AddWorkspace {
        name: String,
        path: String,
        #[serde(default)]
        ssh: Option<SshTarget>,
    },
    RemoveWorkspace {
        id: WorkspaceId,
    },
    RenameWorkspace {
        id: WorkspaceId,
        name: String,
    },
    MoveWorkspace {
        id: WorkspaceId,
        delta: i32,
    },
    SetAttention {
        id: WorkspaceId,
        level: AttentionLevel,
    },
    ClearAttention {
        id: WorkspaceId,
    },
    RefreshGit {
        id: WorkspaceId,
    },
    LoadDiff {
        id: WorkspaceId,
        file: String,
    },
    LoadCommitDiff {
        id: WorkspaceId,
        hash: String,
    },
    LoadCommitFiles {
        id: WorkspaceId,
        hash: String,
    },
    LoadCommitFileDiff {
        id: WorkspaceId,
        hash: String,
        file: String,
    },
    GitStageFile {
        id: WorkspaceId,
        file: String,
    },
    GitUnstageFile {
        id: WorkspaceId,
        file: String,
    },
    GitStageAll {
        id: WorkspaceId,
    },
    GitUnstageAll {
        id: WorkspaceId,
    },
    GitCommit {
        id: WorkspaceId,
        message: String,
    },
    GitCheckoutBranch {
        id: WorkspaceId,
        branch: String,
    },
    GitCheckoutRemoteBranch {
        id: WorkspaceId,
        remote_branch: String,
        local_name: String,
    },
    GitCreateBranch {
        id: WorkspaceId,
        branch: String,
    },
    GitDeleteLocalBranch {
        id: WorkspaceId,
        branch: String,
    },
    GitDeleteRemoteBranch {
        id: WorkspaceId,
        remote: String,
        branch: String,
    },
    GitPush {
        id: WorkspaceId,
    },
    GitPull {
        id: WorkspaceId,
    },
    GitFetch {
        id: WorkspaceId,
    },
    GitDiscardFile {
        id: WorkspaceId,
        file: String,
    },
    GitDiscardAll {
        id: WorkspaceId,
    },
    GitStash {
        id: WorkspaceId,
        message: Option<String>,
    },
    GitStashPullPop {
        id: WorkspaceId,
    },
    GitStashAll {
        id: WorkspaceId,
    },
    StartTerminal {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
        cmd: Vec<String>,
    },
    StopTerminal {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
    },
    SendTerminalInput {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
        data_b64: String,
    },
    ResizeTerminal {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
        cols: u16,
        rows: u16,
    },
    ClearShellResurrection {
        id: WorkspaceId,
        tab_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    WorkspaceList {
        items: Vec<WorkspaceSummary>,
    },
    WorkspaceGitUpdated {
        id: WorkspaceId,
        git: GitState,
    },
    WorkspaceDiffUpdated {
        id: WorkspaceId,
        file: String,
        diff: String,
    },
    CommitFilesLoaded {
        id: WorkspaceId,
        hash: String,
        files: Vec<String>,
    },
    WorkspaceAttentionChanged {
        id: WorkspaceId,
        level: AttentionLevel,
    },
    TerminalStarted {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
    },
    TerminalExited {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
        code: Option<i32>,
    },
    TerminalOutput {
        id: WorkspaceId,
        kind: TerminalKind,
        #[serde(default)]
        tab_id: Option<String>,
        data_b64: String,
    },
    GitActionResult {
        id: WorkspaceId,
        action: String,
        success: bool,
        message: String,
    },
    ShellForegroundChanged {
        id: WorkspaceId,
        tab_id: String,
        command: Option<SavedCommand>,
    },
    ShellResurrectionChanged {
        id: WorkspaceId,
        tab_id: String,
        command: Option<SavedCommand>,
    },
    Error {
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn round_trip<
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    >(
        val: &T,
    ) {
        let json = serde_json::to_string(val).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(*val, back);
    }

    #[test]
    fn ssh_target_round_trip() {
        round_trip(&SshTarget {
            host: "h".into(),
            user: Some("u".into()),
            port: Some(22),
        });
        round_trip(&SshTarget {
            host: "h".into(),
            user: None,
            port: None,
        });
    }

    #[test]
    fn route_round_trip() {
        round_trip(&Route::Home);
        round_trip(&Route::Workspace { id: Uuid::new_v4() });
    }

    #[test]
    fn attention_level_round_trip() {
        for level in [
            AttentionLevel::None,
            AttentionLevel::Notice,
            AttentionLevel::NeedsInput,
            AttentionLevel::Error,
        ] {
            round_trip(&level);
        }
    }

    #[test]
    fn terminal_kind_round_trip() {
        round_trip(&TerminalKind::Agent);
        round_trip(&TerminalKind::Shell);
    }

    #[test]
    fn workspace_summary_round_trip() {
        round_trip(&WorkspaceSummary {
            id: Uuid::new_v4(),
            name: "test".into(),
            path: "/tmp/test".into(),
            branch: Some("main".into()),
            ahead: Some(1),
            behind: Some(2),
            dirty_files: 3,
            attention: AttentionLevel::None,
            agent_running: true,
            shell_running: false,
            last_activity_unix_ms: 12345,
            ssh_host: Some("remote".into()),
        });
    }

    #[test]
    fn changed_file_round_trip() {
        round_trip(&ChangedFile {
            path: "foo.rs".into(),
            index_status: 'M',
            worktree_status: ' ',
        });
    }

    #[test]
    fn commit_info_round_trip() {
        round_trip(&CommitInfo {
            hash: "abc123".into(),
            message: "fix".into(),
            author: "dev".into(),
            date: "2h ago".into(),
        });
    }

    #[test]
    fn branch_info_round_trip() {
        round_trip(&BranchInfo {
            name: "main".into(),
            is_head: true,
            ahead: Some(1),
            behind: None,
        });
    }

    #[test]
    fn tag_info_round_trip() {
        round_trip(&TagInfo {
            name: "v1.0".into(),
            hash: "abc".into(),
            date: "1d ago".into(),
        });
    }

    #[test]
    fn git_state_round_trip() {
        round_trip(&GitState::default());
        round_trip(&GitState {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            ahead: Some(1),
            behind: Some(0),
            changed: vec![ChangedFile {
                path: "f.rs".into(),
                index_status: 'M',
                worktree_status: ' ',
            }],
            recent_commits: vec![CommitInfo {
                hash: "a".into(),
                message: "m".into(),
                author: "a".into(),
                date: "d".into(),
            }],
            local_branches: vec![BranchInfo {
                name: "main".into(),
                is_head: true,
                ahead: None,
                behind: None,
            }],
            remote_branches: vec![RemoteBranchInfo {
                full_name: "origin/main".into(),
            }],
            tags: vec![TagInfo {
                name: "v1".into(),
                hash: "h".into(),
                date: "d".into(),
            }],
        });
    }

    #[test]
    fn command_variants_round_trip() {
        let id = Uuid::new_v4();
        let commands = vec![
            Command::SetRoute(Route::Home),
            Command::SetRoute(Route::Workspace { id }),
            Command::AddWorkspace {
                name: "ws".into(),
                path: "/p".into(),
                ssh: None,
            },
            Command::AddWorkspace {
                name: "ws".into(),
                path: "/p".into(),
                ssh: Some(SshTarget {
                    host: "h".into(),
                    user: Some("u".into()),
                    port: Some(22),
                }),
            },
            Command::RemoveWorkspace { id },
            Command::RenameWorkspace {
                id,
                name: "n".into(),
            },
            Command::MoveWorkspace { id, delta: 1 },
            Command::MoveWorkspace { id, delta: -1 },
            Command::SetAttention {
                id,
                level: AttentionLevel::Error,
            },
            Command::ClearAttention { id },
            Command::RefreshGit { id },
            Command::LoadDiff {
                id,
                file: "f".into(),
            },
            Command::LoadCommitDiff {
                id,
                hash: "h".into(),
            },
            Command::LoadCommitFiles {
                id,
                hash: "h".into(),
            },
            Command::LoadCommitFileDiff {
                id,
                hash: "h".into(),
                file: "f".into(),
            },
            Command::GitStageFile {
                id,
                file: "f".into(),
            },
            Command::GitUnstageFile {
                id,
                file: "f".into(),
            },
            Command::GitStageAll { id },
            Command::GitUnstageAll { id },
            Command::GitCommit {
                id,
                message: "m".into(),
            },
            Command::GitCheckoutBranch {
                id,
                branch: "b".into(),
            },
            Command::GitCheckoutRemoteBranch {
                id,
                remote_branch: "origin/b".into(),
                local_name: "b".into(),
            },
            Command::GitCreateBranch {
                id,
                branch: "b".into(),
            },
            Command::GitPush { id },
            Command::GitPull { id },
            Command::GitFetch { id },
            Command::GitDiscardFile {
                id,
                file: "f".into(),
            },
            Command::GitDiscardAll { id },
            Command::GitStash {
                id,
                message: Some("msg".into()),
            },
            Command::GitStash { id, message: None },
            Command::GitStashPullPop { id },
            Command::GitStashAll { id },
            Command::StartTerminal {
                id,
                kind: TerminalKind::Agent,
                tab_id: None,
                cmd: vec!["bash".into()],
            },
            Command::StartTerminal {
                id,
                kind: TerminalKind::Shell,
                tab_id: Some("t".into()),
                cmd: vec![],
            },
            Command::StopTerminal {
                id,
                kind: TerminalKind::Agent,
                tab_id: None,
            },
            Command::SendTerminalInput {
                id,
                kind: TerminalKind::Shell,
                tab_id: None,
                data_b64: "aGVsbG8=".into(),
            },
            Command::ResizeTerminal {
                id,
                kind: TerminalKind::Shell,
                tab_id: None,
                cols: 80,
                rows: 24,
            },
            Command::ClearShellResurrection {
                id,
                tab_id: "shell".into(),
            },
        ];
        for cmd in &commands {
            round_trip(cmd);
        }
    }

    #[test]
    fn event_variants_round_trip() {
        let id = Uuid::new_v4();
        let events = vec![
            Event::WorkspaceList { items: vec![] },
            Event::WorkspaceGitUpdated {
                id,
                git: GitState::default(),
            },
            Event::WorkspaceDiffUpdated {
                id,
                file: "f".into(),
                diff: "d".into(),
            },
            Event::CommitFilesLoaded {
                id,
                hash: "h".into(),
                files: vec!["a".into()],
            },
            Event::WorkspaceAttentionChanged {
                id,
                level: AttentionLevel::NeedsInput,
            },
            Event::TerminalStarted {
                id,
                kind: TerminalKind::Agent,
                tab_id: None,
            },
            Event::TerminalExited {
                id,
                kind: TerminalKind::Shell,
                tab_id: Some("t".into()),
                code: Some(0),
            },
            Event::TerminalExited {
                id,
                kind: TerminalKind::Shell,
                tab_id: None,
                code: None,
            },
            Event::TerminalOutput {
                id,
                kind: TerminalKind::Agent,
                tab_id: None,
                data_b64: "b64".into(),
            },
            Event::GitActionResult {
                id,
                action: "push".into(),
                success: true,
                message: "ok".into(),
            },
            Event::ShellForegroundChanged {
                id,
                tab_id: "t".into(),
                command: None,
            },
            Event::ShellForegroundChanged {
                id,
                tab_id: "t".into(),
                command: Some(SavedCommand {
                    argv: vec!["python".into(), "script.py".into()],
                    cwd: "/repo/src".into(),
                }),
            },
            Event::ShellResurrectionChanged {
                id,
                tab_id: "t".into(),
                command: None,
            },
            Event::ShellResurrectionChanged {
                id,
                tab_id: "t".into(),
                command: Some(SavedCommand {
                    argv: vec!["cargo".into(), "test".into()],
                    cwd: "/repo".into(),
                }),
            },
            Event::Error {
                message: "oops".into(),
            },
        ];
        for evt in &events {
            round_trip(evt);
        }
    }

    #[test]
    fn saved_command_round_trip() {
        round_trip(&SavedCommand {
            argv: vec!["python".into(), "tests.py".into(), "--flag".into()],
            cwd: "/home/me/work".into(),
        });
        round_trip(&SavedCommand {
            argv: vec![],
            cwd: "".into(),
        });
    }
}
