use std::{collections::HashMap, path::PathBuf, time::Instant};

use protocol::{AttentionLevel, RepositoryId, Route, SshTarget, WorkspaceId};

use crate::workspace::{GitState, WorkspaceTerminals};

pub struct AppState {
    pub route: Route,
    pub workspaces: HashMap<WorkspaceId, Workspace>,
    pub ordered_ids: Vec<WorkspaceId>,
    pub repositories: HashMap<RepositoryId, Repository>,
    pub ordered_repo_ids: Vec<RepositoryId>,
    pub started_at: Instant,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            route: Route::Home,
            workspaces: HashMap::new(),
            ordered_ids: Vec::new(),
            repositories: HashMap::new(),
            ordered_repo_ids: Vec::new(),
            started_at: Instant::now(),
        }
    }
}

/// A registered base repository — a source-only git repo that worktree-backed
/// Workspaces are branched from. Never worked in directly.
pub struct Repository {
    pub id: RepositoryId,
    pub name: String,
    pub path: PathBuf,
    pub default_branch: Option<String>,
    /// Optional override for where this repo's worktrees are created. When
    /// `None`, the default `<repo_parent>/.conduit-worktrees/<repo>/<slug>`
    /// scheme is used.
    pub worktree_root: Option<PathBuf>,
    /// Default agent profile name to launch in new Workspaces (opaque to core;
    /// interpreted by the TUI when it starts the agent terminal).
    pub default_agent: Option<String>,
    pub ssh: Option<SshTarget>,
}

pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub path: PathBuf,
    pub ssh: Option<SshTarget>,
    pub git: GitState,
    pub attention: AttentionLevel,
    pub terminals: WorkspaceTerminals,
    pub last_activity: Instant,
    /// The Repository this Workspace's worktree belongs to (`None` for legacy
    /// bare workspaces that predate the registry).
    pub repository_id: Option<RepositoryId>,
    /// The branch checked out in this worktree.
    pub branch: Option<String>,
    /// The base branch this worktree was created from.
    pub base_branch: Option<String>,
    /// Agent chosen for this Workspace at creation: a configured profile name or
    /// a raw custom command (opaque to core; interpreted by the TUI when it
    /// starts the agent terminal). `None` = use the client's default agent.
    pub agent: Option<String>,
    /// Review state — orthogonal to `attention`. Set by the idle-while-dirty
    /// heuristic or a manual toggle.
    pub ready_for_review: bool,
    /// When true, `ready_for_review` was set manually and the heuristic won't
    /// override it until the agent produces new output.
    pub review_manual: bool,
    /// Whether the agent terminal has produced content inside the settle window.
    pub agent_active: bool,
    /// Whether the agent terminal has settled (gone quiet past the window).
    pub agent_idle: bool,
    /// Transient grace window: while `now < until`, terminal output is treated as
    /// local typing echo and ignored for the spinner / review / prompt heuristics.
    /// Not persisted (always `None` on load) — see `USER_TYPING_GRACE_MS`.
    pub agent_input_suppress_until: Option<Instant>,
}
