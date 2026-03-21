use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use protocol::{
    AttentionLevel, BranchInfo, GitState, RemoteBranchInfo, Route, TerminalKind, WorkspaceId,
    WorkspaceSummary,
};
use ratatui::{
    style::{Color as TuiColor, Modifier, Style},
    text::{Line, Span},
};
use serde::{Deserialize, Serialize};

use crate::ui::widgets::tile_grid;

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
    /// Creates a zero-length selection anchored at the given position.
    pub fn at(col: u16, row: u16) -> Self {
        Self {
            anchor_col: col,
            anchor_row: row,
            end_col: col,
            end_row: row,
            confine: None,
        }
    }

    /// Creates a zero-length selection anchored at the given position, confined to `rect`.
    pub fn at_confined(col: u16, row: u16, rect: ratatui::layout::Rect) -> Self {
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
            self.end_col = self.end_col.clamp(r.x, r.right().saturating_sub(1));
            self.end_row = self.end_row.clamp(r.y, r.bottom().saturating_sub(1));
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
    HomeGrid,
    WsLog,
    WsBranches,
    WsDiff,
    WsTerminal,
    WsTerminalTabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogItem {
    UncommittedHeader,
    ChangedFile(usize),
    Commit(usize),
    CommitFile(usize, usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchSubPane {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteBranchTarget {
    Local { branch: String },
    Remote { remote: String, branch: String, full_name: String },
}

pub struct TuiApp {
    pub route: Route,
    pub focus: Focus,
    pub workspaces: Vec<WorkspaceSummary>,
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
    pub ws_selected_commit: usize,
    pub ws_selected_local_branch: usize,
    pub ws_selected_remote_branch: usize,
    pub ws_branch_sub_pane: BranchSubPane,
    pub ws_pending_select_head_branch: bool,
    pub ws_diff_scroll: u16,
    pub spinner_tick: u8,
    pub dir_browser: Option<DirBrowserState>,
    pub pending_delete_workspace: Option<WorkspaceId>,
    pub rename_workspace_input: Option<String>,
    pub rename_tab_input: Option<String>,
    pub git_action_message: Option<(String, Instant)>,
    pub commit_input: Option<String>,
    pub create_branch_input: Option<String>,
    pub settings: Settings,
    /// Workspace IDs with an in-flight git network operation (pull/push/fetch).
    /// Stores the start time so we can enforce a minimum spinner display duration.
    pub git_op_in_progress: HashMap<WorkspaceId, Instant>,
    /// Deferred git result waiting for spinner minimum duration to elapse.
    pub deferred_git_result: Option<(WorkspaceId, String)>,
    pub settings_open: bool,
    pub settings_selected: usize,
    pub mouse_selection: Option<MouseSelection>,
    /// Set on mouse-up to request clipboard copy on the next frame render.
    pub pending_copy_selection: Option<MouseSelection>,
    pub ssh_workspace_input: Option<SshWorkspaceInput>,
    pub ssh_history: Vec<SshHistoryEntry>,
    pub ssh_history_picker: Option<SshHistoryPicker>,
    pub confirm_discard_file: Option<String>,
    pub stash_input: Option<String>,
    pub confirm_stash_pull_pop: Option<WorkspaceId>,
    pub confirm_delete_branch: Option<DeleteBranchTarget>,
    pub ws_expanded_commit: Option<usize>,
    pub commit_files_cache: HashMap<String, Vec<String>>,
    pub ws_tag_filter: bool,
    /// Indices of expanded tiles on the home screen.
    pub home_expanded_tiles: HashSet<usize>,
    /// Vertical scroll offset for the home tile list.
    pub home_scroll_offset: u16,
    /// Cached grid height from last render, used for scroll calculations.
    pub last_grid_height: u16,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self {
            route: Route::Home,
            focus: Focus::HomeGrid,
            workspaces: Vec::new(),
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
            ws_selected_commit: 0,
            ws_selected_local_branch: 0,
            ws_selected_remote_branch: 0,
            ws_branch_sub_pane: BranchSubPane::Local,
            ws_pending_select_head_branch: false,
            ws_diff_scroll: 0,
            spinner_tick: 0,
            dir_browser: None,
            pending_delete_workspace: None,
            rename_workspace_input: None,
            rename_tab_input: None,
            git_action_message: None,
            commit_input: None,
            create_branch_input: None,
            git_op_in_progress: HashMap::new(),
            deferred_git_result: None,
            settings: load_settings(),
            settings_open: false,
            settings_selected: 0,
            mouse_selection: None,
            pending_copy_selection: None,
            ssh_workspace_input: None,
            ssh_history: load_ssh_history(),
            ssh_history_picker: None,
            confirm_discard_file: None,
            stash_input: None,
            confirm_stash_pull_pop: None,
            confirm_delete_branch: None,
            ws_expanded_commit: None,
            commit_files_cache: HashMap::new(),
            ws_tag_filter: false,
            home_expanded_tiles: HashSet::new(),
            home_scroll_offset: 0,
            last_grid_height: 0,
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
        self.reconcile_workspace_tab_state();
        self.ensure_home_selected_visible();
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

    pub fn open_workspace(&mut self, id: WorkspaceId) {
        self.persist_tabs_for_active_workspace();
        self.route = Route::Workspace { id };
        self.focus = Focus::WsTerminal;
        self.load_tabs_for_workspace(id);
    }

    pub fn go_home(&mut self) {
        self.persist_tabs_for_active_workspace();
        self.route = Route::Home;
        self.focus = Focus::HomeGrid;
    }

    pub fn terminal_fullscreen(&self) -> bool {
        self.active_tab().fullscreen
    }

    pub fn toggle_terminal_fullscreen(&mut self) {
        let idx = self.ws_active_tab.min(self.ws_tabs.len().saturating_sub(1));
        self.ws_tabs[idx].fullscreen = !self.ws_tabs[idx].fullscreen;
    }

    pub fn move_home_selection(&mut self, delta: isize) {
        let len = self.workspaces.len();
        if len == 0 {
            self.home_selected = 0;
            return;
        }
        let new_idx = (self.home_selected as isize + delta).clamp(0, (len - 1) as isize);
        self.home_selected = new_idx as usize;
        self.ensure_home_selected_visible();
    }

    pub fn toggle_home_expanded_tile(&mut self) {
        if self.workspaces.is_empty() {
            self.home_expanded_tiles.clear();
            return;
        }
        let idx = self.home_selected;
        if !self.home_expanded_tiles.remove(&idx) {
            self.home_expanded_tiles.insert(idx);
        }
        self.ensure_home_selected_visible();
    }

    pub fn set_home_selection(&mut self, index: usize) {
        if self.workspaces.is_empty() {
            self.home_selected = 0;
        } else {
            self.home_selected = index.min(self.workspaces.len() - 1);
        }
        self.ensure_home_selected_visible();
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

    /// Scroll the home tile list by `delta` rows (positive = down).
    pub fn scroll_home(&mut self, delta: i16) {
        let expanded_h = tile_grid::tile_h_expanded(self.settings.preview_lines);
        let total = tile_grid::total_height(
            self.workspaces.len(),
            &self.home_expanded_tiles,
            expanded_h,
        );
        let max_offset = total.saturating_sub(self.last_grid_height);
        let new_offset = (self.home_scroll_offset as i32 + delta as i32).clamp(0, max_offset as i32);
        self.home_scroll_offset = new_offset as u16;
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

    pub fn active_tab_passthrough(&self) -> bool {
        self.active_tab().passthrough
    }

    pub fn toggle_active_tab_passthrough(&mut self) {
        let idx = self.ws_active_tab.min(self.ws_tabs.len().saturating_sub(1));
        self.ws_tabs[idx].passthrough = !self.ws_tabs[idx].passthrough;
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

    pub fn begin_delete_workspace(&mut self) {
        self.pending_delete_workspace = self.selected_workspace_id();
    }

    pub fn cancel_delete_workspace(&mut self) {
        self.pending_delete_workspace = None;
    }

    pub fn is_confirming_delete(&self) -> bool {
        self.pending_delete_workspace.is_some()
    }

    pub fn take_delete_workspace(&mut self) -> Option<WorkspaceId> {
        self.pending_delete_workspace.take()
    }

    pub fn begin_rename_workspace_home(&mut self) {
        let Some(id) = self.selected_workspace_id() else {
            return;
        };
        self.rename_workspace_input = self
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.name.clone());
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

    pub fn append_terminal_bytes(&mut self, id: WorkspaceId, tab_id: &str, bytes: &[u8]) {
        let is_new_ws = !self.terminal_state.contains_key(&id);
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(WorkspaceTerminalState::new);
        let is_new_tab = !state.tabs.contains_key(tab_id);
        state.tab_mut(tab_id).append_bytes(bytes);
        if is_new_ws || is_new_tab {
            self.last_resize_sent.remove(&(id, tab_id.to_string()));
        }
    }

    pub fn reset_terminal(&mut self, id: WorkspaceId, tab_id: &str) {
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(WorkspaceTerminalState::new);
        state.tab_mut(tab_id).reset();
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
        let state = self
            .terminal_state
            .entry(id)
            .or_insert_with(WorkspaceTerminalState::new);
        let parser = &mut state.tab_mut(tab_id).parser;
        let current = parser.screen().scrollback() as isize;
        let next = (current + delta).max(0) as usize;
        parser.set_scrollback(next);
    }

    pub fn should_send_resize(
        &mut self,
        id: WorkspaceId,
        tab_id: &str,
        cols: u16,
        rows: u16,
    ) -> bool {
        let key = (id, tab_id.to_string());
        let next = (cols.max(1), rows.max(1));
        if self.last_resize_sent.get(&key).copied() == Some(next) {
            return false;
        }
        self.last_resize_sent.insert(key, next);
        true
    }

    pub fn terminal_lines(&self, id: WorkspaceId, tab_id: &str) -> Vec<Line<'static>> {
        let Some(state) = self.terminal_state.get(&id) else {
            return vec![Line::from("No terminal output yet.")];
        };
        let Some(tab) = state.tabs.get(tab_id) else {
            return vec![Line::from("No terminal output yet.")];
        };
        let parser = &tab.parser;
        let screen = parser.screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let show_cursor = !screen.hide_cursor();
        let (rows, cols) = screen.size();
        let mut lines = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let mut spans = Vec::with_capacity(cols as usize);
            for c in 0..cols {
                let Some(cell) = screen.cell(r, c) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = Style::default();
                let fg = map_color(cell.fgcolor());
                let bg = map_color(cell.bgcolor());
                style = style.fg(fg).bg(bg);
                if cell.bold() {
                    style = style.add_modifier(Modifier::BOLD);
                }
                if cell.italic() {
                    style = style.add_modifier(Modifier::ITALIC);
                }
                if cell.underline() {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                if cell.inverse() {
                    // When colors are Reset (terminal default), we must use explicit
                    // colors so that swapping them produces a visible inversion.
                    let inv_fg = if bg == TuiColor::Reset {
                        TuiColor::Black
                    } else {
                        bg
                    };
                    let inv_bg = if fg == TuiColor::Reset {
                        TuiColor::White
                    } else {
                        fg
                    };
                    style = style.fg(inv_fg).bg(inv_bg);
                }
                if show_cursor && r == cursor_row && c == cursor_col {
                    let cur_fg = if fg == TuiColor::Reset {
                        TuiColor::Black
                    } else {
                        bg
                    };
                    let cur_bg = if fg == TuiColor::Reset {
                        TuiColor::White
                    } else {
                        fg
                    };
                    style = style.fg(cur_fg).bg(cur_bg);
                }
                let text = if cell.has_contents() {
                    cell.contents()
                } else {
                    " ".to_string()
                };
                spans.push(Span::styled(text, style));
            }
            lines.push(Line::from(spans));
        }
        lines
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
        let screen = tab.parser.screen();
        let (rows, cols) = screen.size();
        let use_cols = cols.min(max_cols);
        let scrollback_count = screen.scrollback_count();

        let needed = num_lines as usize;
        let mut collected: Vec<Line<'static>> = Vec::new();

        // Phase 1: visible screen rows, bottom-to-top (most recent first)
        for r in (0..rows).rev() {
            if collected.len() >= needed {
                break;
            }
            let mut has_content = false;
            for c in 0..use_cols {
                if let Some(cell) = screen.cell(r, c) {
                    if cell.has_contents() && !cell.contents().trim().is_empty() {
                        has_content = true;
                        break;
                    }
                }
            }
            if has_content {
                collected.push(self.styled_screen_row(screen, r, use_cols));
            }
        }

        // Phase 2: scrollback rows (offset 0 = most recently scrolled off)
        if collected.len() < needed {
            for offset in 0..scrollback_count {
                if collected.len() >= needed {
                    break;
                }
                let mut has_content = false;
                for c in 0..use_cols {
                    if let Some(cell) = screen.scrollback_cell(offset, c) {
                        if cell.has_contents() && !cell.contents().trim().is_empty() {
                            has_content = true;
                            break;
                        }
                    }
                }
                if has_content {
                    collected.push(self.styled_scrollback_row(screen, offset, use_cols));
                }
            }
        }

        // Both phases collected most-recent-first; reverse to chronological
        collected.truncate(needed);
        collected.reverse();
        collected
    }

    fn styled_scrollback_row(
        &self,
        screen: &vt100::Screen,
        offset: usize,
        use_cols: u16,
    ) -> Line<'static> {
        let mut spans = Vec::new();
        for c in 0..use_cols {
            let Some(cell) = screen.scrollback_cell(offset, c) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            spans.push(styled_span_from_cell(cell));
        }
        Line::from(spans)
    }

    fn styled_screen_row(
        &self,
        screen: &vt100::Screen,
        row: u16,
        use_cols: u16,
    ) -> Line<'static> {
        let mut spans = Vec::new();
        for c in 0..use_cols {
            let Some(cell) = screen.cell(row, c) else {
                continue;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            spans.push(styled_span_from_cell(cell));
        }
        Line::from(spans)
    }

    pub fn terminal_mouse_state(
        &self,
        id: WorkspaceId,
        tab_id: &str,
    ) -> Option<(vt100::MouseProtocolMode, vt100::MouseProtocolEncoding, bool)> {
        let state = self.terminal_state.get(&id)?;
        let tab = state.tabs.get(tab_id)?;
        let screen = tab.parser.screen();
        Some((
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
            screen.alternate_screen(),
        ))
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

    pub fn total_log_items(&self) -> usize {
        let Some(id) = self.active_workspace_id() else {
            return 1; // just the header
        };
        let Some(git) = self.workspace_git.get(&id) else {
            return 1;
        };
        let file_count = if self.ws_uncommitted_expanded && !git.changed.is_empty() {
            git.changed.len()
        } else {
            0
        };
        if self.ws_tag_filter {
            let tag_map = self.tag_map();
            let mut count = 1 + file_count; // header + uncommitted files
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
            1 + file_count + git.recent_commits.len() + expanded_commit_files
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
        let file_count = if self.ws_uncommitted_expanded && !git.changed.is_empty() {
            git.changed.len()
        } else {
            0
        };
        let mut offset = index - 1; // subtract header
        if offset < file_count {
            return LogItem::ChangedFile(offset);
        }
        offset -= file_count;

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
            LogItem::UncommittedHeader | LogItem::ChangedFile(_)
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

    pub fn is_confirming_discard(&self) -> bool {
        self.confirm_discard_file.is_some()
    }

    pub fn begin_discard(&mut self) {
        if let Some(file) = self.selected_log_file() {
            self.confirm_discard_file = Some(file);
        }
    }

    pub fn cancel_discard(&mut self) {
        self.confirm_discard_file = None;
    }

    pub fn take_discard_file(&mut self) -> Option<String> {
        self.confirm_discard_file.take()
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
    }

    pub fn close_settings(&mut self) {
        self.settings_open = false;
    }

    pub fn toggle_selected_setting(&mut self) {
        match self.settings_selected {
            0 => self.settings.attention_notifications = !self.settings.attention_notifications,
            1 => {} // preview_lines uses adjust, not toggle
            _ => {}
        }
        let _ = save_settings(&self.settings);
    }

    pub fn adjust_selected_setting(&mut self, delta: i16) {
        match self.settings_selected {
            1 => {
                let val = self.settings.preview_lines as i16 + delta;
                self.settings.preview_lines = val.clamp(4, 30) as u16;
            }
            _ => {}
        }
        let _ = save_settings(&self.settings);
    }

    pub fn settings_count(&self) -> usize {
        2
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
        self.workspace_tabs.insert(id, state.clone());
        if let Some(path) = self.workspace_path(id) {
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
                    passthrough: false,
                    fullscreen: false,
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

pub struct WorkspaceTerminalState {
    pub tabs: HashMap<String, TerminalBufferState>,
}

impl WorkspaceTerminalState {
    fn new() -> Self {
        let mut tabs = HashMap::new();
        tabs.insert("agent".to_string(), TerminalBufferState::new());
        tabs.insert("shell".to_string(), TerminalBufferState::new());
        Self { tabs }
    }

    fn tab_mut(&mut self, tab_id: &str) -> &mut TerminalBufferState {
        self.tabs
            .entry(tab_id.to_string())
            .or_insert_with(TerminalBufferState::new)
    }
}

const MAX_TERMINAL_HISTORY_BYTES: usize = 2 * 1024 * 1024;

pub struct TerminalBufferState {
    pub parser: vt100::Parser,
    pub history: Vec<u8>,
}

impl TerminalBufferState {
    fn new() -> Self {
        Self {
            parser: make_parser(),
            history: Vec::new(),
        }
    }

    fn append_bytes(&mut self, bytes: &[u8]) {
        self.history.extend_from_slice(bytes);
        if self.history.len() > MAX_TERMINAL_HISTORY_BYTES {
            let trim = self.history.len() - MAX_TERMINAL_HISTORY_BYTES;
            self.history.drain(..trim);
        }
        self.parser.process(bytes);
    }

    fn reset(&mut self) {
        self.parser = make_parser();
        self.history.clear();
    }

    fn rebuild_for_size(&mut self, cols: u16, rows: u16) {
        let mut parser = vt100::Parser::new(rows.max(1), cols.max(1), 8000);
        parser.process(&self.history);
        self.parser = parser;
    }
}

#[derive(Clone)]
pub struct TerminalTab {
    pub id: String,
    pub label: String,
    pub kind: TerminalKind,
    /// When true, Esc and Tab are forwarded to the terminal instead of being intercepted.
    pub passthrough: bool,
    /// When true, git panes are hidden and the terminal fills the workspace.
    pub fullscreen: bool,
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
            passthrough: false,
            fullscreen: false,
        }
    }

    fn shell(id: String, label: String) -> Self {
        Self {
            id,
            label,
            kind: TerminalKind::Shell,
            passthrough: false,
            fullscreen: false,
        }
    }
}

fn make_parser() -> vt100::Parser {
    vt100::Parser::new(24, 120, 8000)
}

fn map_color(color: vt100::Color) -> TuiColor {
    match color {
        vt100::Color::Default => TuiColor::Reset,
        vt100::Color::Idx(i) => TuiColor::Indexed(i),
        vt100::Color::Rgb(r, g, b) => TuiColor::Rgb(r, g, b),
    }
}

/// Converts a vt100 cell into a styled ratatui Span (no cursor rendering).
fn styled_span_from_cell(cell: &vt100::Cell) -> Span<'static> {
    let fg = map_color(cell.fgcolor());
    let bg = map_color(cell.bgcolor());
    let mut style = Style::default().fg(fg).bg(bg);
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    let text = if cell.has_contents() {
        cell.contents()
    } else {
        " ".to_string()
    };
    Span::styled(text, style)
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
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return None;
    };
    Some(base.join("anvl").join("tui_tabs.json"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub attention_notifications: bool,
    #[serde(default = "default_preview_lines")]
    pub preview_lines: u16,
}

fn default_true() -> bool {
    true
}

fn default_preview_lines() -> u16 {
    12
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            attention_notifications: true,
            preview_lines: 12,
        }
    }
}

fn settings_persist_path() -> Option<PathBuf> {
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return None;
    };
    Some(base.join("anvl").join("settings.json"))
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
    let base = if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config")
    } else {
        return None;
    };
    Some(base.join("anvl").join("ssh_history.json"))
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
            shell_running: false,
            last_activity_unix_ms: 0,
            ssh_host: None,
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

    fn app_with_workspaces(n: usize) -> TuiApp {
        let mut app = TuiApp::default();
        let ws: Vec<_> = (0..n).map(|i| make_ws(&format!("ws{i}"))).collect();
        app.set_workspaces(ws);
        app
    }

    // ===== Navigation =====

    #[test]
    fn move_home_selection_down() {
        let mut app = app_with_workspaces(6);
        app.home_selected = 0;
        app.move_home_selection(1);
        assert_eq!(app.home_selected, 1);
    }

    #[test]
    fn move_home_selection_up_clamps() {
        let mut app = app_with_workspaces(6);
        app.home_selected = 0;
        app.move_home_selection(-1);
        assert_eq!(app.home_selected, 0);
    }

    #[test]
    fn move_home_selection_down_clamps_to_last() {
        let mut app = app_with_workspaces(4);
        app.home_selected = 3;
        app.move_home_selection(1);
        assert_eq!(app.home_selected, 3);
    }

    #[test]
    fn move_home_selection_empty() {
        let mut app = TuiApp::default();
        app.move_home_selection(1);
        assert_eq!(app.home_selected, 0);
    }

    #[test]
    fn set_home_selection_clamps() {
        let mut app = app_with_workspaces(3);
        app.set_home_selection(10);
        assert_eq!(app.home_selected, 2);
    }

    #[test]
    fn set_home_selection_empty() {
        let mut app = TuiApp::default();
        app.set_home_selection(5);
        assert_eq!(app.home_selected, 0);
    }

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
        assert_eq!(app.focus, Focus::HomeGrid);
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
            passthrough: false,
            fullscreen: false,
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
    fn toggle_passthrough() {
        let mut app = TuiApp::default();
        app.ws_active_tab = 1; // shell tab
        assert!(!app.active_tab_passthrough());
        app.toggle_active_tab_passthrough();
        assert!(app.active_tab_passthrough());
        app.toggle_active_tab_passthrough();
        assert!(!app.active_tab_passthrough());
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
        app.begin_delete_workspace();
        assert!(app.is_confirming_delete());
        let id = app.take_delete_workspace();
        assert!(id.is_some());
        assert!(!app.is_confirming_delete());
    }

    #[test]
    fn cancel_delete_workspace() {
        let mut app = app_with_workspaces(2);
        app.begin_delete_workspace();
        app.cancel_delete_workspace();
        assert!(!app.is_confirming_delete());
    }

    #[test]
    fn rename_workspace_flow() {
        let mut app = app_with_workspaces(1);
        app.begin_rename_workspace_home();
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
    fn stash_modal() {
        let mut app = TuiApp::default();
        assert!(!app.is_stashing());
        app.stash_input = Some("msg".into());
        assert!(app.is_stashing());
    }

    // ===== Pure helpers =====

    #[test]
    fn mouse_selection_at() {
        let sel = MouseSelection::at(5, 10);
        assert_eq!(sel.anchor_col, 5);
        assert_eq!(sel.anchor_row, 10);
        assert!(sel.is_empty());
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
        let mut sel = MouseSelection::at(0, 0);
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
