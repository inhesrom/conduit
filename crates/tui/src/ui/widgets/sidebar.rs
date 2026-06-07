//! The persistent Repository → Workspace tree on the left of the screen.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use protocol::AttentionLevel;

use crate::app::{Focus, SidebarRow, TuiApp};

/// Steady, non-flashing colour for the ready-for-review marker — deliberately
/// distinct from the attention orange/red so the two never read the same.
pub const REVIEW: Color = Color::Magenta;

/// Default sidebar width in columns.
pub const WIDTH: u16 = 30;

pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let focused = app.focus == Focus::Sidebar;
    let border_style = if focused {
        Style::default().fg(Color::LightBlue)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let title = if app.sidebar_review_filter {
        " Workspaces (review) "
    } else {
        " Workspaces "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = app.sidebar_rows();
    if rows.is_empty() {
        let hint = vec![
            Line::from(""),
            Line::from(Span::styled(
                "  no repositories",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  press N to add one",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        frame.render_widget(Paragraph::new(hint), inner);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let mut line = match row {
            SidebarRow::Repo(id) => repo_line(app, *id),
            SidebarRow::Workspace(wid) => workspace_line(app, *wid),
        };
        if i == app.sidebar_selected {
            let bg = if focused {
                Color::Rgb(40, 44, 72)
            } else {
                Color::Rgb(32, 32, 40)
            };
            line = line.style(Style::default().bg(bg));
        }
        lines.push(line);
    }

    // Keep the selected row visible with a simple scroll offset.
    let h = inner.height.max(1) as usize;
    let scroll = app.sidebar_selected.saturating_sub(h.saturating_sub(1)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
}

/// Maps a click at `(x, y)` within the sidebar `area` to a flattened row index.
/// Mirrors `render`'s block-inner + scroll math exactly so clicks line up after
/// scrolling. Returns `None` if the click is outside the inner content area.
pub fn row_index_at(area: Rect, app: &TuiApp, x: u16, y: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    if x < inner.x || x >= inner.right() || y < inner.y || y >= inner.bottom() {
        return None;
    }
    let rows = app.sidebar_rows();
    if rows.is_empty() {
        return None;
    }
    let h = inner.height.max(1) as usize;
    let scroll = app.sidebar_selected.saturating_sub(h.saturating_sub(1));
    let idx = (y - inner.y) as usize + scroll;
    (idx < rows.len()).then_some(idx)
}

fn repo_line(app: &TuiApp, id: protocol::RepositoryId) -> Line<'static> {
    let Some(repo) = app.repositories.iter().find(|r| r.id == id) else {
        return Line::from("");
    };
    let collapsed = app.collapsed_repos.contains(&id);
    let caret = if collapsed { "▸" } else { "▾" };
    let mut spans = vec![
        Span::styled(format!("{caret} "), Style::default().fg(Color::Gray)),
        Span::styled(repo.name.clone(), Style::default().add_modifier(Modifier::BOLD)),
    ];
    if let Some(branch) = &repo.default_branch {
        spans.push(Span::styled(
            format!("  {branch}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if repo.ready_for_review_count > 0 {
        spans.push(Span::styled(
            format!("  ◆{}", repo.ready_for_review_count),
            Style::default().fg(REVIEW),
        ));
    }
    Line::from(spans)
}

fn workspace_line(app: &TuiApp, wid: protocol::WorkspaceId) -> Line<'static> {
    let Some(ws) = app.workspaces.iter().find(|w| w.id == wid) else {
        return Line::from("");
    };
    let marker = match ws.attention {
        AttentionLevel::NeedsInput => Span::styled("⚠ ", Style::default().fg(Color::Yellow)),
        AttentionLevel::Error => Span::styled("✖ ", Style::default().fg(Color::Red)),
        _ => Span::raw("  "),
    };
    let name_style = if ws.agent_running {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut spans = vec![Span::raw("  "), marker, Span::styled(ws.name.clone(), name_style)];
    if ws.dirty_files > 0 {
        spans.push(Span::styled(
            format!(" ±{}", ws.dirty_files),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if ws.ready_for_review {
        spans.push(Span::styled("  ◆", Style::default().fg(REVIEW)));
    }
    Line::from(spans)
}
