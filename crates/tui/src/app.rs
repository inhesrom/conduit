use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use protocol::{
    AttentionLevel, BranchInfo, GitState, RemoteBranchInfo, RepositoryId, RepositorySummary, Route,
    SavedCommand, TerminalKind, WorkspaceId, WorkspaceSummary,
};
use ratatui::text::Line;
use serde::{Deserialize, Serialize};

use crate::terminal_core::{TerminalCoreKind, WorkspaceTerminalState};
use crate::ui::widgets::tile_grid;

fn url_regex() -> &'static regex::Regex {
    use std::sync::OnceLock;
    static URL_RE: OnceLock<regex::Regex> = OnceLock::new();
    URL_RE.get_or_init(|| regex::Regex::new(r#"https?://[^\s<>"'\)\]\}]+"#).unwrap())
}

const SSH_HISTORY_MAX: usize = 20;

/// Tracks the state of the SSH workspace creation dialog.
pub struct SshWorkspaceInput {
    pub host: String,
    pub user: String,
    pub path: String,
    pub focused_field: SshField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshField {
    Host,
    User,
    Path,
}

impl SshWorkspaceInput {
    pub fn new() -> Self {
        Self {
            host: String::new(),
            user: String::new(),
            path: String::new(),
            focused_field: SshField::Host,
        }
    }

    pub fn cycle_field(&mut self) {
        self.focused_field = match self.focused_field {
            SshField::Host => SshField::User,
            SshField::User => SshField::Path,
            SshField::Path => SshField::Host,
        };
    }

    pub fn active_input_mut(&mut self) -> &mut String {
        match self.focused_field {
            SshField::Host => &mut self.host,
            SshField::User => &mut self.user,
            SshField::Path => &mut self.path,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshHistoryEntry {
    pub host: String,
    pub user: Option<String>,
    pub path: String,
}

pub struct SshHistoryPicker {
    pub selected: usize,
}

/// Tracks a single-line editable text buffer with a UTF-8 byte cursor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditableText {
    /// The current input text.
    pub text: String,
    /// Cursor position as a byte offset into `text`.
    pub cursor: usize,
}

impl EditableText {
    /// Inserts one character at the cursor and moves past it.
    pub fn insert_char(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    /// Deletes the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.previous_cursor_boundary();
        self.text.drain(prev..self.cursor);
        self.cursor = prev;
    }

    /// Deletes the character under the cursor.
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_cursor_boundary();
        self.text.drain(self.cursor..next);
    }

    /// Moves the cursor one character left.
    pub fn move_left(&mut self) {
        self.cursor = self.previous_cursor_boundary();
    }

    /// Moves the cursor one character right.
    pub fn move_right(&mut self) {
        self.cursor = self.next_cursor_boundary();
    }

    /// Moves the cursor to the beginning of the input.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the cursor to the end of the input.
    pub fn move_end(&mut self) {
        self.cursor = self.text.len();
    }

    /// Returns the cursor position as a character offset for rendering.
    pub fn cursor_char_index(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }

    fn previous_cursor_boundary(&self) -> usize {
        self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0)
    }

    fn next_cursor_boundary(&self) -> usize {
        if self.cursor >= self.text.len() {
            return self.text.len();
        }
        self.text[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(idx, _)| self.cursor + idx)
            .unwrap_or(self.text.len())
    }
}

/// Tracks the command popup input, captured output, and completion state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WorkspaceCommandState {
    /// Editable command text.
    pub input: EditableText,
    /// Captured command output shown in the popup.
    pub output: String,
    /// Whether a command is currently running.
    pub running: bool,
    /// Whether the current output belongs to a completed command.
    pub completed: bool,
    /// Process exit code, or `None` if the process was terminated by signal or failed to spawn.
    pub exit_code: Option<i32>,
    /// Vertical scroll offset for the output pane.
    pub output_scroll: u16,
}

impl WorkspaceCommandState {
    /// Creates an empty command popup state.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Identifies command popup state by workspace and command working directory.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WorkspaceCommandKey {
    pub id: WorkspaceId,
    pub cwd: String,
}

/// Tracks the state of the interactive directory browser shown when adding a workspace.
pub struct DirBrowserState {
    /// The filesystem path currently shown in the browser.
    pub path_input: String,
    /// Sorted list of subdirectory names at `path_input`.
    pub entries: Vec<String>,
    /// Index of the currently highlighted entry.
    pub selected: usize,
    /// Whether hidden (dot-prefixed) directories are shown.
    pub show_hidden: bool,
    /// Whether the user is currently typing in the path input field.
    pub editing_path: bool,
}

impl DirBrowserState {
    /// Creates a new browser rooted at `initial_path` and immediately populates entries.
    pub fn new(initial_path: String) -> Self {
        let mut state = Self {
            path_input: initial_path,
            entries: Vec::new(),
            selected: 0,
            show_hidden: false,
            editing_path: false,
        };
        state.refresh_entries();
        state
    }

    /// Re-reads `path_input` from disk and repopulates entries, clamping selection.
    pub fn refresh_entries(&mut self) {
        self.entries.clear();
        let path = Path::new(&self.path_input);
        if let Ok(rd) = fs::read_dir(path) {
            for entry in rd.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                if !ft.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !self.show_hidden && name.starts_with('.') {
                    continue;
                }
                self.entries.push(name);
            }
        }
        self.entries.sort();
        if self.entries.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.entries.len() - 1);
        }
    }

    /// Moves the selection by `delta` rows, clamped to valid bounds.
    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            self.selected = 0;
            return;
        }
        let len = self.entries.len() as isize;
        self.selected = (self.selected as isize + delta).clamp(0, len - 1) as usize;
    }

    /// Drills into the highlighted directory, canonicalizing the path.
    pub fn enter_selected(&mut self) {
        if let Some(name) = self.entries.get(self.selected).cloned() {
            let mut new_path = PathBuf::from(&self.path_input);
            new_path.push(&name);
            if let Ok(canonical) = new_path.canonicalize() {
                self.path_input = canonical.display().to_string();
            } else {
                self.path_input = new_path.display().to_string();
            }
            self.selected = 0;
            self.refresh_entries();
        }
    }

    /// Navigates to the parent directory.
    pub fn go_up(&mut self) {
        let path = PathBuf::from(&self.path_input);
        if let Some(parent) = path.parent() {
            self.path_input = parent.display().to_string();
            self.selected = 0;
            self.refresh_entries();
        }
    }

    /// Flips hidden-file visibility and refreshes the listing.
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh_entries();
    }

    /// Returns the full path of the currently highlighted child directory.
    pub fn selected_child_path(&self) -> Option<String> {
        let name = self.entries.get(self.selected)?;
        let mut p = PathBuf::from(&self.path_input);
        p.push(name);
        Some(p.display().to_string())
    }

    /// Confirms the typed path and returns to list navigation mode.
    pub fn confirm_path_edit(&mut self) {
        self.editing_path = false;
        self.selected = 0;
        self.refresh_entries();
    }

    /// Enters path editing mode.
    pub fn begin_path_edit(&mut self) {
        self.editing_path = true;
    }
}

/// Tracks an in-progress or completed mouse drag selection on the terminal screen.
///
/// Coordinates are in terminal cell units (column, row) relative to the top-left of
/// the full terminal window. `anchor` is where the button was pressed; `end` follows
/// the cursor as it moves.
#[derive(Debug, Clone, Copy)]
pub struct MouseSelection {
    /// Column where the drag began.
    pub anchor_col: u16,
    /// Row where the drag began.
    pub anchor_row: u16,
    /// Current column of the drag endpoint.
    pub end_col: u16,
    /// Current row of the drag endpoint.
    pub end_row: u16,
    /// Optional bounding rect that confines the selection to a single pane.
    pub confine: Option<ratatui::layout::Rect>,
}

impl MouseSelection {
    /// Creates a zero-length selection anchored at the given position, confined to `rect`.
    ///
    /// The anchor is clamped into `rect` so a press on a pane border (just outside an
    /// inset confine band) still anchors inside the band, keeping it aligned with the
    /// endpoint that `clamp_to_confine` also holds inside `rect`.
    pub fn at_confined(col: u16, row: u16, rect: ratatui::layout::Rect) -> Self {
        let max_col = rect.right().saturating_sub(1).max(rect.x);
        let max_row = rect.bottom().saturating_sub(1).max(rect.y);
        let col = col.clamp(rect.x, max_col);
        let row = row.clamp(rect.y, max_row);
        Self {
            anchor_col: col,
            anchor_row: row,
            end_col: col,
            end_row: row,
            confine: Some(rect),
        }
    }

    /// Clamp `end_col`/`end_row` to the confine rect (if set).
    pub fn clamp_to_confine(&mut self) {
        if let Some(r) = self.confine {
            let max_col = r.right().saturating_sub(1).max(r.x);
            let max_row = r.bottom().saturating_sub(1).max(r.y);
            self.end_col = self.end_col.clamp(r.x, max_col);
            self.end_row = self.end_row.clamp(r.y, max_row);
        }
    }

    /// Returns ((start_col, start_row), (end_col, end_row)) ordered by position.
    pub fn ordered(&self) -> ((u16, u16), (u16, u16)) {
        if (self.anchor_row, self.anchor_col) <= (self.end_row, self.end_col) {
            (
                (self.anchor_col, self.anchor_row),
                (self.end_col, self.end_row),
            )
        } else {
            (
                (self.end_col, self.end_row),
                (self.anchor_col, self.anchor_row),
            )
        }
    }

    /// Returns `true` when the anchor and end positions are identical (zero-area selection).
    pub fn is_empty(&self) -> bool {
        self.anchor_col == self.end_col && self.anchor_row == self.end_row
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// The persistent Repository → Workspace tree on the left.
    Sidebar,
    WsLog,
    WsBranches,
    WsDiff,
    WsTerminal,
    WsTerminalTabs,
    /// Review sub-mode panes within the detail.
    ReviewFiles,
    ReviewDiff,
}

/// A flattened row in the sidebar tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarRow {
    Repo(RepositoryId),
    Workspace(WorkspaceId),
}

/// How the left sidebar is displayed. `Ctrl+B` cycles through these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    /// Full Repository → Workspace tree (default).
    Expanded,
    /// Narrow vertical rail: each repo's name stacked one character per row.
    /// Pressing Enter on a repo opens a pop-out listing its workspaces.
    Rail,
    /// Hidden entirely; the detail pane fills the screen.
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickCreateField {
    Name,
    BaseBranch,
    Agent,
    Prompt,
}

/// State of the ctrl+n quick-create-workspace modal.
#[derive(Debug, Clone)]
pub struct QuickCreateState {
    pub repo_id: RepositoryId,
    pub repo_name: String,
    pub name: String,
    /// When false, only the name field is shown; Tab reveals the overrides.
    pub expanded: bool,
    /// Empty = use the repository's default branch.
    pub base_branch: String,
    /// Agent to launch: a configured profile name or a raw custom command.
    /// Pre-filled with the repo/global default. While [`Self::agent_command_edit`]
    /// is false this holds a profile name cycled with ←/→; once true it holds a
    /// free-text launch command.
    pub agent: String,
    /// When true, the agent field is being edited as a raw launch command (the
    /// selected agent was expanded with Enter). When false it's a cyclable
    /// profile selection (`◂ name ▸`).
    pub agent_command_edit: bool,
    pub initial_prompt: String,
    pub field: QuickCreateField,
}

impl QuickCreateState {
    pub fn active_input_mut(&mut self) -> &mut String {
        match self.field {
            QuickCreateField::Name => &mut self.name,
            QuickCreateField::BaseBranch => &mut self.base_branch,
            QuickCreateField::Agent => &mut self.agent,
            QuickCreateField::Prompt => &mut self.initial_prompt,
        }
    }

    /// Advance focus, only visiting the override fields once expanded.
    pub fn next_field(&mut self) {
        self.field = match self.field {
            QuickCreateField::Name if self.expanded => QuickCreateField::BaseBranch,
            QuickCreateField::Name => QuickCreateField::Name,
            QuickCreateField::BaseBranch => QuickCreateField::Agent,
            QuickCreateField::Agent => QuickCreateField::Prompt,
            QuickCreateField::Prompt => QuickCreateField::Name,
        };
    }

    /// Replace the agent field with the next/prev configured agent name,
    /// wrapping around. Used by ←/→ while the Agent field is focused. If the
    /// current value isn't a known agent (a custom command), starts at the end
    /// (so → lands on the first agent, ← on the last).
    pub fn cycle_agent(&mut self, agents: &[AgentProfile], delta: i32) {
        if agents.is_empty() {
            return;
        }
        let len = agents.len() as i32;
        let cur = agents.iter().position(|a| a.name == self.agent);
        let next = match cur {
            Some(i) => (i as i32 + delta).rem_euclid(len),
            None => {
                if delta >= 0 {
                    0
                } else {
                    len - 1
                }
            }
        };
        self.agent = agents[next as usize].name.clone();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogItem {
    UncommittedHeader,
    ChangedFile(usize),
    ChangedDirectory(String),
    Commit(usize),
    CommitFile(usize, usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UncommittedRow {
    File {
        file_index: usize,
        path: String,
        name: String,
        depth: usize,
        index_status: char,
        worktree_status: char,
    },
    Directory {
        path: String,
        name: String,
        depth: usize,
        collapsed: bool,
        index_status: char,
        worktree_status: char,
    },
}

const AGENT_STARTUP_FAILURE_WINDOW: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
struct AgentStartup {
    started_at: Instant,
    is_fallback: bool,
    prompt_sent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentExitAction {
    Fallback { prompt: Option<String> },
    RespawnShell,
    None,
}

impl UncommittedRow {
    pub fn status(&self) -> (char, char) {
        match self {
            UncommittedRow::File {
                index_status,
                worktree_status,
                ..
            }
            | UncommittedRow::Directory {
                index_status,
                worktree_status,
                ..
            } => (*index_status, *worktree_status),
        }
    }
}

fn build_uncommitted_rows(
    git: &GitState,
    collapsed_dirs: Option<&HashSet<String>>,
) -> Vec<UncommittedRow> {
    let mut rows = Vec::new();
    let mut emitted_dirs = HashSet::new();
    let mut descendant_statuses: HashMap<String, Vec<(char, char)>> = HashMap::new();

    for f in &git.changed {
        let parts: Vec<&str> = f.path.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() <= 1 {
            continue;
        }

        let mut prefix = String::new();
        for part in parts.iter().take(parts.len() - 1) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            descendant_statuses
                .entry(prefix.clone())
                .or_default()
                .push((f.index_status, f.worktree_status));
        }
    }

    for (file_index, f) in git.changed.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').filter(|part| !part.is_empty()).collect();
        if parts.len() <= 1 {
            rows.push(UncommittedRow::File {
                file_index,
                path: f.path.clone(),
                name: f.path.clone(),
                depth: 0,
                index_status: f.index_status,
                worktree_status: f.worktree_status,
            });
            continue;
        }

        let mut prefix = String::new();
        let mut hidden_by_collapsed_parent = false;
        for (depth, part) in parts.iter().take(parts.len() - 1).enumerate() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);

            if hidden_by_collapsed_parent {
                continue;
            }

            let collapsed = collapsed_dirs
                .map(|dirs| dirs.contains(&prefix))
                .unwrap_or(false);
            if emitted_dirs.insert(prefix.clone()) {
                let (index_status, worktree_status) =
                    aggregate_directory_status(descendant_statuses.get(&prefix));
                rows.push(UncommittedRow::Directory {
                    path: prefix.clone(),
                    name: (*part).to_string(),
                    depth,
                    collapsed,
                    index_status,
                    worktree_status,
                });
            }
            if collapsed {
                hidden_by_collapsed_parent = true;
            }
        }

        if !hidden_by_collapsed_parent {
            rows.push(UncommittedRow::File {
                file_index,
                path: f.path.clone(),
                name: parts.last().copied().unwrap_or(&f.path).to_string(),
                depth: parts.len() - 1,
                index_status: f.index_status,
                worktree_status: f.worktree_status,
            });
        }
    }

    rows
}

fn aggregate_directory_status(statuses: Option<&Vec<(char, char)>>) -> (char, char) {
    let Some(statuses) = statuses else {
        return (' ', ' ');
    };
    if statuses.is_empty() {
        return (' ', ' ');
    }

    (
        aggregate_index_status(statuses),
        aggregate_worktree_status(statuses),
    )
}

fn aggregate_index_status(statuses: &[(char, char)]) -> char {
    if statuses
        .iter()
        .all(|(index_status, worktree_status)| *index_status == '?' && *worktree_status == '?')
    {
        return '?';
    }

    let mut staged = statuses
        .iter()
        .map(|(index_status, _)| *index_status)
        .filter(|status| *status != ' ' && *status != '?');
    let Some(first) = staged.next() else {
        return ' ';
    };
    if staged.all(|status| status == first)
        && statuses
            .iter()
            .all(|(index_status, _)| *index_status == first)
    {
        first
    } else {
        '*'
    }
}

fn aggregate_worktree_status(statuses: &[(char, char)]) -> char {
    if statuses
        .iter()
        .all(|(index_status, worktree_status)| *index_status == '?' && *worktree_status == '?')
    {
        return '?';
    }

    let mut changed = statuses
        .iter()
        .map(|(_, worktree_status)| *worktree_status)
        .filter(|status| *status != ' ');
    let Some(first) = changed.next() else {
        return ' ';
    };
    if changed.all(|status| status == first)
        && statuses
            .iter()
            .all(|(_, worktree_status)| *worktree_status == first)
    {
        first
    } else {
        '*'
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSubPane {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteBranchTarget {
    Local {
        branch: String,
    },
    Remote {
        remote: String,
        branch: String,
        full_name: String,
    },
}

pub struct TuiApp {
    pub route: Route,
    pub focus: Focus,
    pub workspaces: Vec<WorkspaceSummary>,
    /// Registered base repositories (top tier of the sidebar tree).
    pub repositories: Vec<RepositorySummary>,
    /// Selected row index into the flattened sidebar tree.
    pub sidebar_selected: usize,
    /// How the sidebar is displayed (Expanded tree / vertical Rail / Hidden).
    /// `Ctrl+B` cycles through the three.
    pub sidebar_mode: SidebarMode,
    /// In Rail mode, the index (into `repositories`) of the highlighted repo.
    pub rail_selected: usize,
    /// In Rail mode, the repo whose workspace pop-out is open (None = closed).
    pub sidebar_popout: Option<RepositoryId>,
    /// Selected workspace index within the open pop-out.
    pub popout_selected: usize,
    /// Repositories whose workspace children are collapsed in the sidebar.
    pub collapsed_repos: HashSet<RepositoryId>,
    /// When true, the sidebar shows only ready-for-review workspaces.
    pub sidebar_review_filter: bool,
    /// Active ctrl+n quick-create modal, if any.
    pub quick_create: Option<QuickCreateState>,
    /// Initial prompt to deliver to the agent of the next created workspace.
    pub pending_create_prompt: Option<String>,
    /// (workspace, prompt) waiting for the agent terminal to produce output.
    pub pending_agent_prompt: Option<(WorkspaceId, String)>,
    /// A newly created workspace whose terminals the main loop should start.
    pub pending_open_created: Option<WorkspaceId>,
    /// (workspace, prompt) the main loop should send to the agent now.
    pub pending_prompt_send: Option<(WorkspaceId, String)>,
    /// Whole-branch review: changed file paths per workspace.
    pub review_files: HashMap<WorkspaceId, Vec<String>>,
    pub review_file_selected: usize,
    pub review_diff_scroll: u16,
    /// (workspace, file) whose branch diff the main loop should request now.
    pub pending_review_diff: Option<(WorkspaceId, String)>,
    pub workspace_git: HashMap<WorkspaceId, GitState>,
    pub workspace_diff: HashMap<WorkspaceId, (String, String)>,
    pub terminal_state: HashMap<WorkspaceId, WorkspaceTerminalState>,
    pub last_resize_sent: HashMap<(WorkspaceId, String), (u16, u16)>,
    pub workspace_tabs: HashMap<WorkspaceId, WorkspaceTabsState>,
    pub saved_tabs_by_path: HashMap<String, PersistedWorkspaceTabs>,
    pub ws_tabs: Vec<TerminalTab>,
    pub ws_active_tab: usize,
    pub ws_next_shell_tab: u32,
    pub home_selected: usize,
    pub ws_uncommitted_expanded: bool,
    pub collapsed_uncommitted_dirs: HashMap<WorkspaceId, HashSet<String>>,
    pub ws_selected_commit: usize,
    pub ws_selected_local_branch: usize,
    pub ws_selected_remote_branch: usize,
    pub ws_branch_sub_pane: BranchSubPane,
    pub ws_pending_select_head_branch: bool,
    pub ws_diff_scroll: u16,
    pub spinner_tick: u8,
    pub dir_browser: Option<DirBrowserState>,
    pub pending_delete_workspace: Option<WorkspaceId>,
    pub pending_delete_repo: Option<RepositoryId>,
    pub rename_workspace_input: Option<String>,
    pub rename_tab_input: Option<String>,
    pub git_action_message: Option<(String, Instant)>,
    pub commit_input: Option<String>,
    pub create_branch_input: Option<String>,
    pub workspace_commands: HashMap<WorkspaceCommandKey, WorkspaceCommandState>,
    pub settings: Settings,
    /// When true, Conduit owns keyboard and mouse input for workspace commands.
    /// When false, the focused terminal receives normal input directly.
    pub terminal_command_mode: bool,
    /// Workspace IDs with an in-flight git network operation (pull/push/fetch).
    /// Stores the start time so we can enforce a minimum spinner display duration.
    pub git_op_in_progress: HashMap<WorkspaceId, Instant>,
    /// Deferred git result waiting for spinner minimum duration to elapse.
    pub deferred_git_result: Option<(WorkspaceId, String)>,
    pub settings_open: bool,
    pub settings_selected: usize,
    pub settings_edit_buffer: Option<String>,
    /// Multi-step new agent wizard: (step, staging profile, input buffer).
    /// Steps: 0 = name, 1 = command, 2 = yolo flags.
    pub new_agent_wizard: Option<(usize, AgentProfile, String)>,
    pub confirming_delete_agent: bool,
    pub mouse_selection: Option<MouseSelection>,
    /// Set on mouse-up to request clipboard copy on the next frame render.
    pub pending_copy_selection: Option<MouseSelection>,
    /// Agent fallback restart queued after a fast startup failure.
    pub pending_agent_fallback: Option<(protocol::WorkspaceId, Option<String>)>,
    /// When an agent tab exits, queue a shell respawn in the same tab.
    pub pending_agent_respawn: Option<(protocol::WorkspaceId, Option<String>)>,
    agent_startups: HashMap<WorkspaceId, AgentStartup>,
    pending_agent_startups: HashMap<WorkspaceId, bool>,
    suppressed_agent_exits: HashSet<WorkspaceId>,
    pub ssh_workspace_input: Option<SshWorkspaceInput>,
    pub ssh_history: Vec<SshHistoryEntry>,
    pub ssh_history_picker: Option<SshHistoryPicker>,
    pub confirm_discard_file: Option<String>,
    pub confirm_discard_all: Option<WorkspaceId>,
    pub stash_input: Option<String>,
    pub confirm_stash_pull_pop: Option<WorkspaceId>,
    pub confirm_delete_branch: Option<DeleteBranchTarget>,
    pub ws_expanded_commit: Option<usize>,
    pub commit_files_cache: HashMap<String, Vec<String>>,
    pub ws_tag_filter: bool,
    /// Whether the user is in move-workspace mode on the home screen.
    pub moving_workspace: bool,
    /// Indices of expanded tiles on the home screen.
    pub home_expanded_tiles: HashSet<usize>,
    /// Vertical scroll offset for the home tile list.
    pub home_scroll_offset: u16,
    /// Cached grid height from last render, used for scroll calculations.
    pub last_grid_height: u16,
    /// Cached terminal content area `(cols, rows)` from the last frame. Used to
    /// birth new PTYs at the right size so freshly-spawned TUIs (e.g. Claude)
    /// lay out at the correct width immediately instead of starting wide and
    /// relying on a follow-up resize.
    pub terminal_content_size: (u16, u16),
    /// Pending CPR (Cursor Position Report) responses to write back to PTY.
    pub pending_cpr_responses: Vec<(WorkspaceId, String, protocol::TerminalKind, Vec<u8>)>,
    /// Debug FPS display — computed once per second.
    pub debug_fps: u16,
    pub debug_fps_frame_count: u64,
    pub debug_fps_last_reset: Option<Instant>,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self {
            route: Route::Home,
            focus: Focus::Sidebar,
            workspaces: Vec::new(),
            repositories: Vec::new(),
            sidebar_selected: 0,
            sidebar_mode: SidebarMode::Expanded,
            rail_selected: 0,
            sidebar_popout: None,
            popout_selected: 0,
            collapsed_repos: HashSet::new(),
            sidebar_review_filter: false,
            quick_create: None,
            pending_create_prompt: None,
            pending_agent_prompt: None,
            pending_open_created: None,
            pending_prompt_send: None,
            review_files: HashMap::new(),
            review_file_selected: 0,
            review_diff_scroll: 0,
            pending_review_diff: None,
            workspace_git: HashMap::new(),
            workspace_diff: HashMap::new(),
            terminal_state: HashMap::new(),
            last_resize_sent: HashMap::new(),
            workspace_tabs: HashMap::new(),
            saved_tabs_by_path: load_saved_tabs_by_path(),
            ws_tabs: vec![
                TerminalTab::agent(),
                TerminalTab::shell("shell".to_string(), "shell".to_string()),
            ],
            ws_active_tab: 0,
            ws_next_shell_tab: 2,
            home_selected: 0,
            ws_uncommitted_expanded: false,
            collapsed_uncommitted_dirs: HashMap::new(),
            ws_selected_commit: 0,
            ws_selected_local_branch: 0,
            ws_selected_remote_branch: 0,
            ws_branch_sub_pane: BranchSubPane::Local,
            ws_pending_select_head_branch: false,
            ws_diff_scroll: 0,
            spinner_tick: 0,
            dir_browser: None,
            pending_delete_workspace: None,
            pending_delete_repo: None,
            rename_workspace_input: None,
            rename_tab_input: None,
            git_action_message: None,
            commit_input: None,
            create_branch_input: None,
            workspace_commands: HashMap::new(),
            git_op_in_progress: HashMap::new(),
            deferred_git_result: None,
            settings: load_settings(),
            terminal_command_mode: false,
            settings_open: false,
            settings_selected: 0,
            settings_edit_buffer: None,
            new_agent_wizard: None,
            confirming_delete_agent: false,
            mouse_selection: None,
            pending_copy_selection: None,
            pending_agent_fallback: None,
            pending_agent_respawn: None,
            agent_startups: HashMap::new(),
            pending_agent_startups: HashMap::new(),
            suppressed_agent_exits: HashSet::new(),
            ssh_workspace_input: None,
            ssh_history: load_ssh_history(),
            ssh_history_picker: None,
            confirm_discard_file: None,
            confirm_discard_all: None,
            stash_input: None,
            confirm_stash_pull_pop: None,
            confirm_delete_branch: None,
            ws_expanded_commit: None,
            commit_files_cache: HashMap::new(),
            ws_tag_filter: false,
            moving_workspace: false,
            home_expanded_tiles: HashSet::new(),
            home_scroll_offset: 0,
            last_grid_height: 0,
            terminal_content_size: (120, 24),
            pending_cpr_responses: Vec::new(),
            debug_fps: 0,
            debug_fps_frame_count: 0,
            debug_fps_last_reset: None,
        }
    }
}

impl TuiApp {
    pub fn set_workspaces(&mut self, workspaces: Vec<WorkspaceSummary>) {
        self.persist_tabs_for_active_workspace();
        self.workspaces = workspaces;
        if self.workspaces.is_empty() {
            self.home_selected = 0;
        } else if self.home_selected >= self.workspaces.len() {
            self.home_selected = self.workspaces.len() - 1;
        }
        let valid_command_workspaces: HashSet<WorkspaceId> =
            self.workspaces.iter().map(|ws| ws.id).collect();
        self.workspace_commands
            .retain(|key, _| valid_command_workspaces.contains(&key.id));
        self.agent_startups
            .retain(|id, _| valid_command_workspaces.contains(id));
        self.pending_agent_startups
            .retain(|id, _| valid_command_workspaces.contains(id));
        self.suppressed_agent_exits
            .retain(|id| valid_command_workspaces.contains(id));
        if matches!(
            &self.pending_agent_fallback,
            Some((id, _)) if !valid_command_workspaces.contains(id)
        ) {
            self.pending_agent_fallback = None;
        }
        if matches!(
            &self.pending_agent_respawn,
            Some((id, _)) if !valid_command_workspaces.contains(id)
        ) {
            self.pending_agent_respawn = None;
        }
        self.reconcile_workspace_tab_state();
        self.ensure_home_selected_visible();
        self.clamp_sidebar_selection();
    }

    pub fn set_repositories(&mut self, repositories: Vec<RepositorySummary>) {
        self.repositories = repositories;
        self.clamp_sidebar_selection();
    }

    /// Flattened sidebar rows: each repo header followed by its (optionally
    /// review-filtered) workspace children, then any orphan workspaces.
    pub fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let mut rows = Vec::new();
        for repo in &self.repositories {
            rows.push(SidebarRow::Repo(repo.id));
            if !self.collapsed_repos.contains(&repo.id) {
                for ws in &self.workspaces {
                    if ws.repository_id == Some(repo.id)
                        && (!self.sidebar_review_filter || ws.ready_for_review)
                    {
                        rows.push(SidebarRow::Workspace(ws.id));
                    }
                }
            }
        }
        for ws in &self.workspaces {
            let has_repo = ws
                .repository_id
                .map(|rid| self.repositories.iter().any(|r| r.id == rid))
                .unwrap_or(false);
            if !has_repo && (!self.sidebar_review_filter || ws.ready_for_review) {
                rows.push(SidebarRow::Workspace(ws.id));
            }
        }
        rows
    }

    fn clamp_sidebar_selection(&mut self) {
        let len = self.sidebar_rows().len();
        if len == 0 {
            self.sidebar_selected = 0;
        } else if self.sidebar_selected >= len {
            self.sidebar_selected = len - 1;
        }
    }

    pub fn move_sidebar_selection(&mut self, delta: isize) {
        let len = self.sidebar_rows().len();
        if len == 0 {
            self.sidebar_selected = 0;
            return;
        }
        let n = (self.sidebar_selected as isize + delta).clamp(0, (len - 1) as isize);
        self.sidebar_selected = n as usize;
    }

    pub fn selected_sidebar_row(&self) -> Option<SidebarRow> {
        self.sidebar_rows().get(self.sidebar_selected).copied()
    }

    pub fn selected_sidebar_workspace_id(&self) -> Option<WorkspaceId> {
        match self.selected_sidebar_row()? {
            SidebarRow::Workspace(id) => Some(id),
            SidebarRow::Repo(_) => None,
        }
    }

    pub fn begin_quick_create(&mut self, repo_id: RepositoryId) {
        let repo = self.repositories.iter().find(|r| r.id == repo_id);
        let repo_name = repo.map(|r| r.name.clone()).unwrap_or_default();
        // Default to the repo's configured agent if set, else the global default.
        let agent = repo
            .and_then(|r| r.default_agent.clone())
            .filter(|a| !a.trim().is_empty())
            .unwrap_or_else(|| self.settings.default_agent.clone());
        self.quick_create = Some(QuickCreateState {
            repo_id,
            repo_name,
            name: String::new(),
            expanded: false,
            base_branch: String::new(),
            agent,
            agent_command_edit: false,
            initial_prompt: String::new(),
            field: QuickCreateField::Name,
        });
    }

    pub fn is_quick_creating(&self) -> bool {
        self.quick_create.is_some()
    }

    pub fn cancel_quick_create(&mut self) {
        self.quick_create = None;
    }

    pub fn enter_review_mode(&mut self) {
        self.focus = Focus::ReviewFiles;
        self.review_file_selected = 0;
        self.review_diff_scroll = 0;
    }

    pub fn exit_review_mode(&mut self) {
        self.focus = Focus::WsTerminal;
    }

    /// The repo "in context" for the current sidebar selection: the repo of a
    /// selected workspace, or a selected repo header itself.
    pub fn sidebar_context_repo(&self) -> Option<RepositoryId> {
        match self.selected_sidebar_row()? {
            SidebarRow::Repo(id) => Some(id),
            SidebarRow::Workspace(wid) => self
                .workspaces
                .iter()
                .find(|w| w.id == wid)
                .and_then(|w| w.repository_id),
        }
    }

    pub fn toggle_collapse_selected(&mut self) {
        if let Some(SidebarRow::Repo(id)) = self.selected_sidebar_row() {
            if self.collapsed_repos.contains(&id) {
                self.collapsed_repos.remove(&id);
            } else {
                self.collapsed_repos.insert(id);
            }
            self.clamp_sidebar_selection();
        }
    }

    /// True while the sidebar occupies columns on screen (Expanded or Rail).
    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_mode != SidebarMode::Hidden
    }

    /// Advance the sidebar display: Expanded → Rail → Hidden → Expanded.
    pub fn cycle_sidebar_mode(&mut self) {
        self.sidebar_mode = match self.sidebar_mode {
            SidebarMode::Expanded => {
                // Sync the rail's repo selection to whatever the tree had in context.
                if let Some(rid) = self.sidebar_context_repo() {
                    if let Some(i) = self.repositories.iter().position(|r| r.id == rid) {
                        self.rail_selected = i;
                    }
                }
                SidebarMode::Rail
            }
            SidebarMode::Rail => {
                self.sidebar_popout = None;
                SidebarMode::Hidden
            }
            SidebarMode::Hidden => SidebarMode::Expanded,
        };
    }

    /// Clamped index of the highlighted repo in Rail mode.
    pub fn rail_selected_repo_index(&self) -> usize {
        self.rail_selected
            .min(self.repositories.len().saturating_sub(1))
    }

    pub fn move_rail_selection(&mut self, delta: isize) {
        let len = self.repositories.len();
        if len == 0 {
            self.rail_selected = 0;
            return;
        }
        let n = (self.rail_selected as isize + delta).clamp(0, (len - 1) as isize);
        self.rail_selected = n as usize;
    }

    /// Repository currently highlighted in the rail, if any.
    pub fn selected_rail_repo(&self) -> Option<RepositoryId> {
        self.repositories
            .get(self.rail_selected_repo_index())
            .map(|r| r.id)
    }

    /// Open the workspace pop-out for the currently highlighted rail repo.
    pub fn open_sidebar_popout(&mut self) {
        if let Some(id) = self.selected_rail_repo() {
            self.sidebar_popout = Some(id);
            self.popout_selected = 0;
        }
    }

    pub fn close_sidebar_popout(&mut self) {
        self.sidebar_popout = None;
    }

    pub fn toggle_sidebar_popout(&mut self) {
        if self.sidebar_popout.is_some() {
            self.close_sidebar_popout();
        } else {
            self.open_sidebar_popout();
        }
    }

    /// Workspace ids belonging to the repo whose pop-out is open (all of them,
    /// ignoring the review filter — the pop-out shows every workspace).
    pub fn popout_workspaces(&self) -> Vec<WorkspaceId> {
        let Some(repo) = self.sidebar_popout else {
            return Vec::new();
        };
        self.workspaces
            .iter()
            .filter(|w| w.repository_id == Some(repo))
            .map(|w| w.id)
            .collect()
    }

    pub fn move_popout_selection(&mut self, delta: isize) {
        let len = self.popout_workspaces().len();
        if len == 0 {
            self.popout_selected = 0;
            return;
        }
        let n = (self.popout_selected as isize + delta).clamp(0, (len - 1) as isize);
        self.popout_selected = n as usize;
    }

    pub fn selected_popout_workspace_id(&self) -> Option<WorkspaceId> {
        self.popout_workspaces().get(self.popout_selected).copied()
    }

    pub fn selected_workspace_id(&self) -> Option<WorkspaceId> {
        self.workspaces.get(self.home_selected).map(|w| w.id)
    }

    pub fn active_workspace_id(&self) -> Option<WorkspaceId> {
        match self.route {
            Route::Workspace { id } => Some(id),
            Route::Home => None,
        }
    }

    /// The agent chosen for a Workspace at creation (profile name or custom
    /// command), if any. Used to launch the right agent in its terminal.
    pub fn workspace_agent(&self, id: WorkspaceId) -> Option<String> {
        self.workspaces
            .iter()
            .find(|w| w.id == id)
            .and_then(|w| w.agent.clone())
    }

    pub fn workspace_agent_running(&self, id: WorkspaceId) -> bool {
        self.workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.agent_running)
            .unwrap_or(false)
    }

    pub fn queue_agent_startup(&mut self, id: WorkspaceId, is_fallback: bool) {
        self.pending_agent_startups.insert(id, is_fallback);
    }

    pub fn record_agent_started(&mut self, id: WorkspaceId) {
        if let Some(is_fallback) = self.pending_agent_startups.remove(&id) {
            self.record_agent_startup_at(id, is_fallback, Instant::now());
        }
    }

    fn record_agent_startup_at(&mut self, id: WorkspaceId, is_fallback: bool, started_at: Instant) {
        self.agent_startups.insert(
            id,
            AgentStartup {
                started_at,
                is_fallback,
                prompt_sent: None,
            },
        );
    }

    pub fn record_agent_prompt_sent(&mut self, id: WorkspaceId, prompt: &str) {
        if let Some(startup) = self.agent_startups.get_mut(&id) {
            startup.prompt_sent = Some(prompt.to_string());
        }
    }

    pub fn suppress_next_agent_exit(&mut self, id: WorkspaceId) {
        self.suppressed_agent_exits.insert(id);
        self.agent_startups.remove(&id);
    }

    pub fn handle_agent_exit(
        &mut self,
        id: WorkspaceId,
        exit_code: Option<i32>,
    ) -> AgentExitAction {
        self.handle_agent_exit_at(id, exit_code, Instant::now())
    }

    fn handle_agent_exit_at(
        &mut self,
        id: WorkspaceId,
        exit_code: Option<i32>,
        now: Instant,
    ) -> AgentExitAction {
        if self.suppressed_agent_exits.remove(&id) {
            self.agent_startups.remove(&id);
            let _ = self.take_queued_agent_prompt(id);
            return AgentExitAction::None;
        }

        self.pending_agent_startups.remove(&id);
        let queued_prompt = self.take_queued_agent_prompt(id);
        let Some(startup) = self.agent_startups.remove(&id) else {
            return AgentExitAction::RespawnShell;
        };

        let failed_exit = exit_code.map(|code| code != 0).unwrap_or(true);
        let fast_exit =
            now.saturating_duration_since(startup.started_at) <= AGENT_STARTUP_FAILURE_WINDOW;
        if failed_exit && fast_exit && !startup.is_fallback {
            return AgentExitAction::Fallback {
                prompt: queued_prompt.or(startup.prompt_sent),
            };
        }

        AgentExitAction::RespawnShell
    }

    fn take_queued_agent_prompt(&mut self, id: WorkspaceId) -> Option<String> {
        take_matching_prompt(&mut self.pending_prompt_send, id)
            .or_else(|| take_matching_prompt(&mut self.pending_agent_prompt, id))
    }

    pub fn open_workspace(&mut self, id: WorkspaceId) {
        self.persist_tabs_for_active_workspace();
        self.route = Route::Workspace { id };
        self.focus = Focus::WsTerminal;
        self.load_tabs_for_workspace(id);
    }

    pub fn go_home(&mut self) {
        self.persist_tabs_for_active_workspace();
        self.route = Route::Home;
        self.focus = Focus::Sidebar;
    }

    pub fn terminal_fullscreen(&self) -> bool {
        self.active_tab().fullscreen
    }

    pub fn toggle_terminal_fullscreen(&mut self) {
        let idx = self.ws_active_tab.min(self.ws_tabs.len().saturating_sub(1));
        self.ws_tabs[idx].fullscreen = !self.ws_tabs[idx].fullscreen;
    }

    pub fn end_move_workspace(&mut self) {
        self.moving_workspace = false;
    }

    /// Swap the selected workspace in the given direction and follow it.
    /// Returns the id and delta for sending a MoveWorkspace command.
    pub fn swap_workspace(&mut self, delta: isize) -> Option<(protocol::WorkspaceId, i32)> {
        let len = self.workspaces.len();
        if len < 2 {
            return None;
        }
        let cur = self.home_selected;
        let new_idx = (cur as isize + delta).clamp(0, (len - 1) as isize) as usize;
        if cur == new_idx {
            return None;
        }
        let id = self.workspaces[cur].id;
        self.workspaces.swap(cur, new_idx);
        // Move expanded-tile tracking to match
        let cur_expanded = self.home_expanded_tiles.remove(&cur);
        let new_expanded = self.home_expanded_tiles.remove(&new_idx);
        if cur_expanded {
            self.home_expanded_tiles.insert(new_idx);
        }
        if new_expanded {
            self.home_expanded_tiles.insert(cur);
        }
        self.home_selected = new_idx;
        self.ensure_home_selected_visible();
        Some((id, delta as i32))
    }

    /// Adjusts `home_scroll_offset` so the selected tile is visible.
    pub fn ensure_home_selected_visible(&mut self) {
        if self.last_grid_height == 0 {
            return;
        }
        let expanded_h = tile_grid::tile_h_expanded(self.settings.preview_lines);
        let y = tile_grid::tile_y_offset(self.home_selected, &self.home_expanded_tiles, expanded_h);
        let tile_h = if self.home_expanded_tiles.contains(&self.home_selected) {
            expanded_h
        } else {
            tile_grid::TILE_H
        };
        // Scroll up if tile is above viewport
        if y < self.home_scroll_offset {
            self.home_scroll_offset = y;
        }
        // Scroll down if tile bottom is below viewport
        if y + tile_h > self.home_scroll_offset + self.last_grid_height {
            self.home_scroll_offset = (y + tile_h).saturating_sub(self.last_grid_height);
        }
    }

    pub fn active_tab(&self) -> &TerminalTab {
        &self.ws_tabs[self.ws_active_tab.min(self.ws_tabs.len().saturating_sub(1))]
    }

    pub fn active_tab_id(&self) -> String {
        self.active_tab().id.clone()
    }

    pub fn active_tab_kind(&self) -> TerminalKind {
        self.active_tab().kind
    }

    pub fn apply_foreground_change(
        &mut self,
        _id: WorkspaceId,
        _tab_id: String,
        _command: Option<SavedCommand>,
    ) {
        // Live foreground changes are informational; resurrection overlays are
        // driven only by ShellResurrectionChanged after daemon startup.
    }

    pub fn apply_shell_resurrection_change(
        &mut self,
        id: WorkspaceId,
        tab_id: String,
        command: Option<SavedCommand>,
    ) {
        let has_command = command.is_some();
        if has_command && !self.workspace_tabs.contains_key(&id) {
            let state = self
                .workspace_path(id)
                .and_then(|p| self.saved_tabs_by_path.get(&p).cloned())
                .map(|saved| WorkspaceTabsState::from_saved(&saved))
                .unwrap_or_else(WorkspaceTabsState::default_state);
            self.workspace_tabs
                .insert(id, sanitize_workspace_tabs(state));
        }
        if let Some(state) = self.workspace_tabs.get_mut(&id) {
            if has_command && state.tabs.iter().all(|t| t.id != tab_id) {
                state
                    .tabs
                    .push(TerminalTab::shell(tab_id.clone(), tab_id.clone()));
            }
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.last_command = command.clone();
                tab.overlay_dismissed = false;
            }
        }
        if Some(id) == self.active_workspace_id() {
            if has_command && self.ws_tabs.iter().all(|t| t.id != tab_id) {
                self.ws_tabs
                    .push(TerminalTab::shell(tab_id.clone(), tab_id.clone()));
            }
            if let Some(tab) = self.ws_tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.last_command = command;
                tab.overlay_dismissed = false;
            }
        }
    }

    pub fn pending_resurrect_command(&self) -> Option<&SavedCommand> {
        let tab = self.ws_tabs.get(self.ws_active_tab)?;
        if tab.kind != TerminalKind::Shell {
            return None;
        }
        if tab.overlay_dismissed {
            return None;
        }
        tab.last_command.as_ref()
    }

    pub fn dismiss_resurrect_overlay(&mut self) {
        let Some(id) = self.active_workspace_id() else {
            return;
        };
        let idx = self.ws_active_tab;
        let Some(tab_id) = self.ws_tabs.get(idx).map(|t| t.id.clone()) else {
            return;
        };
        if let Some(tab) = self.ws_tabs.get_mut(idx) {
            tab.overlay_dismissed = true;
            tab.last_command = None;
        }
        if let Some(state) = self.workspace_tabs.get_mut(&id) {
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.last_command = None;
                tab.overlay_dismissed = true;
            }
        }
    }

    pub fn take_resurrect_command(&mut self) -> Option<SavedCommand> {
        let id = self.active_workspace_id()?;
        let idx = self.ws_active_tab;
        let tab_id = self.ws_tabs.get(idx)?.id.clone();
        let cmd = self.ws_tabs.get_mut(idx).and_then(|t| {
            t.overlay_dismissed = true;
            t.last_command.take()
        })?;
        if let Some(state) = self.workspace_tabs.get_mut(&id) {
            if let Some(tab) = state.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.last_command = None;
                tab.overlay_dismissed = true;
            }
        }
        Some(cmd)
    }

    pub fn terminal_command_mode(&self) -> bool {
        self.terminal_command_mode
    }

    pub fn toggle_terminal_command_mode(&mut self) {
        self.terminal_command_mode = !self.terminal_command_mode;
    }

    pub fn move_terminal_tab(&mut self, delta: isize) {
        if self.ws_tabs.is_empty() {
            return;
        }
        let len = self.ws_tabs.len() as isize;
        let next = (self.ws_active_tab as isize + delta).clamp(0, len - 1);
        self.ws_active_tab = next as usize;
        self.persist_tabs_for_active_workspace();
    }

    pub fn set_active_tab_index(&mut self, index: usize) {
        if self.ws_tabs.is_empty() {
            self.ws_active_tab = 0;
        } else {
            self.ws_active_tab = index.min(self.ws_tabs.len() - 1);
        }
        self.persist_tabs_for_active_workspace();
    }

    pub fn add_shell_tab(&mut self) {
        let n = self.ws_next_shell_tab;
        self.ws_next_shell_tab = self.ws_next_shell_tab.saturating_add(1);
        let id = format!("shell-{n}");
        let label = format!("shell-{n}");
        self.ws_tabs.push(TerminalTab::shell(id, label));
        self.ws_active_tab = self.ws_tabs.len() - 1;
        self.persist_tabs_for_active_workspace();
    }

    pub fn can_close_active_tab(&self) -> bool {
        self.ws_tabs
            .get(self.ws_active_tab)
            .map(|t| t.kind == TerminalKind::Shell)
            .unwrap_or(false)
            && self.ws_tabs.len() > 1
    }

    pub fn close_active_tab(&mut self) -> Option<TerminalTab> {
        if !self.can_close_active_tab() {
            return None;
        }
        let idx = self.ws_active_tab.min(self.ws_tabs.len() - 1);
        let removed = self.ws_tabs.remove(idx);
        if self.ws_active_tab >= self.ws_tabs.len() {
            self.ws_active_tab = self.ws_tabs.len().saturating_sub(1);
        }
        self.persist_tabs_for_active_workspace();
        Some(removed)
    }

    pub fn begin_rename_tab(&mut self) {
        let Some(tab) = self.ws_tabs.get(self.ws_active_tab) else {
            return;
        };
        if tab.kind != TerminalKind::Shell {
            return;
        }
        self.rename_tab_input = Some(tab.label.clone());
    }

    pub fn is_renaming_tab(&self) -> bool {
        self.rename_tab_input.is_some()
    }

    pub fn rename_tab_input_mut(&mut self) -> Option<&mut String> {
        self.rename_tab_input.as_mut()
    }

    pub fn cancel_rename_tab(&mut self) {
        self.rename_tab_input = None;
    }

    pub fn apply_rename_tab(&mut self) {
        let Some(name) = self.rename_tab_input.take() else {
            return;
        };
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(tab) = self.ws_tabs.get_mut(self.ws_active_tab) {
            if tab.kind == TerminalKind::Shell {
                tab.label = trimmed.to_string();
            }
        }
        self.persist_tabs_for_active_workspace();
    }

    pub fn begin_add_ssh_workspace(&mut self) {
        if self.ssh_history.is_empty() {
            self.ssh_workspace_input = Some(SshWorkspaceInput::new());
        } else {
            self.ssh_history_picker = Some(SshHistoryPicker { selected: 0 });
        }
    }

    pub fn cancel_ssh_workspace(&mut self) {
        self.ssh_workspace_input = None;
    }

    pub fn cancel_ssh_history_picker(&mut self) {
        self.ssh_history_picker = None;
    }

    pub fn select_ssh_history_entry(&mut self) {
        if let Some(picker) = self.ssh_history_picker.take() {
            if let Some(entry) = self.ssh_history.get(picker.selected) {
                let mut input = SshWorkspaceInput::new();
                input.host = entry.host.clone();
                input.user = entry.user.clone().unwrap_or_default();
                input.path = entry.path.clone();
                self.ssh_workspace_input = Some(input);
            }
        }
    }

    pub fn begin_new_ssh_from_picker(&mut self) {
        self.ssh_history_picker = None;
        self.ssh_workspace_input = Some(SshWorkspaceInput::new());
    }

    pub fn record_ssh_history(&mut self, entry: SshHistoryEntry) {
        self.ssh_history.retain(|e| e != &entry);
        self.ssh_history.insert(0, entry);
        self.ssh_history.truncate(SSH_HISTORY_MAX);
        save_ssh_history(&self.ssh_history);
    }

    pub fn is_adding_ssh_workspace(&self) -> bool {
        self.ssh_workspace_input.is_some()
    }

    pub fn take_ssh_workspace_request(&mut self) -> Option<(String, String, protocol::SshTarget)> {
        let input = self.ssh_workspace_input.take()?;
        let host = input.host.trim().to_string();
        let path = input.path.trim().to_string();
        if host.is_empty() || path.is_empty() {
            return None;
        }
        let user = if input.user.trim().is_empty() {
            None
        } else {
            Some(input.user.trim().to_string())
        };
        let name = format!(
            "{}:{}",
            &host,
            std::path::Path::new(&path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".to_string())
        );
        let target = protocol::SshTarget {
            host,
            user,
            port: None,
        };
        Some((name, path, target))
    }

    pub fn begin_add_workspace(&mut self, initial_path: String) {
        self.dir_browser = Some(DirBrowserState::new(initial_path));
    }

    pub fn cancel_add_workspace(&mut self) {
        self.dir_browser = None;
    }

    pub fn is_adding_workspace(&self) -> bool {
        self.dir_browser.is_some()
    }

    pub fn dir_browser_mut(&mut self) -> Option<&mut DirBrowserState> {
        self.dir_browser.as_mut()
    }

    /// Cancels any pending delete confirmation (workspace or repository).
    pub fn cancel_delete_workspace(&mut self) {
        self.pending_delete_workspace = None;
        self.pending_delete_repo = None;
    }

    /// True while a delete-confirmation modal is showing for a workspace or repo.
    pub fn is_confirming_delete(&self) -> bool {
        self.pending_delete_workspace.is_some() || self.pending_delete_repo.is_some()
    }

    pub fn take_delete_workspace(&mut self) -> Option<WorkspaceId> {
        self.pending_delete_workspace.take()
    }

    pub fn take_delete_repo(&mut self) -> Option<RepositoryId> {
        self.pending_delete_repo.take()
    }

    pub fn cancel_rename_workspace(&mut self) {
        self.rename_workspace_input = None;
    }

    pub fn is_renaming_workspace(&self) -> bool {
        self.rename_workspace_input.is_some()
    }

    pub fn rename_input_mut(&mut self) -> Option<&mut String> {
        self.rename_workspace_input.as_mut()
    }

    pub fn take_rename_request(&mut self) -> Option<(WorkspaceId, String)> {
        let id = self.active_workspace_id()?;
        let name = self.rename_workspace_input.take()?.trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some((id, name))
    }

    pub fn take_rename_request_home(&mut self) -> Option<(WorkspaceId, String)> {
        let id = self.selected_workspace_id()?;
        let name = self.rename_workspace_input.take()?.trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some((id, name))
    }

    pub fn take_add_workspace_request(&mut self) -> Option<(String, String)> {
        let browser = self.dir_browser.take()?;
        let trimmed = browser.path_input.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        let name = workspace_name_from_path(&trimmed);
        Some((name, trimmed))
    }

    pub fn take_add_workspace_request_with_path(
        &mut self,
        path: String,
    ) -> Option<(String, String)> {
        self.dir_browser.take()?;
        let trimmed = path.trim().to_string();
        if trimmed.is_empty() {
            return None;
        }
        let name = workspace_name_from_path(&trimmed);
        Some((name, trimmed))
    }

    pub fn set_workspace_git(&mut self, id: WorkspaceId, git: GitState) {
        self.workspace_git.insert(id, git);
        self.clamp_log_selection();
        self.clamp_selected_branches();
    }

    pub fn set_workspace_diff(&mut self, id: WorkspaceId, file: String, diff: String) {
        self.workspace_diff.insert(id, (file, diff));
    }

    pub fn append_terminal_bytes(
        &mut self,
        id: WorkspaceId,
        tab_id: &str,
        kind: protocol::TerminalKind,
        bytes: &[u8],
    ) {
        let core_kind = self.settings.terminal_core;
        let is_new_ws = !self.terminal_state.contains_key(&id);
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(|| WorkspaceTerminalState::new(core_kind));
        let is_new_tab = !state.tabs.contains_key(tab_id);
        let responses = state.tab_mut(tab_id, core_kind).append_bytes(bytes);
        if is_new_ws || is_new_tab {
            self.last_resize_sent.remove(&(id, tab_id.to_string()));
        }

        for response in responses {
            self.pending_cpr_responses
                .push((id, tab_id.to_string(), kind, response));
        }
    }

    pub fn reset_terminal(&mut self, id: WorkspaceId, tab_id: &str) {
        let core_kind = self.settings.terminal_core;
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(|| WorkspaceTerminalState::new(core_kind));
        state.tab_mut(tab_id, core_kind).reset();
        self.last_resize_sent.remove(&(id, tab_id.to_string()));
    }

    pub fn resize_terminal_parser(&mut self, id: WorkspaceId, tab_id: &str, cols: u16, rows: u16) {
        if let Some(state) = self.terminal_state.get_mut(&id) {
            if let Some(tab) = state.tabs.get_mut(tab_id) {
                tab.rebuild_for_size(cols, rows);
            }
        }
    }

    pub fn has_terminal_tab(&self, id: WorkspaceId, tab_id: &str) -> bool {
        self.terminal_state
            .get(&id)
            .and_then(|s| s.tabs.get(tab_id))
            .is_some()
    }

    pub fn scroll_terminal_scrollback(&mut self, id: WorkspaceId, tab_id: &str, delta: isize) {
        let core_kind = self.settings.terminal_core;
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(|| WorkspaceTerminalState::new(core_kind));
        state.tab_mut(tab_id, core_kind).scroll_scrollback(delta);
    }

    pub fn reset_terminal_scrollback(&mut self, id: WorkspaceId, tab_id: &str) {
        let core_kind = self.settings.terminal_core;
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(|| WorkspaceTerminalState::new(core_kind));
        state.tab_mut(tab_id, core_kind).reset_scrollback();
    }

    pub fn terminal_scrollback_active(&self, id: WorkspaceId, tab_id: &str) -> bool {
        self.terminal_state
            .get(&id)
            .and_then(|s| s.tabs.get(tab_id))
            .map(|t| t.scrollback_active())
            .unwrap_or(false)
    }

    /// Returns true when the terminal at `tab_id` still needs a resize command
    /// for the given size. This is a pure check: it records nothing, so a size
    /// is only latched as "sent" once it has actually been delivered to the PTY
    /// (via [`Self::mark_resize_sent`]). Keeping the check and the latch separate
    /// lets the caller retry a dropped resize instead of silently leaving the
    /// PTY at a stale (often wider) size.
    pub fn needs_resize(&self, id: WorkspaceId, tab_id: &str, cols: u16, rows: u16) -> bool {
        let key = (id, tab_id.to_string());
        self.last_resize_sent.get(&key).copied() != Some((cols.max(1), rows.max(1)))
    }

    /// Records that a resize to `cols`x`rows` has been delivered to the PTY for
    /// `tab_id`, so it isn't resent until the rendered size changes again.
    pub fn mark_resize_sent(&mut self, id: WorkspaceId, tab_id: &str, cols: u16, rows: u16) {
        self.last_resize_sent
            .insert((id, tab_id.to_string()), (cols.max(1), rows.max(1)));
    }

    pub fn terminal_lines(&self, id: WorkspaceId, tab_id: &str) -> Vec<Line<'static>> {
        let Some(state) = self.terminal_state.get(&id) else {
            return vec![Line::from("No terminal output yet.")];
        };
        let Some(tab) = state.tabs.get(tab_id) else {
            return vec![Line::from("No terminal output yet.")];
        };
        tab.lines(url_regex())
    }

    /// Returns the terminal cursor position relative to the visible terminal viewport.
    pub fn terminal_cursor_position(&self, id: WorkspaceId, tab_id: &str) -> Option<(u16, u16)> {
        let state = self.terminal_state.get(&id)?;
        let tab = state.tabs.get(tab_id)?;
        tab.cursor_position()
    }

    /// Returns the URL at the given terminal position, if any.
    pub fn url_at_terminal_position(
        &self,
        id: WorkspaceId,
        tab_id: &str,
        row: u16,
        col: u16,
    ) -> Option<String> {
        let re = url_regex();

        let state = self.terminal_state.get(&id)?;
        let tab = state.tabs.get(tab_id)?;
        let row_text = tab.plain_row(row)?;

        for m in re.find_iter(&row_text) {
            // Map byte offsets to column positions for the match range
            let col_start = row_text[..m.start()].chars().count() as u16;
            let col_end = col_start + row_text[m.start()..m.end()].chars().count() as u16;
            if col >= col_start && col < col_end {
                return Some(m.as_str().to_string());
            }
        }
        None
    }

    /// Parses the agent terminal's last screen row for Claude Code status info.
    /// Returns `None` if Claude Code is not detected (via terminal title).
    pub fn agent_status(&self, id: WorkspaceId) -> Option<AgentStatus> {
        let state = self.terminal_state.get(&id)?;
        let tab = state.tabs.get("agent")?;

        // Detect Claude Code via terminal title
        if !tab.title().to_ascii_lowercase().contains("claude") {
            return None;
        }

        let text = tab.last_row_text();

        let model = ["Opus", "Sonnet", "Haiku"]
            .iter()
            .find(|m| text.contains(**m))
            .map(|m| m.to_string());

        let effort = if let Some(pos) = text.find("Effort:") {
            let after = text[pos + 7..].trim_start();
            after.split_whitespace().next().map(|s| s.to_string())
        } else {
            None
        };

        let context_pct = text
            .split_whitespace()
            .find(|w| w.ends_with('%') && w[..w.len() - 1].chars().all(|c| c.is_ascii_digit()))
            .map(|s| s.to_string());

        Some(AgentStatus {
            model,
            effort,
            context_pct,
        })
    }

    /// Extracts the most recent `num_lines` non-empty rows from a workspace's
    /// agent terminal, with full ANSI styling.
    ///
    /// Walks the visible screen bottom-to-top (freshest output first), then
    /// continues into scrollback (offset 0 = most recently scrolled off) until
    /// `num_lines` rows are collected.  The result is in chronological order.
    pub fn tile_preview_lines(
        &self,
        id: WorkspaceId,
        max_cols: u16,
        num_lines: u16,
    ) -> Vec<Line<'static>> {
        let Some(state) = self.terminal_state.get(&id) else {
            return Vec::new();
        };
        let Some(tab) = state.tabs.get("agent") else {
            return Vec::new();
        };
        tab.preview_lines(max_cols, num_lines)
    }

    pub fn terminal_mouse_state(
        &self,
        id: WorkspaceId,
        tab_id: &str,
    ) -> Option<(vt100::MouseProtocolMode, vt100::MouseProtocolEncoding, bool)> {
        let state = self.terminal_state.get(&id)?;
        let tab = state.tabs.get(tab_id)?;
        Some(tab.mouse_state())
    }

    pub fn tag_map(&self) -> HashMap<String, Vec<String>> {
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        if let Some(id) = self.active_workspace_id() {
            if let Some(git) = self.workspace_git.get(&id) {
                for t in &git.tags {
                    map.entry(t.hash.clone()).or_default().push(t.name.clone());
                }
            }
        }
        map
    }

    pub fn uncommitted_rows(&self) -> Vec<UncommittedRow> {
        let Some(id) = self.active_workspace_id() else {
            return Vec::new();
        };
        let Some(git) = self.workspace_git.get(&id) else {
            return Vec::new();
        };
        self.uncommitted_rows_for_workspace(id, git)
    }

    fn uncommitted_rows_for_workspace(
        &self,
        id: WorkspaceId,
        git: &GitState,
    ) -> Vec<UncommittedRow> {
        build_uncommitted_rows(git, self.collapsed_uncommitted_dirs.get(&id))
    }

    pub fn toggle_uncommitted_directory(&mut self, path: &str) {
        let Some(id) = self.active_workspace_id() else {
            return;
        };
        let collapsed_dirs = self.collapsed_uncommitted_dirs.entry(id).or_default();
        if !collapsed_dirs.insert(path.to_string()) {
            collapsed_dirs.remove(path);
        }
        self.clamp_log_selection();
    }

    pub fn toggle_selected_uncommitted_directory(&mut self) -> bool {
        let LogItem::ChangedDirectory(path) = self.log_item_at(self.ws_selected_commit) else {
            return false;
        };
        self.toggle_uncommitted_directory(&path);
        true
    }

    pub fn total_log_items(&self) -> usize {
        let Some(id) = self.active_workspace_id() else {
            return 1; // just the header
        };
        let Some(git) = self.workspace_git.get(&id) else {
            return 1;
        };
        let uncommitted_count = if self.ws_uncommitted_expanded && !git.changed.is_empty() {
            self.uncommitted_rows_for_workspace(id, git).len()
        } else {
            0
        };
        if self.ws_tag_filter {
            let tag_map = self.tag_map();
            let mut count = 1 + uncommitted_count; // header + visible uncommitted rows
            for (i, c) in git.recent_commits.iter().enumerate() {
                if !tag_map.contains_key(&c.hash) {
                    continue;
                }
                count += 1;
                if self.ws_expanded_commit == Some(i) {
                    if let Some(files) = self.commit_files_cache.get(&c.hash) {
                        count += files.len();
                    }
                }
            }
            count
        } else {
            let expanded_commit_files = self.expanded_commit_file_count(git);
            1 + uncommitted_count + git.recent_commits.len() + expanded_commit_files
        }
    }

    fn expanded_commit_file_count(&self, git: &GitState) -> usize {
        if let Some(ci) = self.ws_expanded_commit {
            if let Some(commit) = git.recent_commits.get(ci) {
                if let Some(files) = self.commit_files_cache.get(&commit.hash) {
                    return files.len();
                }
            }
        }
        0
    }

    pub fn log_item_at(&self, index: usize) -> LogItem {
        if index == 0 {
            return LogItem::UncommittedHeader;
        }
        let id = match self.active_workspace_id() {
            Some(id) => id,
            None => return LogItem::UncommittedHeader,
        };
        let git = match self.workspace_git.get(&id) {
            Some(g) => g,
            None => return LogItem::UncommittedHeader,
        };
        let uncommitted_rows = if self.ws_uncommitted_expanded && !git.changed.is_empty() {
            self.uncommitted_rows_for_workspace(id, git)
        } else {
            Vec::new()
        };
        let mut offset = index - 1; // subtract header
        if offset < uncommitted_rows.len() {
            return match &uncommitted_rows[offset] {
                UncommittedRow::File { file_index, .. } => LogItem::ChangedFile(*file_index),
                UncommittedRow::Directory { path, .. } => LogItem::ChangedDirectory(path.clone()),
            };
        }
        offset -= uncommitted_rows.len();

        // Commits with optional expanded file lists
        let tag_map = if self.ws_tag_filter {
            Some(self.tag_map())
        } else {
            None
        };
        for i in 0..git.recent_commits.len() {
            if let Some(ref tm) = tag_map {
                if !tm.contains_key(&git.recent_commits[i].hash) {
                    continue;
                }
            }
            if offset == 0 {
                return LogItem::Commit(i);
            }
            offset -= 1;
            if self.ws_expanded_commit == Some(i) {
                if let Some(files) = self.commit_files_cache.get(&git.recent_commits[i].hash) {
                    if offset < files.len() {
                        return LogItem::CommitFile(i, offset);
                    }
                    offset -= files.len();
                }
            }
        }

        LogItem::UncommittedHeader // fallback
    }

    pub fn log_item_is_file_context(&self) -> bool {
        matches!(
            self.log_item_at(self.ws_selected_commit),
            LogItem::UncommittedHeader | LogItem::ChangedFile(_) | LogItem::ChangedDirectory(_)
        )
    }

    pub fn selected_log_file(&self) -> Option<String> {
        if let LogItem::ChangedFile(i) = self.log_item_at(self.ws_selected_commit) {
            let id = self.active_workspace_id()?;
            let git = self.workspace_git.get(&id)?;
            git.changed.get(i).map(|c| c.path.clone())
        } else {
            None
        }
    }

    pub fn selected_uncommitted_path(&self) -> Option<String> {
        match self.log_item_at(self.ws_selected_commit) {
            LogItem::ChangedFile(i) => {
                let id = self.active_workspace_id()?;
                let git = self.workspace_git.get(&id)?;
                git.changed.get(i).map(|c| c.path.clone())
            }
            LogItem::ChangedDirectory(path) => Some(path),
            _ => None,
        }
    }

    pub fn selected_uncommitted_status(&self) -> Option<(String, char, char)> {
        match self.log_item_at(self.ws_selected_commit) {
            LogItem::ChangedFile(i) => {
                let id = self.active_workspace_id()?;
                let git = self.workspace_git.get(&id)?;
                let f = git.changed.get(i)?;
                Some((f.path.clone(), f.index_status, f.worktree_status))
            }
            LogItem::ChangedDirectory(path) => {
                let row = self.uncommitted_rows().into_iter().find(
                    |row| matches!(row, UncommittedRow::Directory { path: p, .. } if p == &path),
                )?;
                let (index_status, worktree_status) = row.status();
                Some((path, index_status, worktree_status))
            }
            _ => None,
        }
    }

    pub fn move_workspace_commit_selection(&mut self, delta: isize) {
        let total = self.total_log_items();
        if total == 0 {
            self.ws_selected_commit = 0;
            return;
        }
        let next = (self.ws_selected_commit as isize + delta).clamp(0, total as isize - 1) as usize;
        self.ws_selected_commit = next;
    }

    pub fn selected_commit_hash(&self) -> Option<String> {
        let ci = match self.log_item_at(self.ws_selected_commit) {
            LogItem::Commit(i) => i,
            LogItem::CommitFile(i, _) => i,
            _ => return None,
        };
        let id = self.active_workspace_id()?;
        let git = self.workspace_git.get(&id)?;
        git.recent_commits.get(ci).map(|c| c.hash.clone())
    }

    pub fn selected_commit_file(&self) -> Option<(String, String)> {
        if let LogItem::CommitFile(ci, fi) = self.log_item_at(self.ws_selected_commit) {
            let id = self.active_workspace_id()?;
            let git = self.workspace_git.get(&id)?;
            let hash = git.recent_commits.get(ci)?.hash.clone();
            let file = self.commit_files_cache.get(&hash)?.get(fi)?.clone();
            Some((hash, file))
        } else {
            None
        }
    }

    pub fn is_committing(&self) -> bool {
        self.commit_input.is_some()
    }

    pub fn is_creating_branch(&self) -> bool {
        self.create_branch_input.is_some()
    }

    pub fn begin_workspace_command(&mut self) {
        let Some(key) = self.active_workspace_command_key() else {
            return;
        };
        self.workspace_commands
            .entry(key)
            .or_insert_with(WorkspaceCommandState::new);
    }

    pub fn close_workspace_command(&mut self) {
        let Some(key) = self.active_workspace_command_key() else {
            return;
        };
        self.workspace_commands.remove(&key);
    }

    pub fn is_workspace_command_open(&self) -> bool {
        self.workspace_command().is_some()
    }

    pub fn workspace_command_mut(&mut self) -> Option<&mut WorkspaceCommandState> {
        let key = self.active_workspace_command_key()?;
        self.workspace_commands.get_mut(&key)
    }

    pub fn workspace_command(&self) -> Option<&WorkspaceCommandState> {
        let key = self.active_workspace_command_key()?;
        self.workspace_commands.get(&key)
    }

    pub fn take_workspace_command_request(&mut self) -> Option<String> {
        let state = self.workspace_command_mut()?;
        if state.running {
            return None;
        }
        let command = state.input.text.trim().to_string();
        if command.is_empty() {
            return None;
        }
        state.running = true;
        state.completed = false;
        state.exit_code = None;
        state.output = format_workspace_command_start(&command);
        state.output_scroll = 0;
        Some(command)
    }

    pub fn apply_workspace_command_result(
        &mut self,
        id: WorkspaceId,
        cwd: String,
        command: String,
        exit_code: Option<i32>,
    ) {
        let Some(key) = self.workspace_command_key_for_event(id, &cwd) else {
            return;
        };
        let Some(state) = self.workspace_commands.get_mut(&key) else {
            return;
        };
        state.running = false;
        state.completed = true;
        state.exit_code = exit_code;
        state.output = finish_workspace_command_output(&state.output, &command, exit_code);
        state.output_scroll = 0;
    }

    pub fn append_workspace_command_output(
        &mut self,
        id: WorkspaceId,
        cwd: String,
        stream: String,
        data: String,
    ) {
        let Some(key) = self.workspace_command_key_for_event(id, &cwd) else {
            return;
        };
        let Some(state) = self.workspace_commands.get_mut(&key) else {
            return;
        };
        if stream == "stderr" && !state.output.contains("[stderr]\n") {
            if !state.output.ends_with("\n\n") {
                state.output.push('\n');
            }
            state.output.push_str("[stderr]\n");
        }
        state.output.push_str(&data.replace('\r', "\n"));
    }

    pub fn scroll_workspace_command_output(&mut self, delta: i16) {
        let Some(state) = self.workspace_command_mut() else {
            return;
        };
        if delta < 0 {
            state.output_scroll = state.output_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            state.output_scroll = state.output_scroll.saturating_add(delta as u16);
        }
    }

    pub fn is_confirming_discard(&self) -> bool {
        self.confirm_discard_file.is_some()
    }

    pub fn begin_discard(&mut self) {
        if let Some(file) = self.selected_uncommitted_path() {
            self.confirm_discard_file = Some(file);
        }
    }

    pub fn cancel_discard(&mut self) {
        self.confirm_discard_file = None;
    }

    pub fn take_discard_file(&mut self) -> Option<String> {
        self.confirm_discard_file.take()
    }

    pub fn is_confirming_discard_all(&self) -> bool {
        self.confirm_discard_all.is_some()
    }

    pub fn begin_discard_all(&mut self, id: WorkspaceId) {
        self.confirm_discard_all = Some(id);
    }

    pub fn cancel_discard_all(&mut self) {
        self.confirm_discard_all = None;
    }

    pub fn take_discard_all(&mut self) -> Option<WorkspaceId> {
        self.confirm_discard_all.take()
    }

    pub fn is_confirming_stash_pull_pop(&self) -> bool {
        self.confirm_stash_pull_pop.is_some()
    }

    pub fn begin_stash_pull_pop(&mut self, id: WorkspaceId) {
        self.confirm_stash_pull_pop = Some(id);
    }

    pub fn cancel_stash_pull_pop(&mut self) {
        self.confirm_stash_pull_pop = None;
    }

    pub fn take_stash_pull_pop(&mut self) -> Option<WorkspaceId> {
        self.confirm_stash_pull_pop.take()
    }

    pub fn is_confirming_delete_branch(&self) -> bool {
        self.confirm_delete_branch.is_some()
    }

    pub fn begin_delete_branch(&mut self) {
        match self.ws_branch_sub_pane {
            BranchSubPane::Local => {
                if let Some(branch) = self.selected_local_branch() {
                    if !branch.is_head {
                        self.confirm_delete_branch = Some(DeleteBranchTarget::Local {
                            branch: branch.name.clone(),
                        });
                    }
                }
            }
            BranchSubPane::Remote => {
                if let Some(rb) = self.selected_remote_branch() {
                    let full_name = rb.full_name.clone();
                    if let Some((remote, branch)) = full_name.split_once('/') {
                        self.confirm_delete_branch = Some(DeleteBranchTarget::Remote {
                            remote: remote.to_string(),
                            branch: branch.to_string(),
                            full_name,
                        });
                    }
                }
            }
        }
    }

    pub fn cancel_delete_branch(&mut self) {
        self.confirm_delete_branch = None;
    }

    pub fn take_delete_branch(&mut self) -> Option<DeleteBranchTarget> {
        self.confirm_delete_branch.take()
    }

    pub fn is_stashing(&self) -> bool {
        self.stash_input.is_some()
    }

    pub fn is_settings_open(&self) -> bool {
        self.settings_open
    }

    pub fn open_settings(&mut self) {
        self.settings_open = true;
        self.settings_selected = 0;
        self.settings_edit_buffer = None;
        self.new_agent_wizard = None;
        self.confirming_delete_agent = false;
    }

    pub fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings_edit_buffer = None;
        self.new_agent_wizard = None;
        self.confirming_delete_agent = false;
    }

    pub fn toggle_yolo_mode(&mut self) {
        self.settings.yolo_mode = !self.settings.yolo_mode;
        let _ = save_settings(&self.settings);
    }

    pub fn toggle_selected_setting(&mut self) {
        if self.settings_edit_buffer.is_some() {
            return; // already editing
        }
        match self.settings_selected {
            0 => {} // default agent uses h/l to cycle
            1 | 2 => {
                // Text fields — Enter/Space starts editing
                let current = match self.settings_selected {
                    1 => self
                        .settings
                        .active_agent()
                        .map(|a| a.command.clone())
                        .unwrap_or_default(),
                    2 => self
                        .settings
                        .active_agent()
                        .map(|a| a.yolo_flags.join(" "))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                self.settings_edit_buffer = Some(current);
            }
            3 => self.settings.attention_notifications = !self.settings.attention_notifications,
            4 => {} // preview_lines uses adjust, not toggle
            5 => self.settings.show_frame_counter = !self.settings.show_frame_counter,
            6 => {
                self.settings_edit_buffer = Some(self.settings.prev_workspace_key.clone());
            }
            7 => {
                self.settings_edit_buffer = Some(self.settings.next_workspace_key.clone());
            }
            8 => {
                self.settings_edit_buffer = Some(self.settings.passthrough_key.clone());
            }
            9 => {
                self.settings_edit_buffer = Some(self.settings.scroll_to_bottom_key.clone());
            }
            10 => {}
            _ => {}
        }
        let _ = save_settings(&self.settings);
    }

    pub fn adjust_selected_setting(&mut self, delta: i16) {
        match self.settings_selected {
            0 => {
                // Cycle through agent profiles
                if !self.settings.agents.is_empty() {
                    let current_idx = self
                        .settings
                        .agents
                        .iter()
                        .position(|a| a.name == self.settings.default_agent)
                        .unwrap_or(0);
                    let len = self.settings.agents.len();
                    let new_idx = if delta > 0 {
                        (current_idx + 1) % len
                    } else {
                        (current_idx + len - 1) % len
                    };
                    self.settings.default_agent = self.settings.agents[new_idx].name.clone();
                }
            }
            4 => {
                let val = self.settings.preview_lines as i16 + delta;
                self.settings.preview_lines = val.clamp(4, 30) as u16;
            }
            10 => {
                let next = self.settings.terminal_core.cycle(delta);
                self.set_terminal_core(next);
                return;
            }
            _ => {}
        }
        let _ = save_settings(&self.settings);
    }

    pub fn settings_count(&self) -> usize {
        11
    }

    pub fn is_editing_setting(&self) -> bool {
        self.settings_edit_buffer.is_some()
    }

    pub fn confirm_setting_edit(&mut self) {
        if let Some(buf) = self.settings_edit_buffer.take() {
            let trimmed = buf.trim().to_string();
            match self.settings_selected {
                1 | 2 => {
                    // Agent fields — find the active agent index
                    let idx = self
                        .settings
                        .agents
                        .iter()
                        .position(|a| a.name == self.settings.default_agent);
                    if let Some(idx) = idx {
                        match self.settings_selected {
                            1 => {
                                if !trimmed.is_empty() {
                                    self.settings.agents[idx].command = trimmed;
                                }
                            }
                            2 => {
                                self.settings.agents[idx].yolo_flags =
                                    trimmed.split_whitespace().map(|s| s.to_string()).collect();
                            }
                            _ => {}
                        }
                    }
                }
                6 => {
                    if !trimmed.is_empty() {
                        self.settings.prev_workspace_key = trimmed;
                    }
                }
                7 => {
                    if !trimmed.is_empty() {
                        self.settings.next_workspace_key = trimmed;
                    }
                }
                8 => {
                    if !trimmed.is_empty() {
                        self.settings.passthrough_key = trimmed;
                    }
                }
                9 => {
                    if !trimmed.is_empty() {
                        self.settings.scroll_to_bottom_key = trimmed;
                    }
                }
                _ => {}
            }
            let _ = save_settings(&self.settings);
        }
    }

    pub fn cancel_setting_edit(&mut self) {
        self.settings_edit_buffer = None;
    }

    /// True when the currently-edited settings row is a keybinding field
    /// (captures a raw key press instead of text input).
    pub fn is_editing_keybind(&self) -> bool {
        self.settings_edit_buffer.is_some() && matches!(self.settings_selected, 6 | 7 | 8 | 9)
    }

    /// Apply a captured keybinding string to the current keybinding row and
    /// immediately confirm the edit so the new binding takes effect.
    pub fn apply_captured_keybind(&mut self, binding: String) {
        if !self.is_editing_keybind() {
            return;
        }
        match self.settings_selected {
            6 => self.settings.prev_workspace_key = binding,
            7 => self.settings.next_workspace_key = binding,
            8 => self.settings.passthrough_key = binding,
            9 => self.settings.scroll_to_bottom_key = binding,
            _ => return,
        }
        self.settings_edit_buffer = None;
        let _ = save_settings(&self.settings);
    }

    pub fn set_terminal_core(&mut self, kind: TerminalCoreKind) {
        if self.settings.terminal_core == kind {
            return;
        }
        self.settings.terminal_core = kind;
        for state in self.terminal_state.values_mut() {
            state.ensure_core_kind(kind);
        }
        self.last_resize_sent.clear();
        let _ = save_settings(&self.settings);
    }

    pub fn begin_new_agent(&mut self) {
        let profile = AgentProfile {
            name: String::new(),
            command: String::new(),
            yolo_flags: Vec::new(),
            continue_flags: Vec::new(),
        };
        self.new_agent_wizard = Some((0, profile, String::new()));
    }

    pub fn is_adding_agent(&self) -> bool {
        self.new_agent_wizard.is_some()
    }

    pub fn new_agent_advance(&mut self) {
        if let Some((step, ref mut profile, ref mut buf)) = self.new_agent_wizard {
            let trimmed = buf.trim().to_string();
            match step {
                0 => {
                    if trimmed.is_empty() {
                        return; // name is required
                    }
                    profile.name = trimmed;
                    *buf = String::new();
                    self.new_agent_wizard.as_mut().unwrap().0 = 1;
                }
                1 => {
                    if trimmed.is_empty() {
                        return; // command is required
                    }
                    profile.command = trimmed;
                    *buf = String::new();
                    self.new_agent_wizard.as_mut().unwrap().0 = 2;
                }
                2 => {
                    profile.yolo_flags =
                        trimmed.split_whitespace().map(|s| s.to_string()).collect();
                    // Done — commit the new agent
                    let finished = profile.clone();
                    self.settings.default_agent = finished.name.clone();
                    self.settings.agents.push(finished);
                    self.new_agent_wizard = None;
                    let _ = save_settings(&self.settings);
                }
                _ => {}
            }
        }
    }

    pub fn cancel_new_agent(&mut self) {
        self.new_agent_wizard = None;
    }

    pub fn begin_delete_agent(&mut self) {
        // Only allow if there's more than one agent
        if self.settings.agents.len() > 1 {
            self.confirming_delete_agent = true;
        }
    }

    pub fn confirm_delete_agent(&mut self) {
        self.confirming_delete_agent = false;
        if let Some(idx) = self
            .settings
            .agents
            .iter()
            .position(|a| a.name == self.settings.default_agent)
        {
            self.settings.agents.remove(idx);
            // Switch default to the first remaining agent
            if let Some(first) = self.settings.agents.first() {
                self.settings.default_agent = first.name.clone();
            }
            let _ = save_settings(&self.settings);
        }
    }

    pub fn cancel_delete_agent(&mut self) {
        self.confirming_delete_agent = false;
    }

    pub fn effective_attention(&self, raw: AttentionLevel) -> AttentionLevel {
        if self.settings.attention_notifications {
            raw
        } else {
            AttentionLevel::None
        }
    }

    pub fn begin_git_op(&mut self, id: WorkspaceId) {
        self.git_op_in_progress.insert(id, Instant::now());
    }

    /// Mark git op as done. Returns `true` if enough time has passed and the op
    /// was actually cleared, `false` if we should defer clearing (minimum
    /// display duration not met).
    pub fn finish_git_op(&mut self, id: WorkspaceId) -> bool {
        const MIN_SPINNER_DURATION: std::time::Duration = std::time::Duration::from_millis(600);
        if let Some(started) = self.git_op_in_progress.get(&id) {
            if started.elapsed() >= MIN_SPINNER_DURATION {
                self.git_op_in_progress.remove(&id);
                return true;
            }
            return false;
        }
        true
    }

    pub fn is_git_op_in_progress(&self, id: WorkspaceId) -> bool {
        self.git_op_in_progress.contains_key(&id)
    }

    pub fn begin_create_branch(&mut self) {
        self.create_branch_input = Some(String::new());
    }

    pub fn cancel_create_branch(&mut self) {
        self.create_branch_input = None;
    }

    pub fn move_branch_selection(&mut self, delta: isize) {
        let Some(id) = self.active_workspace_id() else {
            return;
        };
        let Some(git) = self.workspace_git.get(&id) else {
            return;
        };
        match self.ws_branch_sub_pane {
            BranchSubPane::Local => {
                if git.local_branches.is_empty() {
                    self.ws_selected_local_branch = 0;
                    return;
                }
                let len = git.local_branches.len() as isize;
                let next = (self.ws_selected_local_branch as isize + delta).clamp(0, len - 1);
                self.ws_selected_local_branch = next as usize;
            }
            BranchSubPane::Remote => {
                if git.remote_branches.is_empty() {
                    self.ws_selected_remote_branch = 0;
                    return;
                }
                let len = git.remote_branches.len() as isize;
                let next = (self.ws_selected_remote_branch as isize + delta).clamp(0, len - 1);
                self.ws_selected_remote_branch = next as usize;
            }
        }
    }

    pub fn selected_local_branch(&self) -> Option<&BranchInfo> {
        let id = self.active_workspace_id()?;
        let git = self.workspace_git.get(&id)?;
        git.local_branches.get(self.ws_selected_local_branch)
    }

    pub fn selected_remote_branch(&self) -> Option<&RemoteBranchInfo> {
        let id = self.active_workspace_id()?;
        let git = self.workspace_git.get(&id)?;
        git.remote_branches.get(self.ws_selected_remote_branch)
    }

    pub fn toggle_branch_sub_pane(&mut self, direction: BranchSubPane) {
        self.ws_branch_sub_pane = direction;
    }

    fn clamp_selected_branches(&mut self) {
        let Some(id) = self.active_workspace_id() else {
            return;
        };
        if let Some(git) = self.workspace_git.get(&id) {
            if git.local_branches.is_empty() {
                self.ws_selected_local_branch = 0;
            } else if self.ws_selected_local_branch >= git.local_branches.len() {
                self.ws_selected_local_branch = git.local_branches.len() - 1;
            }
            if git.remote_branches.is_empty() {
                self.ws_selected_remote_branch = 0;
            } else if self.ws_selected_remote_branch >= git.remote_branches.len() {
                self.ws_selected_remote_branch = git.remote_branches.len() - 1;
            }
            if self.ws_pending_select_head_branch {
                if let Some(idx) = git.local_branches.iter().position(|b| b.is_head) {
                    self.ws_selected_local_branch = idx;
                    self.ws_branch_sub_pane = BranchSubPane::Local;
                }
                self.ws_pending_select_head_branch = false;
            }
        }
    }

    fn clamp_log_selection(&mut self) {
        let Some(id) = self.active_workspace_id() else {
            return;
        };
        if let Some(git) = self.workspace_git.get(&id) {
            if git.changed.is_empty() {
                self.ws_uncommitted_expanded = false;
            }
            if let Some(ci) = self.ws_expanded_commit {
                if ci >= git.recent_commits.len() {
                    self.ws_expanded_commit = None;
                }
            }
        }
        let total = self.total_log_items();
        if total == 0 {
            self.ws_selected_commit = 0;
        } else if self.ws_selected_commit >= total {
            self.ws_selected_commit = total - 1;
        }
    }

    fn reconcile_workspace_tab_state(&mut self) {
        let valid_ids = self
            .workspaces
            .iter()
            .map(|w| w.id)
            .collect::<std::collections::HashSet<_>>();
        self.workspace_tabs.retain(|id, _| valid_ids.contains(id));
        for ws in &self.workspaces {
            self.workspace_tabs.entry(ws.id).or_insert_with(|| {
                if let Some(saved) = self.saved_tabs_by_path.get(&ws.path) {
                    sanitize_workspace_tabs(WorkspaceTabsState::from_saved(saved))
                } else {
                    WorkspaceTabsState::default_state()
                }
            });
        }
        if let Some(id) = self.active_workspace_id() {
            self.load_tabs_for_workspace(id);
        }
    }

    fn load_tabs_for_workspace(&mut self, id: WorkspaceId) {
        let from_saved = self
            .workspace_path(id)
            .and_then(|p| self.saved_tabs_by_path.get(&p).cloned())
            .map(|saved| WorkspaceTabsState::from_saved(&saved));
        let fallback =
            sanitize_workspace_tabs(from_saved.unwrap_or_else(WorkspaceTabsState::default_state));
        let state = self.workspace_tabs.entry(id).or_insert(fallback).clone();
        self.ws_tabs = state.tabs;
        self.ws_active_tab = state.active.min(self.ws_tabs.len().saturating_sub(1));
        self.ws_next_shell_tab = state.next_shell_tab.max(2);
    }

    fn persist_tabs_for_active_workspace(&mut self) {
        let Some(id) = self.active_workspace_id() else {
            return;
        };
        let state = sanitize_workspace_tabs(WorkspaceTabsState {
            tabs: self.ws_tabs.clone(),
            active: self.ws_active_tab,
            next_shell_tab: self.ws_next_shell_tab,
        });
        let path_opt = self.workspace_path(id);
        self.workspace_tabs.insert(id, state.clone());
        if let Some(path) = path_opt {
            self.saved_tabs_by_path
                .insert(path, PersistedWorkspaceTabs::from_state(&state));
            let _ = save_saved_tabs_by_path(&self.saved_tabs_by_path);
        }
    }

    fn workspace_path(&self, id: WorkspaceId) -> Option<String> {
        self.workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.path.clone())
    }

    fn active_workspace_command_key(&self) -> Option<WorkspaceCommandKey> {
        let id = self.active_workspace_id()?;
        let cwd = self.workspace_path(id)?;
        Some(self.workspace_command_key_for_workspace(id, cwd))
    }

    fn workspace_command_key_for_event(
        &self,
        id: WorkspaceId,
        cwd: &str,
    ) -> Option<WorkspaceCommandKey> {
        let exact = WorkspaceCommandKey {
            id,
            cwd: cwd.to_string(),
        };
        if self.workspace_commands.contains_key(&exact) {
            return Some(exact);
        }
        self.workspace_commands
            .keys()
            .find(|key| key.id == id)
            .cloned()
    }

    fn workspace_command_key_for_workspace(
        &self,
        id: WorkspaceId,
        cwd: String,
    ) -> WorkspaceCommandKey {
        let current = WorkspaceCommandKey { id, cwd };
        if self.workspace_commands.contains_key(&current) {
            return current;
        }
        self.workspace_commands
            .keys()
            .find(|key| key.id == id)
            .cloned()
            .unwrap_or(current)
    }
}

fn take_matching_prompt(
    slot: &mut Option<(WorkspaceId, String)>,
    id: WorkspaceId,
) -> Option<String> {
    match slot.take() {
        Some((prompt_id, prompt)) if prompt_id == id => Some(prompt),
        other => {
            *slot = other;
            None
        }
    }
}

/// Derives a workspace display name from a filesystem path,
/// falling back to `"workspace"` if the path has no file-name component.
fn workspace_name_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "workspace".to_string())
}

fn format_workspace_command_start(command: &str) -> String {
    let mut out = String::with_capacity(command.len() + 16);
    out.push_str("$ ");
    out.push_str(command);
    out.push_str("\n[running]\n\n");
    out
}

fn finish_workspace_command_output(output: &str, command: &str, exit_code: Option<i32>) -> String {
    let status = match exit_code {
        Some(code) => format!("[exit {code}]"),
        None => "[terminated]".to_string(),
    };
    let mut out = if output.is_empty() {
        format_workspace_command_start(command)
    } else {
        output.to_string()
    };
    if let Some(pos) = out.find("[running]") {
        out.replace_range(pos..pos + "[running]".len(), &status);
    } else {
        out.push('\n');
        out.push_str(&status);
        out.push('\n');
    }
    if out.ends_with("\n\n") {
        out.push_str("(no output)");
    }
    out
}

#[derive(Clone)]
pub struct WorkspaceTabsState {
    pub tabs: Vec<TerminalTab>,
    pub active: usize,
    pub next_shell_tab: u32,
}

impl WorkspaceTabsState {
    fn default_state() -> Self {
        Self {
            tabs: vec![
                TerminalTab::agent(),
                TerminalTab::shell("shell".to_string(), "shell".to_string()),
            ],
            active: 0,
            next_shell_tab: 2,
        }
    }

    fn from_saved(saved: &PersistedWorkspaceTabs) -> Self {
        Self {
            tabs: saved
                .tabs
                .iter()
                .map(|t| TerminalTab {
                    id: t.id.clone(),
                    label: t.label.clone(),
                    kind: t.kind,
                    fullscreen: false,
                    last_command: None,
                    overlay_dismissed: false,
                })
                .collect(),
            active: saved.active,
            next_shell_tab: saved.next_shell_tab,
        }
    }
}

fn sanitize_workspace_tabs(mut state: WorkspaceTabsState) -> WorkspaceTabsState {
    if state.tabs.is_empty() {
        return WorkspaceTabsState::default_state();
    }
    let has_agent = state.tabs.iter().any(|t| t.kind == TerminalKind::Agent);
    if !has_agent {
        state.tabs.insert(0, TerminalTab::agent());
    }
    let has_shell = state.tabs.iter().any(|t| t.kind == TerminalKind::Shell);
    if !has_shell {
        state
            .tabs
            .push(TerminalTab::shell("shell".to_string(), "shell".to_string()));
    }
    state.active = state.active.min(state.tabs.len().saturating_sub(1));
    state.next_shell_tab = state.next_shell_tab.max(2);
    state
}

pub struct AgentStatus {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub context_pct: Option<String>,
}

#[derive(Clone)]
pub struct TerminalTab {
    pub id: String,
    pub label: String,
    pub kind: TerminalKind,
    /// When true, git panes are hidden and the terminal fills the workspace.
    pub fullscreen: bool,
    pub last_command: Option<SavedCommand>,
    pub overlay_dismissed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedWorkspaceTabs {
    pub tabs: Vec<PersistedTab>,
    pub active: usize,
    pub next_shell_tab: u32,
}

impl PersistedWorkspaceTabs {
    fn from_state(state: &WorkspaceTabsState) -> Self {
        Self {
            tabs: state
                .tabs
                .iter()
                .map(|t| PersistedTab {
                    id: t.id.clone(),
                    label: t.label.clone(),
                    kind: t.kind,
                })
                .collect(),
            active: state.active,
            next_shell_tab: state.next_shell_tab,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedTab {
    pub id: String,
    pub label: String,
    pub kind: TerminalKind,
}

impl TerminalTab {
    fn agent() -> Self {
        Self {
            id: "agent".to_string(),
            label: "agent".to_string(),
            kind: TerminalKind::Agent,
            fullscreen: false,
            last_command: None,
            overlay_dismissed: false,
        }
    }

    fn shell(id: String, label: String) -> Self {
        Self {
            id,
            label,
            kind: TerminalKind::Shell,
            fullscreen: false,
            last_command: None,
            overlay_dismissed: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct TabsPersistFile {
    workspaces: HashMap<String, PersistedWorkspaceTabs>,
}

fn load_saved_tabs_by_path() -> HashMap<String, PersistedWorkspaceTabs> {
    let Some(path) = tabs_persist_path() else {
        return HashMap::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str::<TabsPersistFile>(&raw)
        .map(|f| f.workspaces)
        .unwrap_or_default()
}

fn save_saved_tabs_by_path(
    workspaces: &HashMap<String, PersistedWorkspaceTabs>,
) -> anyhow::Result<()> {
    let Some(path) = tabs_persist_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let file = TabsPersistFile {
        workspaces: workspaces.clone(),
    };
    let raw = serde_json::to_string_pretty(&file)?;
    fs::write(path, raw)?;
    Ok(())
}

fn tabs_persist_path() -> Option<PathBuf> {
    conduit_config_path("tui_tabs.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub yolo_flags: Vec<String>,
    #[serde(default)]
    pub continue_flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub attention_notifications: bool,
    #[serde(default = "default_preview_lines")]
    pub preview_lines: u16,
    #[serde(default)]
    pub yolo_mode: bool,
    #[serde(default = "default_agents")]
    pub agents: Vec<AgentProfile>,
    #[serde(default = "default_default_agent")]
    pub default_agent: String,
    #[serde(default)]
    pub show_frame_counter: bool,
    #[serde(default = "default_prev_workspace_key")]
    pub prev_workspace_key: String,
    #[serde(default = "default_next_workspace_key")]
    pub next_workspace_key: String,
    #[serde(default = "default_passthrough_key")]
    pub passthrough_key: String,
    #[serde(default = "default_scroll_to_bottom_key")]
    pub scroll_to_bottom_key: String,
    #[serde(default = "default_terminal_core")]
    pub terminal_core: TerminalCoreKind,
}

fn default_true() -> bool {
    true
}

fn default_preview_lines() -> u16 {
    12
}

fn default_agents() -> Vec<AgentProfile> {
    vec![
        AgentProfile {
            name: "claude".to_string(),
            command: "claude".to_string(),
            yolo_flags: vec!["--dangerously-skip-permissions".to_string()],
            continue_flags: vec!["-c".to_string()],
        },
        AgentProfile {
            name: "codex".to_string(),
            command: "codex".to_string(),
            yolo_flags: vec!["--full-auto".to_string()],
            continue_flags: Vec::new(),
        },
    ]
}

fn default_default_agent() -> String {
    "claude".to_string()
}

fn default_prev_workspace_key() -> String {
    "ctrl+shift+h".to_string()
}

fn default_next_workspace_key() -> String {
    "ctrl+shift+l".to_string()
}

fn default_passthrough_key() -> String {
    "ctrl+g".to_string()
}

fn default_scroll_to_bottom_key() -> String {
    "ctrl+end".to_string()
}

fn default_terminal_core() -> TerminalCoreKind {
    TerminalCoreKind::Alacritty
}

impl Settings {
    /// Returns the active agent profile (the default, or first if not found).
    pub fn active_agent(&self) -> Option<&AgentProfile> {
        self.agents
            .iter()
            .find(|a| a.name == self.default_agent)
            .or(self.agents.first())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            attention_notifications: true,
            preview_lines: 12,
            yolo_mode: false,
            agents: default_agents(),
            default_agent: default_default_agent(),
            show_frame_counter: false,
            prev_workspace_key: default_prev_workspace_key(),
            next_workspace_key: default_next_workspace_key(),
            passthrough_key: default_passthrough_key(),
            scroll_to_bottom_key: default_scroll_to_bottom_key(),
            terminal_core: default_terminal_core(),
        }
    }
}

fn settings_persist_path() -> Option<PathBuf> {
    conduit_config_path("settings.json")
}

fn load_settings() -> Settings {
    let Some(path) = settings_persist_path() else {
        return Settings::default();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Settings::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_settings(settings: &Settings) -> anyhow::Result<()> {
    let Some(path) = settings_persist_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(settings)?;
    fs::write(path, raw)?;
    Ok(())
}

fn ssh_history_path() -> Option<PathBuf> {
    conduit_config_path("ssh_history.json")
}

fn conduit_config_path(file_name: &str) -> Option<PathBuf> {
    Some(conduit_config_root()?.join("conduit").join(file_name))
}

#[cfg(not(test))]
fn conduit_config_root() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        Some(PathBuf::from(xdg))
    } else if let Ok(home) = std::env::var("HOME") {
        Some(PathBuf::from(home).join(".config"))
    } else {
        None
    }
}

#[cfg(test)]
thread_local! {
    static TEST_CONFIG_ROOT: std::cell::RefCell<Option<PathBuf>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn conduit_config_root() -> Option<PathBuf> {
    TEST_CONFIG_ROOT.with(|root| root.borrow().clone())
}

#[cfg(test)]
struct TestConfigRootGuard {
    previous: Option<PathBuf>,
}

#[cfg(test)]
fn use_test_config_root(root: PathBuf) -> TestConfigRootGuard {
    let previous = TEST_CONFIG_ROOT.with(|current| current.replace(Some(root)));
    TestConfigRootGuard { previous }
}

#[cfg(test)]
impl Drop for TestConfigRootGuard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        TEST_CONFIG_ROOT.with(|current| {
            current.replace(previous);
        });
    }
}

fn load_ssh_history() -> Vec<SshHistoryEntry> {
    let Some(path) = ssh_history_path() else {
        return Vec::new();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_ssh_history(history: &[SshHistoryEntry]) {
    let Some(path) = ssh_history_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string_pretty(history) {
        let _ = fs::write(path, raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{
        AttentionLevel, BranchInfo, ChangedFile, CommitInfo, GitState, RemoteBranchInfo,
        WorkspaceSummary,
    };
    use uuid::Uuid;

    fn make_ws(name: &str) -> WorkspaceSummary {
        WorkspaceSummary {
            id: Uuid::new_v4(),
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            branch: Some("main".into()),
            ahead: Some(0),
            behind: Some(0),
            dirty_files: 0,
            attention: AttentionLevel::None,
            agent_running: false,
            agent_active: false,
            shell_running: false,
            last_activity_unix_ms: 0,
            ssh_host: None,
            repository_id: None,
            base_branch: None,
            ready_for_review: false,
            agent: None,
        }
    }

    fn make_git_state() -> GitState {
        GitState {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            ahead: Some(0),
            behind: Some(0),
            changed: vec![
                ChangedFile {
                    path: "a.rs".into(),
                    index_status: 'M',
                    worktree_status: ' ',
                },
                ChangedFile {
                    path: "b.rs".into(),
                    index_status: '?',
                    worktree_status: '?',
                },
            ],
            recent_commits: vec![
                CommitInfo {
                    hash: "abc".into(),
                    message: "first".into(),
                    author: "dev".into(),
                    date: "1h".into(),
                },
                CommitInfo {
                    hash: "def".into(),
                    message: "second".into(),
                    author: "dev".into(),
                    date: "2h".into(),
                },
            ],
            local_branches: vec![
                BranchInfo {
                    name: "main".into(),
                    is_head: true,
                    ahead: None,
                    behind: None,
                },
                BranchInfo {
                    name: "dev".into(),
                    is_head: false,
                    ahead: Some(1),
                    behind: None,
                },
            ],
            remote_branches: vec![RemoteBranchInfo {
                full_name: "origin/main".into(),
            }],
            tags: vec![],
        }
    }

    fn make_nested_git_state() -> GitState {
        let mut git = make_git_state();
        git.changed = vec![
            ChangedFile {
                path: "agent-skills/SKILL.md".into(),
                index_status: '?',
                worktree_status: '?',
            },
            ChangedFile {
                path: "agent-skills/guides/setup.md".into(),
                index_status: '?',
                worktree_status: '?',
            },
            ChangedFile {
                path: "Cargo.toml".into(),
                index_status: 'M',
                worktree_status: ' ',
            },
        ];
        git
    }

    fn app_with_workspaces(n: usize) -> TuiApp {
        let mut app = TuiApp::default();
        let ws: Vec<_> = (0..n).map(|i| make_ws(&format!("ws{i}"))).collect();
        app.set_workspaces(ws);
        app
    }

    fn app_with_clean_open_workspace() -> (TuiApp, WorkspaceId) {
        let mut app = TuiApp::default();
        app.saved_tabs_by_path.clear();
        let ws = make_ws("resurrection");
        let id = ws.id;
        app.set_workspaces(vec![ws]);
        app.open_workspace(id);
        app.ws_active_tab = app
            .ws_tabs
            .iter()
            .position(|tab| tab.kind == TerminalKind::Shell)
            .expect("shell tab");
        (app, id)
    }

    fn saved_command(argv: &[&str], cwd: &str) -> SavedCommand {
        SavedCommand {
            argv: argv.iter().map(|s| (*s).to_string()).collect(),
            cwd: cwd.to_string(),
        }
    }

    #[test]
    fn agent_startup_fast_nonzero_exit_falls_back_once() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        let now = Instant::now();

        app.record_agent_startup_at(id, false, now);
        assert!(matches!(
            app.handle_agent_exit_at(id, Some(1), now + Duration::from_secs(1)),
            AgentExitAction::Fallback { prompt: None }
        ));
        assert_eq!(
            app.handle_agent_exit_at(id, Some(1), now + Duration::from_secs(1)),
            AgentExitAction::RespawnShell
        );
    }

    #[test]
    fn agent_startup_zero_exit_does_not_fallback() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        let now = Instant::now();

        app.record_agent_startup_at(id, false, now);

        assert_eq!(
            app.handle_agent_exit_at(id, Some(0), now + Duration::from_secs(1)),
            AgentExitAction::RespawnShell
        );
    }

    #[test]
    fn agent_startup_slow_exit_does_not_fallback() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        let now = Instant::now();

        app.record_agent_startup_at(id, false, now);

        assert_eq!(
            app.handle_agent_exit_at(
                id,
                Some(1),
                now + AGENT_STARTUP_FAILURE_WINDOW + Duration::from_millis(1),
            ),
            AgentExitAction::RespawnShell
        );
    }

    #[test]
    fn agent_startup_fallback_fast_exit_does_not_retry() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        let now = Instant::now();

        app.record_agent_startup_at(id, true, now);

        assert_eq!(
            app.handle_agent_exit_at(id, None, now + Duration::from_secs(1)),
            AgentExitAction::RespawnShell
        );
    }

    #[test]
    fn agent_startup_prompt_sent_during_failed_startup_is_requeued() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        let now = Instant::now();

        app.record_agent_startup_at(id, false, now);
        app.record_agent_prompt_sent(id, "fix this");

        assert_eq!(
            app.handle_agent_exit_at(id, Some(1), now + Duration::from_secs(1)),
            AgentExitAction::Fallback {
                prompt: Some("fix this".to_string()),
            }
        );
    }

    #[test]
    fn resize_only_latches_once_marked_sent() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();

        // A never-before-sized terminal always needs a resize.
        assert!(app.needs_resize(id, "agent", 80, 24));

        // Simulating a *dropped* resize command (parser rebuilt or not, but the
        // PTY command never delivered): without marking it sent, the next frame
        // must retry rather than leave the PTY at a stale width.
        assert!(app.needs_resize(id, "agent", 80, 24));

        // Once the resize is actually delivered we latch it and stop resending.
        app.mark_resize_sent(id, "agent", 80, 24);
        assert!(!app.needs_resize(id, "agent", 80, 24));

        // A new size needs sending again; an unrelated tab is independent.
        assert!(app.needs_resize(id, "agent", 100, 24));
        assert!(app.needs_resize(id, "shell", 80, 24));
    }

    #[test]
    fn editable_text_inserts_and_moves_cursor() {
        let mut input = EditableText::default();
        for c in "git status".chars() {
            input.insert_char(c);
        }
        input.move_left();
        input.move_left();
        input.insert_char('-');
        assert_eq!(input.text, "git stat-us");
        assert_eq!(input.cursor_char_index(), 9);
    }

    #[test]
    fn editable_text_backspace_and_delete_handle_utf8() {
        let mut input = EditableText::default();
        for c in "aéz".chars() {
            input.insert_char(c);
        }
        input.move_left();
        input.backspace();
        assert_eq!(input.text, "az");
        input.delete();
        assert_eq!(input.text, "a");
    }

    #[test]
    fn workspace_command_request_marks_running() {
        let (mut app, _id) = app_with_clean_open_workspace();
        app.begin_workspace_command();
        let state = app.workspace_command_mut().unwrap();
        for c in "git status".chars() {
            state.input.insert_char(c);
        }
        assert_eq!(
            app.take_workspace_command_request(),
            Some("git status".into())
        );
        assert!(app.workspace_command().unwrap().running);
        assert!(app
            .workspace_command()
            .unwrap()
            .output
            .contains("[running]"));
    }

    #[test]
    fn workspace_command_result_finishes_streamed_output() {
        let (mut app, id) = app_with_clean_open_workspace();
        app.begin_workspace_command();
        let cwd = app.workspace_path(id).unwrap();
        for c in "printf hi".chars() {
            app.workspace_command_mut().unwrap().input.insert_char(c);
        }
        app.take_workspace_command_request();
        app.append_workspace_command_output(id, cwd.clone(), "stdout".into(), "hi\n".into());
        app.apply_workspace_command_result(id, cwd, "printf hi".into(), Some(0));
        let state = app.workspace_command().unwrap();
        assert!(state.completed);
        assert!(!state.running);
        assert!(state.output.contains("[exit 0]"));
        assert!(state.output.contains("hi"));
    }

    #[test]
    fn workspace_command_events_fall_back_to_workspace_id() {
        let (mut app, id) = app_with_clean_open_workspace();
        app.begin_workspace_command();
        for c in "printf hi".chars() {
            app.workspace_command_mut().unwrap().input.insert_char(c);
        }
        app.take_workspace_command_request();

        let event_cwd = "/tmp/same-workspace-different-display".to_string();
        app.append_workspace_command_output(id, event_cwd.clone(), "stdout".into(), "hi\n".into());
        app.apply_workspace_command_result(id, event_cwd, "printf hi".into(), Some(0));

        let state = app.workspace_command().unwrap();
        assert!(state.completed);
        assert!(!state.running);
        assert!(state.output.contains("[exit 0]"));
        assert!(state.output.contains("hi"));
    }

    #[test]
    fn workspace_command_survives_workspace_path_display_change() {
        let (mut app, id) = app_with_clean_open_workspace();
        app.begin_workspace_command();
        for c in "printf hi".chars() {
            app.workspace_command_mut().unwrap().input.insert_char(c);
        }
        app.take_workspace_command_request();

        let mut workspaces = app.workspaces.clone();
        workspaces[0].path = "/tmp/same-workspace-renamed".to_string();
        app.set_workspaces(workspaces);
        assert!(app.is_workspace_command_open());

        app.apply_workspace_command_result(
            id,
            "/tmp/same-workspace-renamed".into(),
            "printf hi".into(),
            Some(0),
        );

        let state = app.workspace_command().unwrap();
        assert!(state.completed);
        assert!(!state.running);
    }

    #[test]
    fn workspace_command_state_is_workspace_specific() {
        let mut app = app_with_workspaces(2);
        let first = app.workspaces[0].id;
        let second = app.workspaces[1].id;
        app.open_workspace(first);
        app.begin_workspace_command();
        assert!(app.is_workspace_command_open());
        app.open_workspace(second);
        assert!(!app.is_workspace_command_open());
        app.open_workspace(first);
        assert!(app.is_workspace_command_open());
    }

    // ===== Navigation =====

    #[test]
    fn open_workspace_sets_route_and_focus() {
        let mut app = app_with_workspaces(2);
        let id = app.workspaces[1].id;
        app.open_workspace(id);
        assert!(matches!(app.route, Route::Workspace { id: wid } if wid == id));
        assert_eq!(app.focus, Focus::WsTerminal);
    }

    #[test]
    fn go_home_preserves_tab_fullscreen() {
        let mut app = app_with_workspaces(2);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.toggle_terminal_fullscreen();
        assert!(app.terminal_fullscreen());
        app.go_home();
        assert!(matches!(app.route, Route::Home));
        assert_eq!(app.focus, Focus::Sidebar);
    }

    // ===== Terminal tab management =====

    #[test]
    fn add_shell_tab_increments_counter() {
        let mut app = TuiApp::default();
        let initial = app.ws_next_shell_tab;
        app.add_shell_tab();
        assert_eq!(app.ws_next_shell_tab, initial + 1);
        assert_eq!(app.ws_active_tab, app.ws_tabs.len() - 1);
        assert_eq!(app.ws_tabs.last().unwrap().kind, TerminalKind::Shell);
    }

    #[test]
    fn cannot_close_agent_tab() {
        let mut app = TuiApp::default();
        app.ws_active_tab = 0; // agent tab
        assert!(!app.can_close_active_tab());
        assert!(app.close_active_tab().is_none());
    }

    #[test]
    fn cannot_close_last_tab() {
        let mut app = TuiApp::default();
        // Remove all shell tabs, keep only agent
        app.ws_tabs = vec![TerminalTab {
            id: "agent".into(),
            label: "agent".into(),
            kind: TerminalKind::Agent,
            fullscreen: false,
            last_command: None,
            overlay_dismissed: false,
        }];
        app.ws_active_tab = 0;
        assert!(!app.can_close_active_tab());
    }

    #[test]
    fn close_shell_tab() {
        let mut app = TuiApp::default();
        app.add_shell_tab(); // now 3 tabs: agent, shell, shell-2
        app.ws_active_tab = 2; // select last shell
        assert!(app.can_close_active_tab());
        let removed = app.close_active_tab();
        assert!(removed.is_some());
        assert_eq!(app.ws_tabs.len(), 2);
    }

    #[test]
    fn move_terminal_tab_clamps() {
        let mut app = TuiApp::default(); // 2 tabs
        app.ws_active_tab = 0;
        app.move_terminal_tab(-1);
        assert_eq!(app.ws_active_tab, 0);
        app.move_terminal_tab(100);
        assert_eq!(app.ws_active_tab, app.ws_tabs.len() - 1);
    }

    #[test]
    fn toggle_terminal_command_mode() {
        let mut app = TuiApp::default();
        app.ws_active_tab = 1; // shell tab
        assert!(!app.terminal_command_mode());
        app.toggle_terminal_command_mode();
        assert!(app.terminal_command_mode());
        app.toggle_terminal_command_mode();
        assert!(!app.terminal_command_mode());
    }

    #[test]
    fn shell_resurrection_changed_shows_overlay_for_active_shell() {
        let (mut app, id) = app_with_clean_open_workspace();
        let cmd = saved_command(&["sleep", "300"], "/tmp/resurrection");

        app.apply_shell_resurrection_change(id, app.active_tab_id(), Some(cmd.clone()));

        assert_eq!(app.pending_resurrect_command(), Some(&cmd));
    }

    #[test]
    fn take_resurrect_command_clears_overlay() {
        let (mut app, id) = app_with_clean_open_workspace();
        let tab_id = app.active_tab_id();
        let cmd = saved_command(&["cargo", "test"], "/tmp/resurrection");
        app.apply_shell_resurrection_change(id, tab_id, Some(cmd.clone()));

        assert_eq!(app.take_resurrect_command(), Some(cmd));
        assert!(app.pending_resurrect_command().is_none());
    }

    #[test]
    fn dismiss_resurrect_overlay_clears_overlay() {
        let (mut app, id) = app_with_clean_open_workspace();
        let tab_id = app.active_tab_id();
        let cmd = saved_command(&["vim"], "/tmp/resurrection");
        app.apply_shell_resurrection_change(id, tab_id, Some(cmd));

        app.dismiss_resurrect_overlay();

        assert!(app.pending_resurrect_command().is_none());
    }

    #[test]
    fn shell_resurrection_changed_none_clears_overlay() {
        let (mut app, id) = app_with_clean_open_workspace();
        let tab_id = app.active_tab_id();
        let cmd = saved_command(&["sleep", "300"], "/tmp/resurrection");
        app.apply_shell_resurrection_change(id, tab_id.clone(), Some(cmd));

        app.apply_shell_resurrection_change(id, tab_id, None);

        assert!(app.pending_resurrect_command().is_none());
    }

    #[test]
    fn live_foreground_change_does_not_show_resurrection_overlay() {
        let (mut app, id) = app_with_clean_open_workspace();
        let cmd = saved_command(&["sleep", "300"], "/tmp/resurrection");

        app.apply_foreground_change(id, app.active_tab_id(), Some(cmd));

        assert!(app.pending_resurrect_command().is_none());
    }

    // ===== Git log navigation =====

    #[test]
    fn total_log_items_no_workspace() {
        let app = TuiApp::default();
        assert_eq!(app.total_log_items(), 1); // just header
    }

    #[test]
    fn total_log_items_with_git() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        // header(1) + 2 commits = 3 (uncommitted not expanded)
        assert_eq!(app.total_log_items(), 3);
    }

    #[test]
    fn total_log_items_with_expanded_uncommitted() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_uncommitted_expanded = true;
        // header(1) + 2 changed files + 2 commits = 5
        assert_eq!(app.total_log_items(), 5);
    }

    #[test]
    fn log_item_at_index_zero_is_header() {
        let app = TuiApp::default();
        assert_eq!(app.log_item_at(0), LogItem::UncommittedHeader);
    }

    #[test]
    fn log_item_at_commits() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        assert_eq!(app.log_item_at(1), LogItem::Commit(0));
        assert_eq!(app.log_item_at(2), LogItem::Commit(1));
    }

    #[test]
    fn log_item_at_expanded_files() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_uncommitted_expanded = true;
        assert_eq!(app.log_item_at(0), LogItem::UncommittedHeader);
        assert_eq!(app.log_item_at(1), LogItem::ChangedFile(0));
        assert_eq!(app.log_item_at(2), LogItem::ChangedFile(1));
        assert_eq!(app.log_item_at(3), LogItem::Commit(0));
    }

    #[test]
    fn uncommitted_rows_build_nested_tree() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_nested_git_state());
        app.ws_uncommitted_expanded = true;

        let rows = app.uncommitted_rows();

        assert_eq!(
            rows,
            vec![
                UncommittedRow::Directory {
                    path: "agent-skills".into(),
                    name: "agent-skills".into(),
                    depth: 0,
                    collapsed: false,
                    index_status: '?',
                    worktree_status: '?',
                },
                UncommittedRow::File {
                    file_index: 0,
                    path: "agent-skills/SKILL.md".into(),
                    name: "SKILL.md".into(),
                    depth: 1,
                    index_status: '?',
                    worktree_status: '?',
                },
                UncommittedRow::Directory {
                    path: "agent-skills/guides".into(),
                    name: "guides".into(),
                    depth: 1,
                    collapsed: false,
                    index_status: '?',
                    worktree_status: '?',
                },
                UncommittedRow::File {
                    file_index: 1,
                    path: "agent-skills/guides/setup.md".into(),
                    name: "setup.md".into(),
                    depth: 2,
                    index_status: '?',
                    worktree_status: '?',
                },
                UncommittedRow::File {
                    file_index: 2,
                    path: "Cargo.toml".into(),
                    name: "Cargo.toml".into(),
                    depth: 0,
                    index_status: 'M',
                    worktree_status: ' ',
                },
            ]
        );
    }

    #[test]
    fn log_item_at_expanded_uncommitted_includes_directory_rows() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_nested_git_state());
        app.ws_uncommitted_expanded = true;

        assert_eq!(app.total_log_items(), 8);
        assert_eq!(app.log_item_at(0), LogItem::UncommittedHeader);
        assert_eq!(
            app.log_item_at(1),
            LogItem::ChangedDirectory("agent-skills".into())
        );
        assert_eq!(app.log_item_at(2), LogItem::ChangedFile(0));
        assert_eq!(
            app.log_item_at(3),
            LogItem::ChangedDirectory("agent-skills/guides".into())
        );
        assert_eq!(app.log_item_at(4), LogItem::ChangedFile(1));
        assert_eq!(app.log_item_at(5), LogItem::ChangedFile(2));
        assert_eq!(app.log_item_at(6), LogItem::Commit(0));
    }

    #[test]
    fn log_item_at_collapsed_directory_skips_descendants() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_nested_git_state());
        app.ws_uncommitted_expanded = true;

        app.toggle_uncommitted_directory("agent-skills");

        assert_eq!(app.total_log_items(), 5);
        assert_eq!(
            app.log_item_at(1),
            LogItem::ChangedDirectory("agent-skills".into())
        );
        assert_eq!(app.log_item_at(2), LogItem::ChangedFile(2));
        assert_eq!(app.log_item_at(3), LogItem::Commit(0));
    }

    #[test]
    fn toggle_selected_uncommitted_directory_collapses_and_expands() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_nested_git_state());
        app.ws_uncommitted_expanded = true;
        app.ws_selected_commit = 1;

        assert!(app.toggle_selected_uncommitted_directory());
        assert_eq!(app.total_log_items(), 5);
        assert!(matches!(
            app.uncommitted_rows().first(),
            Some(UncommittedRow::Directory {
                path,
                collapsed: true,
                ..
            }) if path == "agent-skills"
        ));

        assert!(app.toggle_selected_uncommitted_directory());
        assert_eq!(app.total_log_items(), 8);
        assert!(matches!(
            app.uncommitted_rows().first(),
            Some(UncommittedRow::Directory {
                path,
                collapsed: false,
                ..
            }) if path == "agent-skills"
        ));
    }

    #[test]
    fn selected_uncommitted_status_on_directory_uses_folder_path() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_nested_git_state());
        app.ws_uncommitted_expanded = true;
        app.ws_selected_commit = 1;

        assert_eq!(
            app.selected_uncommitted_status(),
            Some(("agent-skills".to_string(), '?', '?'))
        );
        assert_eq!(app.selected_log_file(), None);
    }

    #[test]
    fn selected_uncommitted_status_on_staged_directory_unstages() {
        let mut git = make_nested_git_state();
        git.changed[0].index_status = 'A';
        git.changed[0].worktree_status = ' ';

        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, git);
        app.ws_uncommitted_expanded = true;
        app.ws_selected_commit = 1;

        assert_eq!(
            app.selected_uncommitted_status(),
            Some(("agent-skills".to_string(), '*', '*'))
        );
    }

    #[test]
    fn move_workspace_commit_selection_clamps() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.move_workspace_commit_selection(-10);
        assert_eq!(app.ws_selected_commit, 0);
        app.move_workspace_commit_selection(100);
        assert_eq!(app.ws_selected_commit, app.total_log_items() - 1);
    }

    // ===== Modal state transitions =====

    #[test]
    fn add_cancel_workspace() {
        let mut app = TuiApp::default();
        assert!(!app.is_adding_workspace());
        app.begin_add_workspace("/tmp".into());
        assert!(app.is_adding_workspace());
        app.cancel_add_workspace();
        assert!(!app.is_adding_workspace());
    }

    #[test]
    fn delete_workspace_flow() {
        let mut app = app_with_workspaces(2);
        assert!(!app.is_confirming_delete());
        app.pending_delete_workspace = app.selected_workspace_id();
        assert!(app.is_confirming_delete());
        let id = app.take_delete_workspace();
        assert!(id.is_some());
        assert!(!app.is_confirming_delete());
    }

    #[test]
    fn cancel_delete_workspace() {
        let mut app = app_with_workspaces(2);
        app.pending_delete_workspace = app.selected_workspace_id();
        app.cancel_delete_workspace();
        assert!(!app.is_confirming_delete());
    }

    #[test]
    fn delete_repo_flow() {
        let mut app = TuiApp::default();
        assert!(!app.is_confirming_delete());
        let repo_id = uuid::Uuid::new_v4();
        app.pending_delete_repo = Some(repo_id);
        assert!(app.is_confirming_delete());
        assert_eq!(app.take_delete_repo(), Some(repo_id));
        assert!(!app.is_confirming_delete());
    }

    #[test]
    fn cancel_clears_both_pending_deletes() {
        let mut app = app_with_workspaces(1);
        app.pending_delete_workspace = app.selected_workspace_id();
        app.pending_delete_repo = Some(uuid::Uuid::new_v4());
        app.cancel_delete_workspace();
        assert!(!app.is_confirming_delete());
        assert!(app.pending_delete_workspace.is_none());
        assert!(app.pending_delete_repo.is_none());
    }

    #[test]
    fn rename_workspace_flow() {
        let mut app = app_with_workspaces(1);
        app.rename_workspace_input = Some("ws-0".to_string());
        assert!(app.is_renaming_workspace());
        app.cancel_rename_workspace();
        assert!(!app.is_renaming_workspace());
    }

    #[test]
    fn settings_open_close() {
        let mut app = TuiApp::default();
        assert!(!app.is_settings_open());
        app.open_settings();
        assert!(app.is_settings_open());
        assert_eq!(app.settings_selected, 0);
        app.close_settings();
        assert!(!app.is_settings_open());
    }

    #[test]
    fn tests_do_not_use_real_config_by_default() {
        assert!(tabs_persist_path().is_none());
        assert!(settings_persist_path().is_none());
        assert!(ssh_history_path().is_none());

        let mut settings = Settings::default();
        settings.passthrough_key = "ctrl+shift+p".to_string();
        save_settings(&settings).unwrap();

        assert_eq!(load_settings().passthrough_key, default_passthrough_key());
    }

    #[test]
    fn settings_persistence_can_use_test_config_root() {
        let root = std::env::temp_dir().join(format!("conduit-config-test-{}", Uuid::new_v4()));
        let _guard = use_test_config_root(root.clone());
        let mut settings = Settings::default();
        settings.passthrough_key = "ctrl+shift+p".to_string();

        save_settings(&settings).unwrap();

        assert_eq!(load_settings().passthrough_key, "ctrl+shift+p");
        assert!(root.join("conduit").join("settings.json").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn scroll_to_bottom_key_captures_and_applies_immediately() {
        use crate::keymap;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = TuiApp::default();
        app.open_settings();
        app.settings_selected = 9;
        // Enter capture mode for the scroll-to-bottom binding row.
        app.toggle_selected_setting();
        assert!(app.is_editing_keybind());

        // Simulate the user pressing Ctrl+Shift+B.
        let pressed = KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let captured = keymap::keybind_from_event(pressed).expect("key produces a binding");
        app.apply_captured_keybind(captured.clone());

        // After capture, the setting is persisted in memory and edit mode is exited.
        assert_eq!(app.settings.scroll_to_bottom_key, captured);
        assert!(!app.is_editing_keybind());

        // And pressing the same key afterwards matches the new binding live.
        assert!(keymap::matches_keybinding(
            pressed,
            &app.settings.scroll_to_bottom_key
        ));
    }

    #[test]
    fn command_mode_key_captures_and_applies_immediately() {
        use crate::keymap;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = TuiApp::default();
        app.open_settings();
        app.settings_selected = 8;
        app.toggle_selected_setting();
        assert!(app.is_editing_keybind());

        let pressed = KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        let captured = keymap::keybind_from_event(pressed).expect("key produces a binding");
        app.apply_captured_keybind(captured.clone());

        assert_eq!(app.settings.passthrough_key, captured);
        assert!(!app.is_editing_keybind());
        assert!(keymap::matches_keybinding(
            pressed,
            &app.settings.passthrough_key
        ));
    }

    #[test]
    fn ssh_workspace_flow_no_history() {
        let mut app = TuiApp::default();
        app.ssh_history.clear();
        app.begin_add_ssh_workspace();
        assert!(app.is_adding_ssh_workspace());
        app.cancel_ssh_workspace();
        assert!(!app.is_adding_ssh_workspace());
    }

    #[test]
    fn ssh_workspace_flow_with_history() {
        let mut app = TuiApp::default();
        app.ssh_history = vec![SshHistoryEntry {
            host: "h".into(),
            user: None,
            path: "/p".into(),
        }];
        app.begin_add_ssh_workspace();
        assert!(app.ssh_history_picker.is_some());
        app.select_ssh_history_entry();
        assert!(app.is_adding_ssh_workspace());
    }

    #[test]
    fn commit_modal() {
        let mut app = TuiApp::default();
        assert!(!app.is_committing());
        app.commit_input = Some("initial".into());
        assert!(app.is_committing());
    }

    #[test]
    fn create_branch_modal() {
        let mut app = TuiApp::default();
        assert!(!app.is_creating_branch());
        app.begin_create_branch();
        assert!(app.is_creating_branch());
        app.cancel_create_branch();
        assert!(!app.is_creating_branch());
    }

    #[test]
    fn discard_flow() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_uncommitted_expanded = true;
        app.ws_selected_commit = 1; // first changed file
        app.begin_discard();
        assert!(app.is_confirming_discard());
        let file = app.take_discard_file();
        assert!(file.is_some());
        assert!(!app.is_confirming_discard());
    }

    #[test]
    fn discard_all_flow() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        assert!(!app.is_confirming_discard_all());
        app.begin_discard_all(id);
        assert!(app.is_confirming_discard_all());
        app.cancel_discard_all();
        assert!(!app.is_confirming_discard_all());
        app.begin_discard_all(id);
        let taken = app.take_discard_all();
        assert_eq!(taken, Some(id));
        assert!(!app.is_confirming_discard_all());
    }

    #[test]
    fn stash_modal() {
        let mut app = TuiApp::default();
        assert!(!app.is_stashing());
        app.stash_input = Some("msg".into());
        assert!(app.is_stashing());
    }

    // ===== Pure helpers =====

    #[test]
    fn mouse_selection_at_confined_clamps_anchor_into_rect() {
        // A press outside the confine band (e.g. on a pane border) anchors inside it.
        let rect = ratatui::layout::Rect::new(2, 1, 6, 4); // cols 2..=7, rows 1..=4
        let sel = MouseSelection::at_confined(0, 0, rect);
        assert_eq!((sel.anchor_col, sel.anchor_row), (2, 1));
        assert!(sel.is_empty());
        let sel = MouseSelection::at_confined(100, 100, rect);
        assert_eq!((sel.anchor_col, sel.anchor_row), (7, 4));
    }

    #[test]
    fn mouse_selection_ordered_forward() {
        let sel = MouseSelection {
            anchor_col: 0,
            anchor_row: 0,
            end_col: 5,
            end_row: 3,
            confine: None,
        };
        let ((sc, sr), (ec, er)) = sel.ordered();
        assert_eq!((sc, sr), (0, 0));
        assert_eq!((ec, er), (5, 3));
    }

    #[test]
    fn mouse_selection_ordered_backward() {
        let sel = MouseSelection {
            anchor_col: 5,
            anchor_row: 3,
            end_col: 0,
            end_row: 0,
            confine: None,
        };
        let ((sc, sr), (ec, er)) = sel.ordered();
        assert_eq!((sc, sr), (0, 0));
        assert_eq!((ec, er), (5, 3));
    }

    #[test]
    fn mouse_selection_not_empty_when_dragged() {
        let mut sel = MouseSelection {
            anchor_col: 0,
            anchor_row: 0,
            end_col: 0,
            end_row: 0,
            confine: None,
        };
        sel.end_col = 5;
        assert!(!sel.is_empty());
    }

    #[test]
    fn ssh_input_cycle_field() {
        let mut input = SshWorkspaceInput::new();
        assert_eq!(input.focused_field, SshField::Host);
        input.cycle_field();
        assert_eq!(input.focused_field, SshField::User);
        input.cycle_field();
        assert_eq!(input.focused_field, SshField::Path);
        input.cycle_field();
        assert_eq!(input.focused_field, SshField::Host);
    }

    #[test]
    fn ssh_input_active_input_mut() {
        let mut input = SshWorkspaceInput::new();
        input.active_input_mut().push_str("host.com");
        assert_eq!(input.host, "host.com");
        input.cycle_field();
        input.active_input_mut().push_str("user");
        assert_eq!(input.user, "user");
        input.cycle_field();
        input.active_input_mut().push_str("/path");
        assert_eq!(input.path, "/path");
    }

    #[test]
    fn effective_attention_with_notifications_on() {
        let app = TuiApp::default();
        assert_eq!(
            app.effective_attention(AttentionLevel::Error),
            AttentionLevel::Error
        );
    }

    #[test]
    fn effective_attention_with_notifications_off() {
        let mut app = TuiApp::default();
        app.settings.attention_notifications = false;
        assert_eq!(
            app.effective_attention(AttentionLevel::Error),
            AttentionLevel::None
        );
    }

    #[test]
    fn begin_finish_git_op() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        assert!(!app.is_git_op_in_progress(id));
        app.begin_git_op(id);
        assert!(app.is_git_op_in_progress(id));
        // finish_git_op returns false when minimum duration not met (just started)
        let cleared = app.finish_git_op(id);
        assert!(!cleared); // too soon
        assert!(app.is_git_op_in_progress(id)); // still in progress
    }

    #[test]
    fn toggle_fullscreen() {
        let mut app = TuiApp::default();
        assert!(!app.terminal_fullscreen());
        app.toggle_terminal_fullscreen();
        assert!(app.terminal_fullscreen());
        app.toggle_terminal_fullscreen();
        assert!(!app.terminal_fullscreen());
    }

    #[test]
    fn fullscreen_is_per_tab() {
        let mut app = TuiApp::default(); // 2 tabs: agent (0), shell (1)

        // Toggle fullscreen on tab 0
        app.ws_active_tab = 0;
        app.toggle_terminal_fullscreen();
        assert!(app.terminal_fullscreen());

        // Tab 1 should still be off
        app.ws_active_tab = 1;
        assert!(!app.terminal_fullscreen());

        // Toggle fullscreen on tab 1
        app.toggle_terminal_fullscreen();
        assert!(app.terminal_fullscreen());

        // Tab 0 should still be on (independent)
        app.ws_active_tab = 0;
        assert!(app.terminal_fullscreen());

        // Toggle off on tab 0
        app.toggle_terminal_fullscreen();
        assert!(!app.terminal_fullscreen());

        // Tab 1 should be unaffected
        app.ws_active_tab = 1;
        assert!(app.terminal_fullscreen());
    }

    #[test]
    fn vt100_keeps_scrollback_for_partial_scroll_regions() {
        let mut parser = vt100::Parser::new(4, 4, 8);
        parser.process(
            b"\x1b[1;1H1111\x1b[2;1H2222\x1b[3;1H3333\x1b[4;1H4444\x1b[2;3r\x1b[3;1HAAAA\r\n",
        );

        parser.set_scrollback(1);
        let rows = parser.screen().rows(0, 4).collect::<Vec<_>>();

        assert_eq!(rows.first().map(String::as_str), Some("2222"));
    }

    #[test]
    fn dir_browser_move_selection() {
        let mut state = DirBrowserState {
            path_input: String::new(),
            entries: vec!["a".into(), "b".into(), "c".into()],
            selected: 0,
            show_hidden: false,
            editing_path: false,
        };
        state.move_selection(1);
        assert_eq!(state.selected, 1);
        state.move_selection(10);
        assert_eq!(state.selected, 2); // clamped
        state.move_selection(-10);
        assert_eq!(state.selected, 0); // clamped
    }

    #[test]
    fn dir_browser_move_selection_empty() {
        let mut state = DirBrowserState {
            path_input: String::new(),
            entries: vec![],
            selected: 0,
            show_hidden: false,
            editing_path: false,
        };
        state.move_selection(1);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn move_branch_selection_local() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_branch_sub_pane = BranchSubPane::Local;
        app.ws_selected_local_branch = 0;
        app.move_branch_selection(1);
        assert_eq!(app.ws_selected_local_branch, 1);
        app.move_branch_selection(100);
        assert_eq!(app.ws_selected_local_branch, 1); // clamped to last
        app.move_branch_selection(-100);
        assert_eq!(app.ws_selected_local_branch, 0); // clamped to first
    }

    #[test]
    fn move_branch_selection_remote() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_branch_sub_pane = BranchSubPane::Remote;
        app.ws_selected_remote_branch = 0;
        app.move_branch_selection(100);
        assert_eq!(app.ws_selected_remote_branch, 0); // only 1 remote branch
    }

    #[test]
    fn set_workspaces_clamps_selection() {
        let mut app = app_with_workspaces(5);
        app.home_selected = 4;
        app.set_workspaces(vec![make_ws("only")]);
        assert_eq!(app.home_selected, 0);
    }

    #[test]
    fn set_workspaces_empty() {
        let mut app = app_with_workspaces(3);
        app.home_selected = 2;
        app.set_workspaces(vec![]);
        assert_eq!(app.home_selected, 0);
    }

    #[test]
    fn cycle_agent_wraps_through_configured_agents() {
        let agents = default_agents(); // [claude, codex]
        let mut qc = QuickCreateState {
            repo_id: Uuid::new_v4(),
            repo_name: "r".into(),
            name: String::new(),
            expanded: true,
            base_branch: String::new(),
            agent: "claude".into(),
            agent_command_edit: false,
            initial_prompt: String::new(),
            field: QuickCreateField::Agent,
        };
        qc.cycle_agent(&agents, 1);
        assert_eq!(qc.agent, "codex");
        qc.cycle_agent(&agents, 1); // wraps back to the first
        assert_eq!(qc.agent, "claude");
        qc.cycle_agent(&agents, -1); // wraps to the last
        assert_eq!(qc.agent, "codex");
    }

    #[test]
    fn cycle_agent_from_custom_command_lands_on_an_agent() {
        let agents = default_agents(); // [claude, codex]
        let mut qc = QuickCreateState {
            repo_id: Uuid::new_v4(),
            repo_name: "r".into(),
            name: String::new(),
            expanded: true,
            base_branch: String::new(),
            agent: "my-custom-cmd".into(), // not a known profile
            agent_command_edit: false,
            initial_prompt: String::new(),
            field: QuickCreateField::Agent,
        };
        qc.cycle_agent(&agents, 1); // forward from unknown → first
        assert_eq!(qc.agent, "claude");
        qc.agent = "my-custom-cmd".into();
        qc.cycle_agent(&agents, -1); // backward from unknown → last
        assert_eq!(qc.agent, "codex");
    }

    #[test]
    fn rename_tab_flow() {
        let mut app = TuiApp::default();
        app.ws_active_tab = 1; // shell tab
        app.begin_rename_tab();
        assert!(app.is_renaming_tab());
        app.rename_tab_input = Some("new-name".into());
        app.apply_rename_tab();
        assert!(!app.is_renaming_tab());
        assert_eq!(app.ws_tabs[1].label, "new-name");
    }

    #[test]
    fn rename_tab_agent_noop() {
        let mut app = TuiApp::default();
        app.ws_active_tab = 0; // agent tab
        app.begin_rename_tab();
        assert!(!app.is_renaming_tab()); // should not allow renaming agent
    }

    #[test]
    fn cancel_rename_tab() {
        let mut app = TuiApp::default();
        app.ws_active_tab = 1;
        app.begin_rename_tab();
        app.cancel_rename_tab();
        assert!(!app.is_renaming_tab());
    }

    #[test]
    fn stash_pull_pop_flow() {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        assert!(!app.is_confirming_stash_pull_pop());
        app.begin_stash_pull_pop(id);
        assert!(app.is_confirming_stash_pull_pop());
        let taken = app.take_stash_pull_pop();
        assert_eq!(taken, Some(id));
        assert!(!app.is_confirming_stash_pull_pop());
    }

    #[test]
    fn cancel_stash_pull_pop() {
        let mut app = TuiApp::default();
        app.begin_stash_pull_pop(Uuid::new_v4());
        app.cancel_stash_pull_pop();
        assert!(!app.is_confirming_stash_pull_pop());
    }

    // ===== A1: workspace_name_from_path =====

    #[test]
    fn workspace_name_from_path_normal() {
        assert_eq!(workspace_name_from_path("/home/user/project"), "project");
    }

    #[test]
    fn workspace_name_from_path_root() {
        assert_eq!(workspace_name_from_path("/"), "workspace");
    }

    #[test]
    fn workspace_name_from_path_hidden() {
        assert_eq!(workspace_name_from_path("/home/user/.hidden"), ".hidden");
    }

    #[test]
    fn workspace_name_from_path_empty() {
        assert_eq!(workspace_name_from_path(""), "workspace");
    }

    // ===== A2: sanitize_workspace_tabs =====

    #[test]
    fn sanitize_workspace_tabs_empty_returns_default() {
        let state = WorkspaceTabsState {
            tabs: vec![],
            active: 0,
            next_shell_tab: 0,
        };
        let result = sanitize_workspace_tabs(state);
        assert_eq!(result.tabs.len(), 2);
        assert_eq!(result.tabs[0].kind, TerminalKind::Agent);
        assert_eq!(result.tabs[1].kind, TerminalKind::Shell);
        assert_eq!(result.next_shell_tab, 2);
    }

    #[test]
    fn sanitize_workspace_tabs_missing_agent_prepends() {
        let state = WorkspaceTabsState {
            tabs: vec![TerminalTab::shell("s1".into(), "s1".into())],
            active: 0,
            next_shell_tab: 2,
        };
        let result = sanitize_workspace_tabs(state);
        assert_eq!(result.tabs[0].kind, TerminalKind::Agent);
        assert!(result.tabs.iter().any(|t| t.kind == TerminalKind::Shell));
    }

    #[test]
    fn sanitize_workspace_tabs_missing_shell_appends() {
        let state = WorkspaceTabsState {
            tabs: vec![TerminalTab::agent()],
            active: 0,
            next_shell_tab: 2,
        };
        let result = sanitize_workspace_tabs(state);
        assert_eq!(result.tabs.last().unwrap().kind, TerminalKind::Shell);
        assert!(result.tabs.iter().any(|t| t.kind == TerminalKind::Agent));
    }

    #[test]
    fn sanitize_workspace_tabs_active_clamped() {
        let state = WorkspaceTabsState {
            tabs: vec![
                TerminalTab::agent(),
                TerminalTab::shell("s1".into(), "s1".into()),
            ],
            active: 99,
            next_shell_tab: 2,
        };
        let result = sanitize_workspace_tabs(state);
        assert_eq!(result.active, 1); // clamped to last index
    }

    #[test]
    fn sanitize_workspace_tabs_next_shell_tab_raised() {
        let state = WorkspaceTabsState {
            tabs: vec![
                TerminalTab::agent(),
                TerminalTab::shell("s1".into(), "s1".into()),
            ],
            active: 0,
            next_shell_tab: 0,
        };
        let result = sanitize_workspace_tabs(state);
        assert_eq!(result.next_shell_tab, 2);
    }

    // ===== A3: tag_map =====

    #[test]
    fn tag_map_no_workspace_returns_empty() {
        let app = TuiApp::default();
        assert!(app.tag_map().is_empty());
    }

    #[test]
    fn tag_map_workspace_no_tags_returns_empty() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state()); // tags is empty
        assert!(app.tag_map().is_empty());
    }

    #[test]
    fn tag_map_workspace_with_tags_groups_by_hash() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        let mut git = make_git_state();
        git.tags = vec![
            protocol::TagInfo {
                name: "v1.0".into(),
                hash: "abc".into(),
                date: "1d".into(),
            },
            protocol::TagInfo {
                name: "v1.1".into(),
                hash: "abc".into(),
                date: "2d".into(),
            },
            protocol::TagInfo {
                name: "v2.0".into(),
                hash: "def".into(),
                date: "3d".into(),
            },
        ];
        app.set_workspace_git(id, git);
        let map = app.tag_map();
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("abc").unwrap().len(), 2);
        assert!(map.get("abc").unwrap().contains(&"v1.0".to_string()));
        assert!(map.get("abc").unwrap().contains(&"v1.1".to_string()));
        assert_eq!(map.get("def").unwrap(), &vec!["v2.0".to_string()]);
    }

    // ===== A4: total_log_items + log_item_at with tag filter =====

    #[test]
    fn total_log_items_tag_filter_only_tagged_commits() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        let mut git = make_git_state();
        // Tag only first commit ("abc"), second ("def") is untagged
        git.tags = vec![protocol::TagInfo {
            name: "v1.0".into(),
            hash: "abc".into(),
            date: "1d".into(),
        }];
        app.set_workspace_git(id, git);
        app.ws_tag_filter = true;
        // header(1) + 1 tagged commit = 2
        assert_eq!(app.total_log_items(), 2);
    }

    #[test]
    fn total_log_items_tag_filter_expanded_commit_includes_files() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        let mut git = make_git_state();
        git.tags = vec![protocol::TagInfo {
            name: "v1.0".into(),
            hash: "abc".into(),
            date: "1d".into(),
        }];
        app.set_workspace_git(id, git);
        app.ws_tag_filter = true;
        app.ws_expanded_commit = Some(0); // expand commit index 0 ("abc")
        app.commit_files_cache
            .insert("abc".into(), vec!["file1.rs".into(), "file2.rs".into()]);
        // header(1) + 1 tagged commit + 2 expanded files = 4
        assert_eq!(app.total_log_items(), 4);
    }

    #[test]
    fn log_item_at_tag_filter_skips_untagged() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        let mut git = make_git_state();
        // Tag only the second commit ("def"), first ("abc") is untagged
        git.tags = vec![protocol::TagInfo {
            name: "v2.0".into(),
            hash: "def".into(),
            date: "2d".into(),
        }];
        app.set_workspace_git(id, git);
        app.ws_tag_filter = true;
        // index 0 = header, index 1 = Commit(1) because Commit(0) is untagged and skipped
        assert_eq!(app.log_item_at(0), LogItem::UncommittedHeader);
        assert_eq!(app.log_item_at(1), LogItem::Commit(1));
    }

    // ===== A5: selected_commit_hash / selected_commit_file / selected_log_file =====

    #[test]
    fn selected_commit_hash_on_commit_item() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_selected_commit = 1; // Commit(0)
        assert_eq!(app.selected_commit_hash(), Some("abc".to_string()));
    }

    #[test]
    fn selected_commit_hash_on_uncommitted_header() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_selected_commit = 0; // UncommittedHeader
        assert_eq!(app.selected_commit_hash(), None);
    }

    #[test]
    fn selected_commit_file_on_commit_file_item() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_expanded_commit = Some(0);
        app.commit_files_cache.insert(
            "abc".into(),
            vec!["src/main.rs".into(), "Cargo.toml".into()],
        );
        // With expanded commit 0: header(0), Commit(0)(1), CommitFile(0,0)(2), CommitFile(0,1)(3), Commit(1)(4)
        app.ws_selected_commit = 2; // CommitFile(0, 0)
        let result = app.selected_commit_file();
        assert_eq!(result, Some(("abc".to_string(), "src/main.rs".to_string())));
    }

    #[test]
    fn selected_log_file_on_changed_file() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_uncommitted_expanded = true;
        app.ws_selected_commit = 1; // ChangedFile(0) = "a.rs"
        assert_eq!(app.selected_log_file(), Some("a.rs".to_string()));
    }

    // ===== A6: take_ssh_workspace_request =====

    #[test]
    fn take_ssh_workspace_request_valid() {
        let mut app = TuiApp::default();
        let mut input = SshWorkspaceInput::new();
        input.host = "myhost.com".to_string();
        input.path = "/home/user/project".to_string();
        input.user = "admin".to_string();
        app.ssh_workspace_input = Some(input);
        let result = app.take_ssh_workspace_request();
        assert!(result.is_some());
        let (name, path, target) = result.unwrap();
        assert!(name.contains("myhost.com"));
        assert_eq!(path, "/home/user/project");
        assert_eq!(target.host, "myhost.com");
        assert_eq!(target.user, Some("admin".to_string()));
    }

    #[test]
    fn take_ssh_workspace_request_empty_host() {
        let mut app = TuiApp::default();
        let mut input = SshWorkspaceInput::new();
        input.host = "".to_string();
        input.path = "/some/path".to_string();
        app.ssh_workspace_input = Some(input);
        assert!(app.take_ssh_workspace_request().is_none());
    }

    #[test]
    fn take_ssh_workspace_request_empty_path() {
        let mut app = TuiApp::default();
        let mut input = SshWorkspaceInput::new();
        input.host = "myhost.com".to_string();
        input.path = "".to_string();
        app.ssh_workspace_input = Some(input);
        assert!(app.take_ssh_workspace_request().is_none());
    }

    #[test]
    fn take_ssh_workspace_request_whitespace_user_becomes_none() {
        let mut app = TuiApp::default();
        let mut input = SshWorkspaceInput::new();
        input.host = "myhost.com".to_string();
        input.path = "/home/user/project".to_string();
        input.user = "   ".to_string();
        app.ssh_workspace_input = Some(input);
        let result = app.take_ssh_workspace_request();
        assert!(result.is_some());
        let (_, _, target) = result.unwrap();
        assert_eq!(target.user, None);
    }

    // ===== A7: take_add_workspace_request =====

    #[test]
    fn take_add_workspace_request_valid() {
        let mut app = TuiApp::default();
        app.dir_browser = Some(DirBrowserState {
            path_input: "/home/user/project".to_string(),
            entries: vec![],
            selected: 0,
            show_hidden: false,
            editing_path: false,
        });
        let result = app.take_add_workspace_request();
        assert!(result.is_some());
        let (name, path) = result.unwrap();
        assert_eq!(name, "project");
        assert_eq!(path, "/home/user/project");
    }

    #[test]
    fn take_add_workspace_request_empty_path() {
        let mut app = TuiApp::default();
        app.dir_browser = Some(DirBrowserState {
            path_input: "".to_string(),
            entries: vec![],
            selected: 0,
            show_hidden: false,
            editing_path: false,
        });
        assert!(app.take_add_workspace_request().is_none());
    }

    #[test]
    fn take_add_workspace_request_no_browser() {
        let mut app = TuiApp::default();
        assert!(app.take_add_workspace_request().is_none());
    }

    // ===== Delete branch =====

    #[test]
    fn begin_delete_branch_local() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_branch_sub_pane = BranchSubPane::Local;
        app.ws_selected_local_branch = 1; // "dev", not HEAD
        app.begin_delete_branch();
        assert_eq!(
            app.confirm_delete_branch,
            Some(DeleteBranchTarget::Local {
                branch: "dev".into()
            })
        );
    }

    #[test]
    fn begin_delete_branch_remote() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_branch_sub_pane = BranchSubPane::Remote;
        app.ws_selected_remote_branch = 0; // "origin/main"
        app.begin_delete_branch();
        assert_eq!(
            app.confirm_delete_branch,
            Some(DeleteBranchTarget::Remote {
                remote: "origin".into(),
                branch: "main".into(),
                full_name: "origin/main".into(),
            })
        );
    }

    #[test]
    fn begin_delete_branch_head_is_noop() {
        let mut app = app_with_workspaces(1);
        let id = app.workspaces[0].id;
        app.open_workspace(id);
        app.set_workspace_git(id, make_git_state());
        app.ws_branch_sub_pane = BranchSubPane::Local;
        app.ws_selected_local_branch = 0; // "main", HEAD
        app.begin_delete_branch();
        assert!(app.confirm_delete_branch.is_none());
    }

    #[test]
    fn cancel_delete_branch_clears_state() {
        let mut app = TuiApp::default();
        app.confirm_delete_branch = Some(DeleteBranchTarget::Local {
            branch: "foo".into(),
        });
        app.cancel_delete_branch();
        assert!(app.confirm_delete_branch.is_none());
    }

    #[test]
    fn take_delete_branch_extracts_and_clears() {
        let mut app = TuiApp::default();
        app.confirm_delete_branch = Some(DeleteBranchTarget::Local {
            branch: "foo".into(),
        });
        let taken = app.take_delete_branch();
        assert_eq!(
            taken,
            Some(DeleteBranchTarget::Local {
                branch: "foo".into()
            })
        );
        assert!(app.confirm_delete_branch.is_none());
    }
}
