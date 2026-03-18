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
use protocol::{AttentionLevel, Route};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceHit {
    Header,
    TerminalTab(usize),
    TerminalPane,
    LogList(usize),
    BranchesPane(usize),
    DiffPane,
}

#[derive(Debug, Clone, Copy)]
struct WorkspaceLayout {
    header: Rect,
    terminal_tabs: Rect,
    terminal_pane: Rect,
    git_log: Rect,
    git_branches: Rect,
    git_diff: Rect,
    footer: Rect,
}

fn layout(area: Rect, focus: crate::app::Focus, terminal_fullscreen: bool) -> WorkspaceLayout {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    if terminal_fullscreen {
        let terminal_area = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(5), Constraint::Min(3)])
            .split(chunks[1]);
        let zero = Rect::new(0, 0, 0, 0);
        return WorkspaceLayout {
            header: chunks[0],
            terminal_tabs: terminal_area[0],
            terminal_pane: terminal_area[1],
            git_log: zero,
            git_branches: zero,
            git_diff: zero,
            footer: chunks[2],
        };
    }

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints(match focus {
            crate::app::Focus::WsTerminal | crate::app::Focus::WsTerminalTabs => {
                [Constraint::Percentage(72), Constraint::Percentage(28)]
            }
            crate::app::Focus::WsLog
            | crate::app::Focus::WsBranches
            | crate::app::Focus::WsDiff => [Constraint::Percentage(35), Constraint::Percentage(65)],
            _ => [Constraint::Percentage(55), Constraint::Percentage(45)],
        })
        .split(chunks[1]);
    let terminal_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(3)])
        .split(body[0]);
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
        header: chunks[0],
        terminal_tabs: terminal_area[0],
        terminal_pane: terminal_area[1],
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
) -> (Style, BorderType) {
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
pub fn build_terminal_title_line(
    attention: AttentionLevel,
    flash_on: bool,
    passthrough: bool,
) -> Line<'static> {
    let raw_badge = if passthrough {
        Some(Span::styled(
            " [passthrough]",
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

    let title = ws_id
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
            format!(
                "Workspace: {} ({})  {}  ◈{}{}",
                w.name, w.path, branch, w.dirty_files, ab
            )
        })
        .unwrap_or_else(|| "Workspace".to_string());

    let (header_style, header_border_type) = standard_border_style(false);
    frame.render_widget(
        Paragraph::new(if let Some(name) = &app.rename_workspace_input {
            format!("{title}\nRename: {name}")
        } else {
            title
        })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(header_style)
                .border_type(header_border_type),
        ),
        l.header,
    );

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
        let diff_text = ws_id
            .and_then(|id| app.workspace_diff.get(&id))
            .map(|(_, d)| d.clone())
            .unwrap_or_else(|| "Select a file and press Enter to load diff.".to_string());
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

    // --- Terminal Tabs ---
    let ws_summary = ws_id.and_then(|id| app.workspaces.iter().find(|w| w.id == id));
    let (agent_running, shell_running) = ws_summary
        .map(|w| (w.agent_running, w.shell_running))
        .unwrap_or((false, false));
    let (tabs_border_style, tabs_border_type) =
        standard_border_style(app.focus == crate::app::Focus::WsTerminalTabs);
    let tabs_block = Block::default()
        .title("Tabs")
        .borders(Borders::ALL)
        .border_style(tabs_border_style)
        .border_type(tabs_border_type);
    let tabs_inner = tabs_block.inner(l.terminal_tabs);
    frame.render_widget(tabs_block, l.terminal_tabs);

    let tab_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(
            app.ws_tabs
                .iter()
                .map(|_| Constraint::Ratio(1, app.ws_tabs.len().max(1) as u32))
                .collect::<Vec<_>>(),
        )
        .split(tabs_inner);
    for (i, tab) in app.ws_tabs.iter().enumerate() {
        let running = match tab.kind {
            protocol::TerminalKind::Agent => agent_running,
            protocol::TerminalKind::Shell => shell_running,
        };
        let label = if i == app.ws_active_tab {
            app.rename_tab_input
                .as_ref()
                .cloned()
                .unwrap_or_else(|| tab.label.clone())
        } else {
            tab.label.clone()
        };
        let is_active = i == app.ws_active_tab;
        let is_agent = matches!(tab.kind, protocol::TerminalKind::Agent);
        let (border_style, border_type) = if is_active
            && is_agent
            && matches!(
                attention,
                AttentionLevel::NeedsInput | AttentionLevel::Error
            )
            && app.spinner_tick % 2 == 0
        {
            let color = match attention {
                AttentionLevel::Error => Color::Red,
                _ => ORANGE,
            };
            (
                Style::default().fg(color).add_modifier(Modifier::BOLD),
                BorderType::Thick,
            )
        } else if is_active {
            (
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
                BorderType::Thick,
            )
        } else {
            (
                Style::default().add_modifier(Modifier::DIM),
                BorderType::Plain,
            )
        };
        let status = if running { "run" } else { "stop" };
        let content = Line::from(format!("{label} {status}"));
        let tab_block = Block::default()
            .title(format!("{}", i + 1))
            .borders(Borders::ALL)
            .border_style(border_style)
            .border_type(border_type);
        frame.render_widget(Paragraph::new(content).block(tab_block), tab_rects[i]);
    }

    // --- Terminal Pane ---
    let terminal_focused = app.focus == crate::app::Focus::WsTerminal;
    let terminal_lines = ws_id
        .map(|id| app.terminal_lines(id, &app.active_tab_id()))
        .unwrap_or_else(|| vec![Line::from("No terminal output yet.")]);
    let (term_style, term_border_type) =
        pane_border_style(terminal_focused, attention, app.spinner_tick % 2 == 0);
    let term_title = build_terminal_title_line(
        attention,
        app.spinner_tick % 2 == 0,
        app.active_tab_passthrough(),
    );
    frame.render_widget(Clear, l.terminal_pane);
    frame.render_widget(
        Paragraph::new(terminal_lines).block(
            Block::default()
                .title(term_title)
                .borders(Borders::ALL)
                .border_style(term_style)
                .border_type(term_border_type),
        ),
        l.terminal_pane,
    );

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
}

pub fn hit_test(area: Rect, app: &TuiApp, x: u16, y: u16) -> Option<WorkspaceHit> {
    let l = layout(area, app.focus, app.terminal_fullscreen());

    let point_inside = |r: Rect| x >= r.x && y >= r.y && x < r.right() && y < r.bottom();
    if point_inside(l.header) {
        return Some(WorkspaceHit::Header);
    }
    if point_inside(l.terminal_tabs) {
        if app.ws_tabs.is_empty() {
            return Some(WorkspaceHit::TerminalTab(0));
        }
        let tab_w = (l.terminal_tabs.width / app.ws_tabs.len() as u16).max(1);
        let idx = ((x.saturating_sub(l.terminal_tabs.x)) / tab_w) as usize;
        return Some(WorkspaceHit::TerminalTab(idx.min(app.ws_tabs.len() - 1)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    // --- pane_border_style tests ---

    #[test]
    fn pane_border_no_attention_unfocused() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::None, false);
        assert_eq!(border_type, BorderType::Plain);
        assert_eq!(style.fg, Some(Color::White));
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn pane_border_no_attention_focused() {
        let (style, border_type) = pane_border_style(true, AttentionLevel::None, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_needs_input_flash_on() {
        let (style, border_type) = pane_border_style(true, AttentionLevel::NeedsInput, true);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(ORANGE));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_needs_input_flash_off() {
        // flash_off reverts to focused style
        let (style, border_type) = pane_border_style(true, AttentionLevel::NeedsInput, false);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_error_flash_on() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::Error, true);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pane_border_error_flash_off_unfocused() {
        let (style, border_type) = pane_border_style(false, AttentionLevel::Error, false);
        assert_eq!(border_type, BorderType::Plain);
        assert_eq!(style.fg, Some(Color::White));
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    #[test]
    fn pane_border_notice_no_flash() {
        // Notice level does not trigger attention flash
        let (style, border_type) = pane_border_style(true, AttentionLevel::Notice, true);
        assert_eq!(border_type, BorderType::Thick);
        assert_eq!(style.fg, Some(Color::LightBlue));
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

    #[test]
    fn hit_test_header() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        // Header is the first 3 rows (Constraint::Length(3))
        let result = hit_test(area, &app, 10, 1);
        assert_eq!(result, Some(WorkspaceHit::Header));
    }

    #[test]
    fn hit_test_terminal_pane() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        // Terminal pane is below tabs, in the upper body area.
        // Layout: header(3) + body split. Terminal tabs are Length(5), terminal pane is Min(3).
        // With WsTerminal focus the terminal gets 72% of body.
        // Body = rows 3..38. 72% of 35 = ~25. Tabs = 5 rows (rows 3..8), terminal = rest (rows 8..28).
        let result = hit_test(area, &app, 10, 15);
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
    fn hit_test_terminal_tabs() {
        let (app, _) = app_with_workspace();
        let area = Rect::new(0, 0, 120, 40);
        let l = layout(area, app.focus, app.terminal_fullscreen());
        let result = hit_test(area, &app, l.terminal_tabs.x + 1, l.terminal_tabs.y + 1);
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
}
