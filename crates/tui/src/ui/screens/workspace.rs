use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};

use crate::app::TuiApp;
use crate::ui::footer;
use crate::ui::widgets::tile_grid::ORANGE;
use crate::ui::widgets::workspace_bar;
use protocol::{AttentionLevel, Route};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceHit {
    WorkspaceBarPill(usize),
    TerminalTab(usize),
    TerminalPane,
    ScrollToBottom,
    LogList(usize),
    BranchesPane(usize),
    DiffPane,
}

#[derive(Debug, Clone, Copy)]
struct WorkspaceLayout {
    workspace_bar: Rect,
    terminal_pane: Rect,
    git_log: Rect,
    git_branches: Rect,
    git_diff: Rect,
    footer: Rect,
}

fn layout(area: Rect, focus: crate::app::Focus, terminal_fullscreen: bool) -> WorkspaceLayout {
    // Top-level: workspace bar + body + footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(area);

    if terminal_fullscreen {
        let zero = Rect::new(0, 0, 0, 0);
        return WorkspaceLayout {
            workspace_bar: chunks[0],
            terminal_pane: chunks[1],
            git_log: zero,
            git_branches: zero,
            git_diff: zero,
            footer: chunks[2],
        };
    }

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints(match focus {
            crate::app::Focus::WsBar
            | crate::app::Focus::WsTerminal
            | crate::app::Focus::WsTerminalTabs => {
                [Constraint::Percentage(72), Constraint::Percentage(28)]
            }
            crate::app::Focus::WsLog
            | crate::app::Focus::WsBranches
            | crate::app::Focus::WsDiff => [Constraint::Percentage(35), Constraint::Percentage(65)],
            _ => [Constraint::Percentage(55), Constraint::Percentage(45)],
        })
        .split(chunks[1]);
    let git_area = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(body[1]);

    // Split left pane into git log (top) + branches (bottom)
    let left_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(git_area[0]);

    WorkspaceLayout {
        workspace_bar: chunks[0],
        terminal_pane: body[0],
        git_log: left_split[0],
        git_branches: left_split[1],
        git_diff: git_area[1],
        footer: chunks[2],
    }
}

/// Returns the standard focused/unfocused border style used by all non-attention panes.
fn standard_border_style(focused: bool) -> (Style, BorderType) {
    if focused {
        (
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD),
            BorderType::Thick,
        )
    } else {
        (
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::DIM),
            BorderType::Plain,
        )
    }
}

/// Computes the border style for the terminal pane, accounting for attention level.
///
/// When the workspace has an active attention state (`NeedsInput` or `Error`) and
/// `flash_on` is true, the border flashes in the corresponding colour.  Otherwise
/// the standard focused / unfocused styling is used.
pub fn pane_border_style(
    focused: bool,
    attention: AttentionLevel,
    flash_on: bool,
    command_mode: bool,
) -> (Style, BorderType) {
    if command_mode {
        return (
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            BorderType::Thick,
        );
    }
    match attention {
        AttentionLevel::NeedsInput if flash_on => (
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
            BorderType::Thick,
        ),
        AttentionLevel::Error if flash_on => (
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            BorderType::Thick,
        ),
        _ => standard_border_style(focused),
    }
}

/// Builds the title `Line` for the terminal pane, with an optional attention badge.
#[cfg(test)]
pub fn build_terminal_title_line(
    attention: AttentionLevel,
    flash_on: bool,
    command_mode: bool,
) -> Line<'static> {
    let raw_badge = if command_mode {
        Some(Span::styled(
            " [command]",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ))
    } else {
        None
    };

    let mut spans = match attention {
        AttentionLevel::NeedsInput => {
            let badge_style = if flash_on {
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ORANGE)
            };
            vec![Span::raw("Terminal "), Span::styled("⚠ input", badge_style)]
        }
        AttentionLevel::Error => {
            let badge_style = if flash_on {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Red)
            };
            vec![Span::raw("Terminal "), Span::styled("✖ error", badge_style)]
        }
        _ => vec![Span::raw("Terminal")],
    };
    if let Some(badge) = raw_badge {
        spans.push(badge);
    }
    Line::from(spans)
}

/// Builds the workspace info string used on the border line and for hit-testing.
fn ws_info_string(app: &TuiApp) -> String {
    let ws_id = match app.route {
        Route::Workspace { id } => Some(id),
        _ => None,
    };
    ws_id
        .and_then(|id| app.workspaces.iter().find(|w| w.id == id))
        .map(|w| {
            let git = ws_id.and_then(|id| app.workspace_git.get(&id));
            let branch = git.and_then(|g| g.branch.as_deref()).unwrap_or("-");
            let ab = match (git.and_then(|g| g.ahead), git.and_then(|g| g.behind)) {
                (Some(a), Some(b)) if a == 0 && b == 0 => " =".to_string(),
                (Some(a), Some(b)) => {
                    let mut s = String::new();
                    if a > 0 {
                        s.push_str(&format!(" ↑{a}"));
                    }
                    if b > 0 {
                        s.push_str(&format!(" ↓{b}"));
                    }
                    s
                }
                _ => String::new(),
            };
            if let Some(rename) = &app.rename_workspace_input {
                format!("Rename: {rename}")
            } else {
                format!("{} · {}  ◈{}{}", w.name, branch, w.dirty_files, ab)
            }
        })
        .unwrap_or_else(|| "Workspace".to_string())
}

const MAX_TAB_WIDTH: u16 = 24;

/// Computes the x-range for each tab on the terminal pane's top border.
/// Tabs are sized to fit their label, capped at `MAX_TAB_WIDTH`.
/// Returns a vec of `(start_x, end_x)` for each tab.
fn compute_tab_ranges(
    pane: &Rect,
    ws_info_width: u16,
    tab_label_widths: &[u16],
) -> Vec<(u16, u16)> {
    let inner_left = pane.x + 1;
    let inner_right = pane.right().saturating_sub(1);
    let tab_area_start = inner_left + ws_info_width + 2; // +2 for spacing after ws info

    let mut ranges = Vec::new();
    let mut x = tab_area_start;
    for (i, &label_w) in tab_label_widths.iter().enumerate() {
        if i > 0 {
            x += 1; // 1-char gap between tabs
        }
        let width = label_w.min(MAX_TAB_WIDTH);
        let end = (x + width).min(inner_right);
        if x >= inner_right {
            break;
        }
        ranges.push((x, end));
        x = end;
    }
    ranges
}

fn status_marker(c: char) -> char {
    match c {
        ' ' => '-',
        other => other,
    }
}

fn changed_file_line(f: &protocol::ChangedFile) -> Line<'static> {
    let idx = f.index_status;
    let wt = f.worktree_status;
    let idx_style = match idx {
        '?' => Style::default().fg(Color::Red),
        ' ' => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::Green),
    };
    let wt_style = match wt {
        '?' => Style::default().fg(Color::Red),
        ' ' => Style::default().fg(Color::DarkGray),
        _ => Style::default().fg(Color::Yellow),
    };

    Line::from(vec![
        Span::raw("  "),
        Span::styled("I:", Style::default().fg(Color::DarkGray)),
        Span::styled(status_marker(idx).to_string(), idx_style),
        Span::raw(" "),
        Span::styled("W:", Style::default().fg(Color::DarkGray)),
        Span::styled(status_marker(wt).to_string(), wt_style),
        Span::raw(format!(" {}", f.path)),
    ])
}

const BRAILLE_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn spinner_frame(tick: u8) -> &'static str {
    BRAILLE_SPINNER[(tick as usize) % BRAILLE_SPINNER.len()]
}

pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let l = layout(area, app.focus, app.terminal_fullscreen());

    let ws_id = match app.route {
        Route::Workspace { id } => Some(id),
        _ => None,
    };
    let attention = app.effective_attention(
        ws_id
            .and_then(|id| app.workspaces.iter().find(|w| w.id == id))
            .map(|w| w.attention)
            .unwrap_or(AttentionLevel::None),
    );

    // --- Workspace status bar ---
    let bar_selected = if matches!(app.focus, crate::app::Focus::WsBar) {
        Some(app.ws_bar_selected)
    } else {
        None
    };
    let bar_line = workspace_bar::build_workspace_bar_line(
        &app.workspaces,
        ws_id,
        app.spinner_tick % 2 == 0,
        app.settings.attention_notifications,
        l.workspace_bar.width,
        bar_selected,
    );
    frame.render_widget(Paragraph::new(bar_line), l.workspace_bar);

    // Build workspace info string for the border line
    let ws_info = ws_info_string(app);

    if !app.terminal_fullscreen() {
        // --- Git Log (merged uncommitted + commits + tags) ---
        let changed = ws_id
            .and_then(|id| app.workspace_git.get(&id))
            .map(|g| g.changed.clone())
            .unwrap_or_default();
        let commits = ws_id
            .and_then(|id| app.workspace_git.get(&id))
            .map(|g| g.recent_commits.clone())
            .unwrap_or_default();
        let tags = ws_id
            .and_then(|id| app.workspace_git.get(&id))
            .map(|g| g.tags.clone())
            .unwrap_or_default();

        let total = app.total_log_items();
        let mut log_list_state = ListState::default();
        if total > 0 {
            log_list_state.select(Some(app.ws_selected_commit.min(total - 1)));
        }

        let mut log_items: Vec<ListItem> = Vec::new();

        // Uncommitted header
        {
            let arrow = if app.ws_uncommitted_expanded {
                "▼"
            } else {
                "▶"
            };
            let count = changed.len();
            let header_style = if count > 0 {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            log_items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("{arrow} Uncommitted Changes"), header_style),
                Span::styled(format!(" ({count})"), Style::default().fg(Color::DarkGray)),
            ])));
        }

        // Expanded files
        if app.ws_uncommitted_expanded && !changed.is_empty() {
            for f in &changed {
                log_items.push(ListItem::new(changed_file_line(f)));
            }
        }

        // Build tag lookup: commit hash → list of tag names
        let tag_map = {
            let mut m: std::collections::HashMap<String, Vec<String>> =
                std::collections::HashMap::new();
            for t in &tags {
                m.entry(t.hash.clone()).or_default().push(t.name.clone());
            }
            m
        };

        // Commits
        for (i, c) in commits.iter().enumerate() {
            // When tag filter is active, skip commits without tags
            if app.ws_tag_filter && !tag_map.contains_key(&c.hash) {
                continue;
            }
            let is_expanded = app.ws_expanded_commit == Some(i);
            let arrow = if is_expanded { "▼ " } else { "▶ " };
            let mut spans = vec![Span::styled(
                format!("{arrow}{} ", c.hash),
                Style::default().fg(Color::Yellow),
            )];
            // Inline tag badges right after hash
            if let Some(tag_names) = tag_map.get(&c.hash) {
                for name in tag_names {
                    spans.push(Span::styled(
                        format!("[{name}] "),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }
            spans.push(Span::raw(&c.message));
            spans.push(Span::styled(
                format!(" ({}, {})", c.author, c.date),
                Style::default().fg(Color::DarkGray),
            ));
            log_items.push(ListItem::new(Line::from(spans)));
            if is_expanded {
                if let Some(files) = app.commit_files_cache.get(&c.hash) {
                    for f in files {
                        log_items.push(ListItem::new(Line::from(Span::raw(format!("    {f}")))));
                    }
                }
            }
        }

        let (log_style, log_border_type) =
            standard_border_style(app.focus == crate::app::Focus::WsLog);
        let commit_list = List::new(log_items)
            .block(
                Block::default()
                    .title("Git Log")
                    .borders(Borders::ALL)
                    .border_style(log_style)
                    .border_type(log_border_type),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
        frame.render_stateful_widget(commit_list, l.git_log, &mut log_list_state);

        // --- Branches Pane ---
        {
            let branch_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(l.git_branches);

            let is_branches_focused = app.focus == crate::app::Focus::WsBranches;
            let local_active = matches!(app.ws_branch_sub_pane, crate::app::BranchSubPane::Local);
            let remote_active = matches!(app.ws_branch_sub_pane, crate::app::BranchSubPane::Remote);

            // Local branches
            let local_branches = ws_id
                .and_then(|id| app.workspace_git.get(&id))
                .map(|g| g.local_branches.clone())
                .unwrap_or_default();
            let mut local_list_state = ListState::default();
            if !local_branches.is_empty() {
                local_list_state.select(Some(
                    app.ws_selected_local_branch.min(local_branches.len() - 1),
                ));
            }
            let local_items = local_branches
                .iter()
                .map(|b| {
                    let mut spans = Vec::new();
                    if b.is_head {
                        spans.push(Span::styled(
                            "* ",
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::raw("  "));
                    }
                    let name_style = if b.is_head {
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                    spans.push(Span::styled(b.name.clone(), name_style));
                    let git_op_active = b.is_head
                        && ws_id
                            .map(|id| app.is_git_op_in_progress(id))
                            .unwrap_or(false);
                    if git_op_active {
                        // Re-style all existing spans yellow during git ops
                        let yellow_bold = Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD);
                        for s in &mut spans {
                            *s = Span::styled(s.content.clone(), yellow_bold);
                        }
                        spans.push(Span::styled(
                            format!(" {}", spinner_frame(app.spinner_tick)),
                            yellow_bold,
                        ));
                    }
                    // Ahead/behind indicators
                    match (b.ahead, b.behind) {
                        (Some(a), Some(b_count)) if a == 0 && b_count == 0 => {
                            spans.push(Span::styled(
                                " =",
                                Style::default().add_modifier(Modifier::DIM),
                            ));
                        }
                        (ahead, behind) => {
                            if let Some(a) = ahead {
                                if a > 0 {
                                    spans.push(Span::styled(
                                        format!(" \u{2191}{a}"),
                                        Style::default().fg(Color::Green),
                                    ));
                                }
                            }
                            if let Some(b_count) = behind {
                                if b_count > 0 {
                                    spans.push(Span::styled(
                                        format!(" \u{2193}{b_count}"),
                                        Style::default().fg(Color::Red),
                                    ));
                                }
                            }
                        }
                    }
                    ListItem::new(Line::from(spans))
                })
                .collect::<Vec<_>>();

            let local_title = if is_branches_focused && local_active {
                "Local [*]"
            } else {
                "Local"
            };
            let (local_style, local_border_type) = if is_branches_focused && local_active {
                (
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                    BorderType::Thick,
                )
            } else {
                (
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::DIM),
                    BorderType::Plain,
                )
            };
            let local_list = List::new(local_items)
                .block(
                    Block::default()
                        .title(local_title)
                        .borders(Borders::ALL)
                        .border_style(local_style)
                        .border_type(local_border_type),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                );
            frame.render_stateful_widget(local_list, branch_split[0], &mut local_list_state);

            // Remote branches
            let remote_branches = ws_id
                .and_then(|id| app.workspace_git.get(&id))
                .map(|g| g.remote_branches.clone())
                .unwrap_or_default();
            let mut remote_list_state = ListState::default();
            if !remote_branches.is_empty() {
                remote_list_state.select(Some(
                    app.ws_selected_remote_branch.min(remote_branches.len() - 1),
                ));
            }
            let remote_items = remote_branches
                .iter()
                .map(|b| ListItem::new(Line::from(Span::raw(format!("  {}", b.full_name)))))
                .collect::<Vec<_>>();

            let remote_title = if is_branches_focused && remote_active {
                "Remote [*]"
            } else {
                "Remote"
            };
            let (remote_style, remote_border_type) = if is_branches_focused && remote_active {
                (
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                    BorderType::Thick,
                )
            } else {
                (
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::DIM),
                    BorderType::Plain,
                )
            };
            let remote_list = List::new(remote_items)
                .block(
                    Block::default()
                        .title(remote_title)
                        .borders(Borders::ALL)
                        .border_style(remote_style)
                        .border_type(remote_border_type),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                );
            frame.render_stateful_widget(remote_list, branch_split[1], &mut remote_list_state);
        }

        // --- Diff Pane ---
        // Validate that the cached diff matches the current selection — otherwise an
        // out-of-order async completion or a non-file selection would display stale data.
        let expected_key: Option<String> = match app.log_item_at(app.ws_selected_commit) {
            crate::app::LogItem::ChangedFile(_) => app.selected_log_file(),
            crate::app::LogItem::CommitFile(_, _) => app
                .selected_commit_file()
                .map(|(hash, file)| format!("{hash}:{file}")),
            _ => None,
        };
        let diff_text = match (ws_id, expected_key.as_deref()) {
            (Some(id), Some(key)) => match app.workspace_diff.get(&id) {
                Some((cached_file, diff)) if cached_file == key => diff.clone(),
                _ => "Loading diff…".to_string(),
            },
            _ => "Select a file to view diff.".to_string(),
        };
        let diff_lines = diff_text
            .lines()
            .map(|line| {
                if line.starts_with('+') {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Green),
                    ))
                } else if line.starts_with('-') {
                    Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(Color::Red),
                    ))
                } else {
                    Line::from(Span::raw(line.to_string()))
                }
            })
            .collect::<Vec<_>>();
        let (diff_style, diff_border_type) =
            standard_border_style(app.focus == crate::app::Focus::WsDiff);
        frame.render_widget(
            Paragraph::new(diff_lines)
                .block(
                    Block::default()
                        .title("Diff")
                        .borders(Borders::ALL)
                        .border_style(diff_style)
                        .border_type(diff_border_type),
                )
                .scroll((app.ws_diff_scroll, 0))
                .wrap(Wrap { trim: false }),
            l.git_diff,
        );
    } // end if !terminal_fullscreen

    // --- Terminal Pane (with browser-style tab notch) ---
    let terminal_focused = app.focus == crate::app::Focus::WsTerminal;
    let tabs_focused = app.focus == crate::app::Focus::WsTerminalTabs;
    let terminal_lines = ws_id
        .map(|id| app.terminal_lines(id, &app.active_tab_id()))
        .unwrap_or_else(|| vec![Line::from("No terminal output yet.")]);
    let (term_style, term_border_type) = pane_border_style(
        terminal_focused,
        attention,
        app.spinner_tick % 2 == 0,
        app.terminal_command_mode(),
    );

    // Render terminal pane with Borders::ALL — we'll overwrite the top border row
    frame.render_widget(Clear, l.terminal_pane);
    frame.render_widget(
        Paragraph::new(terminal_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(term_style)
                .border_type(term_border_type),
        ),
        l.terminal_pane,
    );

    // --- Inline tab rendering on the terminal pane's top border ---

    let flash_on = app.spinner_tick % 2 == 0;
    let pane = l.terminal_pane;
    let border_y = pane.y;
    let buf = frame.buffer_mut();

    // Build tab labels
    let mut tab_labels: Vec<String> = Vec::new();
    for (i, tab) in app.ws_tabs.iter().enumerate() {
        let label = if i == app.ws_active_tab {
            app.rename_tab_input
                .as_ref()
                .cloned()
                .unwrap_or_else(|| tab.label.clone())
        } else {
            tab.label.clone()
        };
        tab_labels.push(label);
    }

    // Determine usable width: inside the left/right borders of terminal pane
    let inner_left = pane.x + 1;
    let inner_right = pane.right().saturating_sub(1);
    let inner_width = inner_right.saturating_sub(inner_left);

    // Workspace info goes on the left of the border line
    let ws_info_display: String = if ws_info.len() as u16 > inner_width / 2 {
        ws_info.chars().take((inner_width / 2) as usize).collect()
    } else {
        ws_info.clone()
    };
    let ws_info_width = ws_info_display.len() as u16;

    // Compute tab label widths and positions
    let tab_label_widths: Vec<u16> = tab_labels
        .iter()
        .enumerate()
        .map(|(i, lbl)| format!(" {}: {} ", i + 1, lbl).len() as u16)
        .collect();
    let tab_ranges = compute_tab_ranges(&pane, ws_info_width, &tab_label_widths);

    let active = app.ws_active_tab;

    // Write workspace info on the left portion of the border line
    let ws_info_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    for (i, ch) in ws_info_display.chars().enumerate() {
        let x = inner_left + 1 + i as u16; // +1 for a space after the border
        if x < inner_right && x < buf.area().width && border_y < buf.area().height {
            buf[(x, border_y)].set_char(ch).set_style(ws_info_style);
        }
    }

    // Draw tabs inline on the border line: active = pill/badge, inactive = gray label
    for (i, _tab) in app.ws_tabs.iter().enumerate() {
        if i >= tab_ranges.len() {
            break;
        }
        let (tab_start, tab_end) = tab_ranges[i];

        if i == active {
            // Active tab: pill/badge with background color
            let is_agent = matches!(
                app.ws_tabs.get(active).map(|t| &t.kind),
                Some(protocol::TerminalKind::Agent)
            );
            let active_style = if is_agent
                && matches!(
                    attention,
                    AttentionLevel::NeedsInput | AttentionLevel::Error
                )
                && flash_on
            {
                let color = match attention {
                    AttentionLevel::Error => Color::Red,
                    _ => ORANGE,
                };
                Style::default()
                    .bg(color)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else if tabs_focused {
                Style::default()
                    .bg(Color::LightBlue)
                    .fg(Color::Black)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .bg(Color::Blue)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            };

            // Draw label as a filled pill
            let label_text = format!(" {}: {} ", i + 1, tab_labels[i]);
            let avail = tab_end.saturating_sub(tab_start) as usize;
            let display: String = if label_text.len() > avail {
                label_text.chars().take(avail).collect()
            } else {
                label_text
            };
            for (ci, ch) in display.chars().enumerate() {
                let x = tab_start + ci as u16;
                if x < tab_end && x < buf.area().width && border_y < buf.area().height {
                    buf[(x, border_y)].set_char(ch).set_style(active_style);
                }
            }
        } else {
            // Inactive tab: dimmed label on the border line
            let label = format!(" {}: {} ", i + 1, tab_labels[i]);
            let avail = (tab_end.saturating_sub(tab_start)) as usize;
            let display: String = if label.len() > avail {
                label.chars().take(avail).collect()
            } else {
                label
            };
            let inactive_style = Style::default().fg(Color::Gray);
            for (ci, ch) in display.chars().enumerate() {
                let x = tab_start + ci as u16;
                if x < tab_end && x < buf.area().width && border_y < buf.area().height {
                    buf[(x, border_y)].set_char(ch).set_style(inactive_style);
                }
            }
        }
    }

    // --- Agent status bar on bottom border of terminal pane ---
    {
        let bottom_y = pane.bottom().saturating_sub(1);
        let inner_left = pane.x + 1;
        let inner_right = pane.right().saturating_sub(1);
        let buf = frame.buffer_mut();

        // Toggle on the left side
        let yolo = app.settings.yolo_mode;
        let (toggle_text, toggle_style) = if yolo {
            (
                "YOLO Mode \u{2501}\u{2501}\u{2501}\u{2501}\u{2501}\u{25CF}",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            (
                "\u{25CF}\u{2501}\u{2501}\u{2501}\u{2501}\u{2501} Safe Mode",
                Style::default().fg(ORANGE),
            )
        };

        let toggle_start = inner_left + 1;
        let toggle_len = toggle_text.chars().count() as u16;
        for (i, ch) in toggle_text.chars().enumerate() {
            let x = toggle_start + i as u16;
            if x < inner_right && x >= inner_left && bottom_y < buf.area().height {
                buf[(x, bottom_y)].set_char(ch).set_style(toggle_style);
            }
        }

        // Command-mode indicator: yellow pill just right of the YOLO/Safe Mode toggle.
        let command_mode_on = app.terminal_command_mode();
        let command_badge = " [command] ";
        let command_len = command_badge.chars().count() as u16;
        let gap_before_badge = 2u16;
        let badge_start = toggle_start + toggle_len + gap_before_badge;
        if command_mode_on {
            let badge_style = Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD);
            for (i, ch) in command_badge.chars().enumerate() {
                let x = badge_start + i as u16;
                if x < inner_right && x >= inner_left && bottom_y < buf.area().height {
                    buf[(x, bottom_y)].set_char(ch).set_style(badge_style);
                }
            }
        }
        let left_content_end = if command_mode_on {
            badge_start + command_len
        } else {
            toggle_start + toggle_len
        };

        // Agent status fields right-aligned
        if let Some(ws_id) = ws_id {
            if let Some(status) = app.agent_status(ws_id) {
                let mut segments: Vec<(String, Style)> = Vec::new();
                if let Some(ref model) = status.model {
                    segments.push((
                        model.clone(),
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                if let Some(ref effort) = status.effort {
                    segments.push((
                        format!("Effort: {}", effort),
                        Style::default().fg(Color::White),
                    ));
                }
                if let Some(ref pct) = status.context_pct {
                    segments.push((
                        pct.clone(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                }

                let gap = 2u16;
                let mut cursor = inner_right.saturating_sub(1);
                let left_limit = left_content_end + gap;
                for (text, style) in segments.iter().rev() {
                    let len = text.chars().count() as u16;
                    let start = cursor.saturating_sub(len);
                    if start < left_limit {
                        break;
                    }
                    for (i, ch) in text.chars().enumerate() {
                        let x = start + i as u16;
                        if x < cursor && x >= inner_left && bottom_y < buf.area().height {
                            buf[(x, bottom_y)].set_char(ch).set_style(*style);
                        }
                    }
                    cursor = start.saturating_sub(gap);
                }
            }
        }
    }

    // --- "Scroll to bottom" indicator on terminal bottom border ---
    if let Some(ws_id) = ws_id {
        if app.terminal_scrollback_active(ws_id, &app.active_tab_id()) {
            let bottom_y = pane.bottom().saturating_sub(1);
            let inner_right = pane.right().saturating_sub(1);
            let buf = frame.buffer_mut();
            let label = format!(
                " \u{2193} Scroll to bottom ({}) ",
                app.settings.scroll_to_bottom_key
            );
            let label_len = label.chars().count() as u16;
            let center_x = pane.x + (pane.width.saturating_sub(label_len)) / 2;
            let style = Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD);
            for (i, ch) in label.chars().enumerate() {
                let x = center_x + i as u16;
                if x > pane.x && x < inner_right && bottom_y < buf.area().height {
                    buf[(x, bottom_y)].set_char(ch).set_style(style);
                }
            }
        }
    }

    // --- Footer ---
    footer::render(frame, l.footer, app);

    // --- Toast overlay for git action messages ---
    if let Some((msg, ts)) = &app.git_action_message {
        if ts.elapsed() < std::time::Duration::from_secs(3) {
            let toast_width = (msg.len() as u16 + 4).min(area.width);
            let toast_rect = Rect::new(
                area.x + area.width.saturating_sub(toast_width).saturating_sub(1),
                area.y + area.height.saturating_sub(4),
                toast_width,
                3,
            );
            frame.render_widget(Clear, toast_rect);
            frame.render_widget(
                Paragraph::new(msg.as_str()).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Green)),
                ),
                toast_rect,
            );
        }
    }

    // --- Create Branch modal ---
    if let Some(input) = &app.create_branch_input {
        let modal_w = 60u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new(format!("{input}_"))
                .block(
                    Block::default()
                        .title("New Branch (Enter to create, Esc to cancel)")
                        .borders(Borders::ALL)
                        .border_style(
                            Style::default()
                                .fg(Color::LightBlue)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Commit modal ---
    if let Some(input) = &app.commit_input {
        let modal_w = 60u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new(format!("{input}_"))
                .block(
                    Block::default()
                        .title("Commit Message (Enter to commit, Esc to cancel)")
                        .borders(Borders::ALL)
                        .border_style(
                            Style::default()
                                .fg(Color::LightBlue)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Discard confirmation modal ---
    if let Some(file) = &app.confirm_discard_file {
        let modal_w = 60u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new(format!("Discard changes to {file}?"))
                .block(
                    Block::default()
                        .title("Confirm (y/Enter = yes, n/Esc = cancel)")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Discard-all confirmation modal ---
    if app.confirm_discard_all.is_some() {
        let modal_w = 64u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new("Discard ALL uncommitted changes? Cannot be undone.")
                .block(
                    Block::default()
                        .title("Confirm (y/Enter = yes, n/Esc = cancel)")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Stash-pull-pop confirmation modal ---
    if app.confirm_stash_pull_pop.is_some() {
        let modal_w = 64u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new("Local changes would be overwritten. Stash, pull, then pop?")
                .block(
                    Block::default()
                        .title("Confirm (y/Enter = yes, n/Esc = cancel)")
                        .borders(Borders::ALL)
                        .border_style(
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Delete branch confirmation modal ---
    if let Some(ref target) = app.confirm_delete_branch {
        let prompt = match target {
            crate::app::DeleteBranchTarget::Local { branch } => {
                format!("Delete local branch '{branch}'?")
            }
            crate::app::DeleteBranchTarget::Remote { full_name, .. } => {
                format!("Delete remote branch '{full_name}'?")
            }
        };
        let modal_w = 60u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new(prompt)
                .block(
                    Block::default()
                        .title("Confirm (y = yes, n/Esc/Enter = cancel)")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Stash message modal ---
    if let Some(input) = &app.stash_input {
        let modal_w = 60u16.min(area.width.saturating_sub(4));
        let modal_h = 5u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new(format!("{input}_"))
                .block(
                    Block::default()
                        .title("Stash Message (Enter to stash, Esc to cancel)")
                        .borders(Borders::ALL)
                        .border_style(
                            Style::default()
                                .fg(Color::LightBlue)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }

    // --- Resurrect-command overlay ---
    if let Some(cmd) = app.pending_resurrect_command() {
        let preview = crate::resurrect::preview_line(cmd);
        let max_inner_w = area.width.saturating_sub(6) as usize;
        let desired_w = preview.chars().count().saturating_add(6).max(40);
        let modal_w = (desired_w.min(max_inner_w.max(40)) as u16).min(area.width.saturating_sub(2));
        let modal_h = 7u16;
        let modal_rect = Rect::new(
            area.x + (area.width.saturating_sub(modal_w)) / 2,
            area.y + (area.height.saturating_sub(modal_h)) / 2,
            modal_w,
            modal_h,
        );
        let body = vec![
            Line::from(Span::styled(
                "Last running:",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("  {preview}")),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "[Enter]",
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" to re-run    "),
                Span::styled(
                    "[Esc]",
                    Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" for shell"),
            ]),
        ];
        frame.render_widget(Clear, modal_rect);
        frame.render_widget(
            Paragraph::new(body)
                .block(
                    Block::default()
                        .title("Resume command")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(ORANGE).add_modifier(Modifier::BOLD))
                        .border_type(BorderType::Thick),
                )
                .wrap(Wrap { trim: false }),
            modal_rect,
        );
    }
}

pub fn hit_test(area: Rect, app: &TuiApp, x: u16, y: u16) -> Option<WorkspaceHit> {
    let l = layout(area, app.focus, app.terminal_fullscreen());

    let point_inside = |r: Rect| x >= r.x && y >= r.y && x < r.right() && y < r.bottom();

    // Check if click is on the workspace bar
    if point_inside(l.workspace_bar) {
        if let Some(idx) =
            crate::ui::widgets::workspace_bar::pill_index_at(l.workspace_bar, &app.workspaces, x, y)
        {
            return Some(WorkspaceHit::WorkspaceBarPill(idx));
        }
        return None;
    }

    // Check if click is on the terminal pane's top border (tab area)
    let border_y = l.terminal_pane.y;
    if y == border_y && x >= l.terminal_pane.x && x < l.terminal_pane.right() {
        // Compute tab ranges (same logic as render)
        if !app.ws_tabs.is_empty() {
            let pane = l.terminal_pane;
            let inner_left = pane.x + 1;
            let inner_right = pane.right().saturating_sub(1);
            let inner_width = inner_right.saturating_sub(inner_left);
            let ws_info_width = {
                let info = ws_info_string(app);
                let max_w = inner_width / 2;
                (info.len() as u16).min(max_w)
            };
            let tab_label_widths: Vec<u16> = app
                .ws_tabs
                .iter()
                .enumerate()
                .map(|(i, tab)| {
                    let label = if i == app.ws_active_tab {
                        app.rename_tab_input
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|| tab.label.clone())
                    } else {
                        tab.label.clone()
                    };
                    format!(" {}: {} ", i + 1, label).len() as u16
                })
                .collect();
            let ranges = compute_tab_ranges(&pane, ws_info_width, &tab_label_widths);

            for (i, &(start, end)) in ranges.iter().enumerate() {
                if x >= start && x < end {
                    return Some(WorkspaceHit::TerminalTab(i));
                }
            }
        }
        // Click on border but not on a tab — treat as terminal pane
        return Some(WorkspaceHit::TerminalPane);
    }
    // Check if click is on the "scroll to bottom" indicator (terminal bottom border)
    {
        let bottom_y = l.terminal_pane.bottom().saturating_sub(1);
        if y == bottom_y && x >= l.terminal_pane.x && x < l.terminal_pane.right() {
            let label = format!(
                " \u{2193} Scroll to bottom ({}) ",
                app.settings.scroll_to_bottom_key
            );
            let label_len = label.chars().count() as u16;
            let center_x =
                l.terminal_pane.x + (l.terminal_pane.width.saturating_sub(label_len)) / 2;
            if x >= center_x && x < center_x + label_len {
                if let Some(ws_id) = app.active_workspace_id() {
                    if app.terminal_scrollback_active(ws_id, &app.active_tab_id()) {
                        return Some(WorkspaceHit::ScrollToBottom);
                    }
                }
            }
        }
    }
    if point_inside(l.terminal_pane) {
        return Some(WorkspaceHit::TerminalPane);
    }
    if point_inside(l.git_diff) {
        return Some(WorkspaceHit::DiffPane);
    }
    if point_inside(l.git_log) {
        let total = app.total_log_items();
        if total == 0 {
            return Some(WorkspaceHit::LogList(0));
        }
        let content_top = l.git_log.y.saturating_add(1);
        if y < content_top {
            return Some(WorkspaceHit::LogList(0));
        }
        let idx = (y - content_top) as usize;
        return Some(WorkspaceHit::LogList(idx.min(total - 1)));
    }
    if point_inside(l.git_branches) {
        let content_top = l.git_branches.y.saturating_add(1);
        if y < content_top {
            return Some(WorkspaceHit::BranchesPane(0));
        }
        let idx = (y - content_top) as usize;
        return Some(WorkspaceHit::BranchesPane(idx));
    }
    None
}

/// Returns the `Rect` of the pane containing the given point, if any.
///
/// Used to confine mouse drag selections to a single pane.
pub fn pane_rect_at(area: Rect, app: &TuiApp, x: u16, y: u16) -> Option<Rect> {
    let l = layout(area, app.focus, app.terminal_fullscreen());
    let point_inside = |r: Rect| x >= r.x && y >= r.y && x < r.right() && y < r.bottom();

    if point_inside(l.terminal_pane) {
        return Some(l.terminal_pane);
    }
    if point_inside(l.git_log) {
        return Some(l.git_log);
    }
    if point_inside(l.git_diff) {
        return Some(l.git_diff);
    }
    if point_inside(l.git_branches) {
        return Some(l.git_branches);
    }
    None
}

pub fn terminal_content_rect(
    area: Rect,
    focus: crate::app::Focus,
    terminal_fullscreen: bool,
) -> Rect {
    let pane = layout(area, focus, terminal_fullscreen).terminal_pane;
    Rect::new(
        pane.x.saturating_add(1),
        pane.y.saturating_add(1),
        pane.width.saturating_sub(2),
        pane.height.saturating_sub(2),
    )
}

/// Returns the outer `Rect` of every bordered pane in the workspace screen.
///
/// Used during text extraction so that border cells can be replaced with spaces,
/// preventing box-drawing characters from leaking into clipboard text.
pub fn border_rects(area: Rect, app: &TuiApp) -> Vec<Rect> {
    let l = layout(area, app.focus, app.terminal_fullscreen());
    let mut rects = vec![l.terminal_pane, l.footer];

    if !app.terminal_fullscreen() {
        rects.push(l.git_log);
        rects.push(l.git_diff);

        // Branch sub-pane split (local/remote)
        let branch_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(l.git_branches);
        rects.push(branch_split[0]);
        rects.push(branch_split[1]);
    }

    rects.retain(|r| r.width > 0 && r.height > 0);
    rects
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    // --- pane_border_style tests ---

    #[test]
    fn pane_border_no_attention_unfocused() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::None, false, false);
        assert_eq!(border_type, BorderType::Plain);
        assert_eq!(style.fg, Some(Color::White));
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn pane_border_no_attention_focused() {
        let (style, border_type) = pane_border_style(true, AttentionLevel::None, false, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_needs_input_flash_on() {
        let (style, border_type) = pane_border_style(true, AttentionLevel::NeedsInput, true, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(ORANGE));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_needs_input_flash_off() {
        // flash_off reverts to focused style
        let (style, border_type) =
            pane_border_style(true, AttentionLevel::NeedsInput, false, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_error_flash_on() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::Error, true, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_error_flash_off_unfocused() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::Error, false, false);
        assert_eq!(border_type, BorderType::Plain);
        assert_eq!(style.fg, Some(Color::White));
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn pane_border_notice_no_flash() {
        // Notice level does not trigger attention flash
        let (style, border_type) = pane_border_style(true, AttentionLevel::Notice, true, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_command_mode_overrides_all() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::None, false, true);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::Yellow));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    // --- build_terminal_title_line tests ---

    #[test]
    fn terminal_title_no_attention() {
        let line = build_terminal_title_line(AttentionLevel::None, false, false);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "Terminal");
    }

    #[test]
    fn terminal_title_needs_input() {
        let line = build_terminal_title_line(AttentionLevel::NeedsInput, true, false);
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "Terminal ");
        assert_eq!(line.spans[1].content, "⚠ input");
        assert_eq!(line.spans[1].style.fg, Some(ORANGE));
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn terminal_title_error() {
        let line = build_terminal_title_line(AttentionLevel::Error, true, false);
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[0].content, "Terminal ");
        assert_eq!(line.spans[1].content, "✖ error");
        assert_eq!(line.spans[1].style.fg, Some(Color::Red));
        assert!(line.spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    // --- hit_test tests ---

    use crate::app::Focus;
    use protocol::WorkspaceSummary;

    fn make_ws_summary() -> WorkspaceSummary {
        WorkspaceSummary {
            id: uuid::Uuid::new_v4(),
            name: "test".into(),
            path: "/tmp/test".into(),
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

    fn app_with_workspace() -> (crate::app::TuiApp, uuid::Uuid) {
        let mut app = crate::app::TuiApp::default();
        let ws = make_ws_summary();
        let id = ws.id;
        app.set_workspaces(vec![ws]);
        app.open_workspace(id);
        (app, id)
    }

    /// Helper to get the first tab's x-range for hit testing.
    fn first_tab_x(app: &TuiApp, area: Rect) -> u16 {
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let pane = l.terminal_pane;
        let inner_left = pane.x + 1;
        let inner_right = pane.right().saturating_sub(1);
        let inner_width = inner_right.saturating_sub(inner_left);
        let info = ws_info_string(app);
        let ws_info_width = (info.len() as u16).min(inner_width / 2);
        let label_widths: Vec<u16> = app
            .ws_tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| format!(" {}: {} ", i + 1, tab.label).len() as u16)
            .collect();
        let ranges = compute_tab_ranges(&pane, ws_info_width, &label_widths);
        ranges.first().map(|r| r.0 + 1).unwrap_or(inner_left)
    }

    #[test]
    fn hit_test_border_line_returns_tab() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let tab_x = first_tab_x(&app, area);
        let result = hit_test(area, &app, tab_x, l.terminal_pane.y);
        assert!(matches!(result, Some(WorkspaceHit::TerminalTab(_))));
    }

    #[test]
    fn hit_test_terminal_pane() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, app.focus, app.terminal_fullscreen());
        // Click inside the terminal pane content area (below the top border)
        let result = hit_test(area, &app, 10, l.terminal_pane.y + 2);
        assert_eq!(result, Some(WorkspaceHit::TerminalPane));
    }

    #[test]
    fn hit_test_git_log() {
        let (mut app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        // Switch focus to something neutral so git panes get a reasonable size.
        app.focus = Focus::WsLog;
        // Git panes are in the lower body section. With WsLog focus the git area gets 65%.
        // Body = 35 rows (3..38). Terminal gets 35% = ~12 rows, git gets 65% = ~23 rows.
        // Git area starts around row 15. Left 35% has log (top) and branches (bottom).
        // The git log is in the top half of the left git area.
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let result = hit_test(area, &app, l.git_log.x + 1, l.git_log.y + 1);
        assert!(matches!(result, Some(WorkspaceHit::LogList(_))));
    }

    #[test]
    fn hit_test_branches_pane() {
        let (mut app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        app.focus = Focus::WsBranches;
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let result = hit_test(area, &app, l.git_branches.x + 1, l.git_branches.y + 1);
        assert!(matches!(result, Some(WorkspaceHit::BranchesPane(_))));
    }

    #[test]
    fn hit_test_diff_pane() {
        let (mut app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        app.focus = Focus::WsDiff;
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let result = hit_test(area, &app, l.git_diff.x + 1, l.git_diff.y + 1);
        assert_eq!(result, Some(WorkspaceHit::DiffPane));
    }

    #[test]
    fn hit_test_terminal_tabs_on_border() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let tab_x = first_tab_x(&app, area);
        let result = hit_test(area, &app, tab_x, l.terminal_pane.y);
        assert!(matches!(result, Some(WorkspaceHit::TerminalTab(_))));
    }

    #[test]
    fn hit_test_footer_returns_none() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        // Footer is the last 2 rows: rows 38..40
        let result = hit_test(area, &app, 10, 39);
        assert_eq!(result, None);
    }

    #[test]
    fn hit_test_fullscreen_git_area_returns_terminal_or_none() {
        let (mut app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        app.toggle_terminal_fullscreen();
        // In fullscreen mode, git panes are zero-sized.
        // The area that would normally be git log should now be terminal pane or None.
        let l_normal = layout(area, Focus::WsLog, false);
        let git_log_x = l_normal.git_log.x + 1;
        let git_log_y = l_normal.git_log.y + 1;
        let result = hit_test(area, &app, git_log_x, git_log_y);
        // Should be TerminalPane (terminal expands to fill) or None, but NOT LogList
        assert!(!matches!(result, Some(WorkspaceHit::LogList(_))));
    }

    // --- render smoke tests ---

    use crate::app::TuiApp;
    use protocol::{BranchInfo, ChangedFile, CommitInfo, GitState, RemoteBranchInfo};

    fn smoke_render_workspace(app: &TuiApp, width: u16, height: u16) {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::render(frame, area, app);
            })
            .unwrap();
    }

    fn line_to_string(line: Line<'_>) -> String {
        line.spans
            .into_iter()
            .map(|span| match span.content {
                Cow::Borrowed(s) => s.to_string(),
                Cow::Owned(s) => s,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn make_git_state() -> GitState {
        GitState {
            branch: Some("main".into()),
            upstream: Some("origin/main".into()),
            ahead: Some(1),
            behind: Some(0),
            changed: vec![
                ChangedFile {
                    path: "src/lib.rs".into(),
                    index_status: 'M',
                    worktree_status: ' ',
                },
                ChangedFile {
                    path: "README.md".into(),
                    index_status: '?',
                    worktree_status: '?',
                },
            ],
            recent_commits: vec![
                CommitInfo {
                    hash: "abc1234".into(),
                    message: "feat: initial commit".into(),
                    author: "dev".into(),
                    date: "2025-01-01".into(),
                },
                CommitInfo {
                    hash: "def5678".into(),
                    message: "fix: bug fix".into(),
                    author: "dev".into(),
                    date: "2025-01-02".into(),
                },
            ],
            local_branches: vec![
                BranchInfo {
                    name: "main".into(),
                    is_head: true,
                    ahead: Some(1),
                    behind: Some(0),
                },
                BranchInfo {
                    name: "feature".into(),
                    is_head: false,
                    ahead: Some(0),
                    behind: Some(2),
                },
            ],
            remote_branches: vec![RemoteBranchInfo {
                full_name: "origin/main".into(),
            }],
            tags: Vec::new(),
        }
    }

    #[test]
    fn render_workspace_basic() {
        let (app, _) = app_with_workspace();
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_workspace_with_git_state() {
        let (mut app, id) = app_with_workspace();
        app.set_workspace_git(id, make_git_state());
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_workspace_fullscreen() {
        let (mut app, _) = app_with_workspace();
        app.toggle_terminal_fullscreen();
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_workspace_commit_modal() {
        let (mut app, _) = app_with_workspace();
        app.commit_input = Some("msg".into());
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_workspace_create_branch_modal() {
        let (mut app, _) = app_with_workspace();
        app.create_branch_input = Some("new-branch".into());
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_workspace_very_small_terminal() {
        let (app, _) = app_with_workspace();
        smoke_render_workspace(&app, 20, 10);
    }

    #[test]
    fn render_workspace_expanded_uncommitted() {
        let (mut app, id) = app_with_workspace();
        app.set_workspace_git(id, make_git_state());
        app.ws_uncommitted_expanded = true;
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn changed_file_line_makes_unstaged_status_explicit() {
        let line = changed_file_line(&ChangedFile {
            path: "models/spectrogram/checkpoint_epoch_005.pt".into(),
            index_status: ' ',
            worktree_status: 'M',
        });
        assert_eq!(
            line_to_string(line),
            "  I:- W:M models/spectrogram/checkpoint_epoch_005.pt"
        );
    }

    #[test]
    fn changed_file_line_distinguishes_staged_and_unstaged() {
        let staged = line_to_string(changed_file_line(&ChangedFile {
            path: "checkpoint.pt".into(),
            index_status: 'M',
            worktree_status: ' ',
        }));
        let unstaged = line_to_string(changed_file_line(&ChangedFile {
            path: "checkpoint.pt".into(),
            index_status: ' ',
            worktree_status: 'M',
        }));

        assert_eq!(staged, "  I:M W:- checkpoint.pt");
        assert_eq!(unstaged, "  I:- W:M checkpoint.pt");
    }

    // --- border_rects tests ---

    #[test]
    fn border_rects_normal_mode() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        let rects = border_rects(area, &app);
        // Should have: terminal_pane, footer,
        // git_log, git_diff, 2 branch sub-panes = 6 minimum
        assert!(rects.len() >= 6, "got {} rects", rects.len());
        for r in &rects {
            assert!(r.width > 0 && r.height > 0, "zero-sized rect: {:?}", r);
        }
    }

    #[test]
    fn border_rects_fullscreen_has_fewer() {
        let (mut app, _) = app_with_workspace();
        app.toggle_terminal_fullscreen();
        let area = Rect::new(0, 0, 120, 40);
        let rects_fs = border_rects(area, &app);

        app.toggle_terminal_fullscreen();
        let rects_normal = border_rects(area, &app);

        assert!(
            rects_fs.len() < rects_normal.len(),
            "fullscreen {} should have fewer rects than normal {}",
            rects_fs.len(),
            rects_normal.len()
        );
    }

    // --- New browser-tab layout tests ---

    #[test]
    fn layout_terminal_pane_starts_at_top() {
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, Focus::WsTerminal, false);
        // Terminal pane starts below the 2-line workspace bar (row 2)
        assert_eq!(l.terminal_pane.y, 2);
    }

    #[test]
    fn layout_fullscreen_terminal_fills_body() {
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, Focus::WsTerminal, true);
        // In fullscreen, terminal pane gets all rows except bar (2) and footer (2)
        assert_eq!(l.terminal_pane.height, 36);
    }

    #[test]
    fn layout_terminal_pane_reclaimed_space() {
        // The new layout gives all vertical space to the terminal pane
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, Focus::WsTerminal, false);
        // Terminal pane should have at least 10 rows at 120x40
        assert!(
            l.terminal_pane.height >= 10,
            "terminal pane should have reclaimed space, got {} rows",
            l.terminal_pane.height
        );
    }

    #[test]
    fn border_rects_includes_terminal_pane() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        let rects = border_rects(area, &app);
        let l = layout(area, app.focus, app.terminal_fullscreen());
        assert!(
            rects.contains(&l.terminal_pane),
            "border_rects should include the terminal pane"
        );
    }

    #[test]
    fn render_with_attention_flash_smoke() {
        let (mut app, id) = app_with_workspace();
        // Set agent attention to NeedsInput
        if let Some(ws) = app.workspaces.iter_mut().find(|w| w.id == id) {
            ws.attention = AttentionLevel::NeedsInput;
        }
        app.spinner_tick = 0; // flash on
        smoke_render_workspace(&app, 120, 40);
        app.spinner_tick = 1; // flash off
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_with_rename_tab_smoke() {
        let (mut app, _) = app_with_workspace();
        app.rename_tab_input = Some("my-agent".into());
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_with_rename_workspace_smoke() {
        let (mut app, _) = app_with_workspace();
        app.rename_workspace_input = Some("new-name".into());
        smoke_render_workspace(&app, 120, 40);
    }

    #[test]
    fn render_fullscreen_with_notch() {
        let (mut app, _) = app_with_workspace();
        app.toggle_terminal_fullscreen();
        smoke_render_workspace(&app, 120, 40);
        // Also test very small
        smoke_render_workspace(&app, 30, 8);
    }

    #[test]
    fn ws_info_string_basic() {
        let (mut app, id) = app_with_workspace();
        app.set_workspace_git(id, make_git_state());
        let info = ws_info_string(&app);
        assert!(
            info.contains("test"),
            "should contain workspace name: {}",
            info
        );
        assert!(
            info.contains("main"),
            "should contain branch name: {}",
            info
        );
    }

    #[test]
    fn ws_info_string_rename_mode() {
        let (mut app, _) = app_with_workspace();
        app.rename_workspace_input = Some("new-name".into());
        let info = ws_info_string(&app);
        assert!(
            info.contains("Rename:"),
            "should show rename prompt: {}",
            info
        );
    }

    #[test]
    fn terminal_content_rect_inset_by_one() {
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, Focus::WsTerminal, false);
        let content = terminal_content_rect(area, Focus::WsTerminal, false);
        assert_eq!(content.x, l.terminal_pane.x + 1);
        assert_eq!(content.y, l.terminal_pane.y + 1);
        assert_eq!(content.width, l.terminal_pane.width - 2);
        assert_eq!(content.height, l.terminal_pane.height - 2);
    }

    #[test]
    fn hit_test_multiple_tabs() {
        let (mut app, _) = app_with_workspace();
        // Add a second shell tab
        app.ws_tabs.push(crate::app::TerminalTab {
            id: "shell-2".into(),
            label: "shell".into(),
            kind: protocol::TerminalKind::Shell,
            fullscreen: false,
            last_command: None,
            overlay_dismissed: false,
        });
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, app.focus, app.terminal_fullscreen());
        // Compute ranges and click in the third tab
        let pane = l.terminal_pane;
        let inner_left = pane.x + 1;
        let inner_right = pane.right().saturating_sub(1);
        let inner_width = inner_right.saturating_sub(inner_left);
        let info = ws_info_string(&app);
        let ws_info_width = (info.len() as u16).min(inner_width / 2);
        let label_widths: Vec<u16> = app
            .ws_tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| format!(" {}: {} ", i + 1, tab.label).len() as u16)
            .collect();
        let ranges = compute_tab_ranges(&pane, ws_info_width, &label_widths);
        assert!(
            ranges.len() >= 3,
            "expected 3 tab ranges, got {}",
            ranges.len()
        );
        let third_tab_x = ranges[2].0 + 1;
        let result = hit_test(area, &app, third_tab_x, l.terminal_pane.y);
        assert!(
            matches!(result, Some(WorkspaceHit::TerminalTab(2))),
            "expected TerminalTab(2), got {:?}",
            result
        );
    }
}
