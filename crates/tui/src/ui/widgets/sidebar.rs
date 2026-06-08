//! The persistent Repository → Workspace tree on the left of the screen.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use protocol::AttentionLevel;

use crate::app::{Focus, SidebarMode, SidebarRow, TuiApp};

/// Steady, non-flashing colour for the ready-for-review marker — deliberately
/// distinct from the attention orange/red so the two never read the same.
pub const REVIEW: Color = Color::Magenta;
const AGENT_ACTIVE: Color = Color::LightBlue;
const BRAILLE_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Default sidebar width in columns.
pub const WIDTH: u16 = 30;

/// Width of the collapsed vertical rail in columns (borders + a 3-col strip).
pub const RAIL_WIDTH: u16 = 5;

/// On-screen width of the sidebar for a given display mode. Single source of
/// truth for the carve: rendering, mouse hit-testing, and embedded-terminal
/// sizing all derive their sidebar width from here, so the PTY can never be
/// told it is wider than the pane Conduit actually draws.
pub fn width(mode: SidebarMode) -> u16 {
    match mode {
        SidebarMode::Expanded => WIDTH,
        SidebarMode::Rail => RAIL_WIDTH,
        SidebarMode::Hidden => 0,
    }
}

/// Background colour for the selected row/strip when the sidebar is focused.
const SELECT_BG_FOCUSED: Color = Color::Rgb(40, 44, 72);
/// Background colour for the selected row/strip when the sidebar is unfocused.
const SELECT_BG_UNFOCUSED: Color = Color::Rgb(32, 32, 40);

/// Stronger row highlight used by expanded sidebar and pop-out selections.
const ROW_SELECT_BG_FOCUSED: Color = Color::LightBlue;
const ROW_SELECT_BG_UNFOCUSED: Color = Color::Blue;

/// Accent colour for the "you are here" bar marking the currently-open
/// workspace — persistent regardless of where the keyboard cursor sits.
const ACTIVE_ACCENT: Color = Color::Cyan;

fn row_selected_bg(focused: bool) -> Color {
    if focused {
        ROW_SELECT_BG_FOCUSED
    } else {
        ROW_SELECT_BG_UNFOCUSED
    }
}

fn row_selected_fg(focused: bool) -> Color {
    if focused {
        Color::Black
    } else {
        Color::White
    }
}

fn selected_row_line(mut line: Line<'static>, focused: bool) -> Line<'static> {
    let bg = row_selected_bg(focused);
    for span in &mut line.spans {
        let fg = selected_span_fg(span.style.fg, focused);
        span.style = span.style.bg(bg).fg(fg).add_modifier(Modifier::BOLD);
    }
    line.style(Style::default().bg(bg))
}

fn selected_span_fg(current: Option<Color>, focused: bool) -> Color {
    let Some(color) = current else {
        return row_selected_fg(focused);
    };
    if matches!(color, Color::Yellow | Color::Red) || color == REVIEW || color == ACTIVE_ACCENT {
        color
    } else {
        row_selected_fg(focused)
    }
}

fn paint_selected_row(frame: &mut Frame, inner: Rect, selected: usize, scroll: u16, focused: bool) {
    let Some(visible_row) = selected.checked_sub(scroll as usize) else {
        return;
    };
    if visible_row >= inner.height as usize {
        return;
    }
    frame.buffer_mut().set_style(
        Rect::new(inner.x, inner.y + visible_row as u16, inner.width, 1),
        Style::default().bg(row_selected_bg(focused)),
    );
}

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
            line = selected_row_line(line, focused);
        }
        lines.push(line);
    }

    // Keep the selected row visible with a simple scroll offset.
    let h = inner.height.max(1) as usize;
    let scroll = app.sidebar_selected.saturating_sub(h.saturating_sub(1)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
    paint_selected_row(frame, inner, app.sidebar_selected, scroll, focused);
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
        Span::styled(
            repo.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(branch) = &repo.default_branch {
        spans.push(Span::styled(
            format!("  {branch}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if repo_has_active_agent(app, id) {
        spans.push(Span::styled(
            format!("  {}", spinner_frame(app.spinner_tick)),
            Style::default().fg(AGENT_ACTIVE),
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
    let active = app.active_workspace_id() == Some(wid);
    let marker = match ws.attention {
        AttentionLevel::NeedsInput => Span::styled("⚠ ", Style::default().fg(Color::Yellow)),
        AttentionLevel::Error => Span::styled("✖ ", Style::default().fg(Color::Red)),
        _ if ws.agent_active => Span::styled(
            format!("{} ", spinner_frame(app.spinner_tick)),
            Style::default().fg(AGENT_ACTIVE),
        ),
        _ => Span::raw("  "),
    };
    // The currently-open workspace always carries a left accent bar + bold name
    // so it stays visible no matter where the keyboard cursor or focus is. The
    // bar replaces the 2-col indent, keeping every row's columns aligned.
    let indent = if active {
        Span::styled("▌ ", Style::default().fg(ACTIVE_ACCENT))
    } else {
        Span::raw("  ")
    };
    let name_style = if active {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else if ws.agent_running {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };
    let mut spans = vec![indent, marker, Span::styled(ws.name.clone(), name_style)];
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

fn spinner_frame(tick: u8) -> &'static str {
    BRAILLE_SPINNER[(tick as usize) % BRAILLE_SPINNER.len()]
}

fn visually_active(ws: &protocol::WorkspaceSummary) -> bool {
    ws.agent_active
        && !matches!(
            ws.attention,
            AttentionLevel::NeedsInput | AttentionLevel::Error
        )
}

fn repo_has_active_agent(app: &TuiApp, id: protocol::RepositoryId) -> bool {
    app.workspaces
        .iter()
        .any(|ws| ws.repository_id == Some(id) && visually_active(ws))
}

// ---------------------------------------------------------------------------
// Collapsed vertical rail + workspace pop-out
// ---------------------------------------------------------------------------

/// Builds a single rail row: `ch` centred in a `width`-wide strip, padded with
/// spaces so the (optional) background fills the whole strip.
fn rail_char_line(ch: char, width: u16, style: Style) -> Line<'static> {
    let w = width.max(1) as usize;
    let left = w.saturating_sub(1) / 2;
    let right = w.saturating_sub(1).saturating_sub(left);
    Line::from(vec![
        Span::styled(" ".repeat(left), style),
        Span::styled(ch.to_string(), style),
        Span::styled(" ".repeat(right), style),
    ])
}

/// Builds the rail's lines (each repo's name stacked one char per row, repos
/// separated by a blank row) alongside a parallel `owners` vec mapping every
/// line index to its repository index — so rendering and click hit-testing
/// stay in lockstep. The leading separator is owned by the following repo so
/// there are no dead rows.
fn rail_lines(
    app: &TuiApp,
    width: u16,
    selected: usize,
    focused: bool,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut owners: Vec<usize> = Vec::new();
    let sel_bg = if focused {
        SELECT_BG_FOCUSED
    } else {
        SELECT_BG_UNFOCUSED
    };
    for (ri, repo) in app.repositories.iter().enumerate() {
        let is_sel = ri == selected;
        let name_style = if is_sel {
            Style::default()
                .fg(Color::White)
                .bg(sel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        if ri > 0 {
            let sep = if is_sel {
                Style::default().bg(sel_bg)
            } else {
                Style::default()
            };
            lines.push(rail_char_line(' ', width, sep));
            owners.push(ri);
        }
        if repo_has_active_agent(app, repo.id) {
            let mut s = Style::default().fg(AGENT_ACTIVE);
            if is_sel {
                s = s.bg(sel_bg);
            }
            lines.push(rail_char_line(
                spinner_frame(app.spinner_tick)
                    .chars()
                    .next()
                    .unwrap_or(' '),
                width,
                s,
            ));
            owners.push(ri);
        }
        if repo.ready_for_review_count > 0 {
            let mut s = Style::default().fg(REVIEW);
            if is_sel {
                s = s.bg(sel_bg);
            }
            lines.push(rail_char_line('◆', width, s));
            owners.push(ri);
        }
        for ch in repo.name.chars() {
            lines.push(rail_char_line(ch, width, name_style));
            owners.push(ri);
        }
    }
    (lines, owners)
}

/// Scroll offset (in rows) that keeps the selected repo's run visible.
fn rail_scroll(owners: &[usize], selected: usize, inner_h: usize) -> usize {
    if owners.is_empty() || inner_h == 0 {
        return 0;
    }
    let Some(first) = owners.iter().position(|&o| o == selected) else {
        return 0;
    };
    let last = owners.iter().rposition(|&o| o == selected).unwrap_or(first);
    let mut scroll = 0usize;
    if last + 1 > inner_h {
        scroll = last + 1 - inner_h;
    }
    if first < scroll {
        scroll = first;
    }
    scroll.min(owners.len().saturating_sub(1))
}

/// Renders the collapsed vertical rail. Each repository's name is stacked one
/// character per row down a narrow strip; the selected repo is highlighted.
/// Pressing Enter on it opens the workspace pop-out (see `render_popout`).
pub fn render_rail(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let focused = app.focus == Focus::Sidebar;
    let border_style = if focused {
        Style::default().fg(Color::LightBlue)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.repositories.is_empty() {
        return;
    }
    let selected = app.rail_selected_repo_index();
    let (lines, owners) = rail_lines(app, inner.width, selected, focused);
    let scroll = rail_scroll(&owners, selected, inner.height.max(1) as usize);
    frame.render_widget(Paragraph::new(lines).scroll((scroll as u16, 0)), inner);
}

/// Maps a click within the rail `area` to a repository index, mirroring
/// `render_rail`'s inner + scroll math.
pub fn rail_repo_index_at(area: Rect, app: &TuiApp, x: u16, y: u16) -> Option<usize> {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    if x < inner.x || x >= inner.right() || y < inner.y || y >= inner.bottom() {
        return None;
    }
    if app.repositories.is_empty() {
        return None;
    }
    let selected = app.rail_selected_repo_index();
    let (_lines, owners) = rail_lines(app, inner.width, selected, false);
    let scroll = rail_scroll(&owners, selected, inner.height.max(1) as usize);
    let li = (y - inner.y) as usize + scroll;
    owners.get(li).copied()
}

/// Geometry of the workspace pop-out: a box just right of the rail, anchored at
/// the selected repo's row and clamped to the screen. `None` when none is open.
pub fn popout_rect(rail: Rect, full: Rect, app: &TuiApp) -> Option<Rect> {
    app.sidebar_popout?;
    let content_h = app.popout_workspaces().len().max(1) as u16;
    let box_h = (content_h + 2).min(full.height.max(3));
    let avail = full.right().saturating_sub(rail.right());
    let box_w = 40u16.min(avail).max(12);
    let x = rail.right();

    let inner = Block::default().borders(Borders::ALL).inner(rail);
    let selected = app.rail_selected_repo_index();
    let (_l, owners) = rail_lines(app, inner.width, selected, false);
    let scroll = rail_scroll(&owners, selected, inner.height.max(1) as usize);
    let first = owners.iter().position(|&o| o == selected).unwrap_or(0);
    let y_in_inner = (first.saturating_sub(scroll)) as u16;
    let mut y = inner.y.saturating_add(y_in_inner);
    let max_y = full.bottom().saturating_sub(box_h);
    if y > max_y {
        y = max_y;
    }
    Some(Rect::new(x, y, box_w, box_h))
}

/// Renders the workspace pop-out for the rail's open repo, listing its
/// workspaces with the normal horizontal `workspace_line` styling.
pub fn render_popout(frame: &mut Frame, rail: Rect, full: Rect, app: &TuiApp) {
    let Some(rect) = popout_rect(rail, full, app) else {
        return;
    };
    let Some(repo_id) = app.sidebar_popout else {
        return;
    };
    let name = app
        .repositories
        .iter()
        .find(|r| r.id == repo_id)
        .map(|r| r.name.clone())
        .unwrap_or_default();

    frame.render_widget(Clear, rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::LightBlue))
        .title(format!(" {name} "));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let ids = app.popout_workspaces();
    if ids.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  no workspaces",
                Style::default().fg(Color::DarkGray),
            )),
            inner,
        );
        return;
    }
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(ids.len());
    for (i, wid) in ids.iter().enumerate() {
        let mut line = workspace_line(app, *wid);
        if i == app.popout_selected {
            line = selected_row_line(line, true);
        }
        lines.push(line);
    }
    let h = inner.height.max(1) as usize;
    let scroll = app.popout_selected.saturating_sub(h.saturating_sub(1)) as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), inner);
    paint_selected_row(frame, inner, app.popout_selected, scroll, true);
}

/// Maps a click to a workspace index within the open pop-out, if inside its
/// inner content area.
pub fn popout_row_index_at(rail: Rect, full: Rect, app: &TuiApp, x: u16, y: u16) -> Option<usize> {
    let rect = popout_rect(rail, full, app)?;
    let inner = Block::default().borders(Borders::ALL).inner(rect);
    if x < inner.x || x >= inner.right() || y < inner.y || y >= inner.bottom() {
        return None;
    }
    let ids = app.popout_workspaces();
    if ids.is_empty() {
        return None;
    }
    let h = inner.height.max(1) as usize;
    let scroll = app.popout_selected.saturating_sub(h.saturating_sub(1));
    let idx = (y - inner.y) as usize + scroll;
    (idx < ids.len()).then_some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{RepositorySummary, WorkspaceSummary};
    use ratatui::{backend::TestBackend, buffer::Buffer, Terminal};
    use uuid::Uuid;

    fn repo_summary(id: protocol::RepositoryId) -> RepositorySummary {
        RepositorySummary {
            id,
            name: "conduit".to_string(),
            path: "/tmp/conduit".to_string(),
            default_branch: Some("main".to_string()),
            worktree_root: None,
            default_agent: None,
            ssh_host: None,
            workspace_count: 1,
            ready_for_review_count: 0,
        }
    }

    fn workspace_summary(
        id: protocol::WorkspaceId,
        repo_id: protocol::RepositoryId,
    ) -> WorkspaceSummary {
        WorkspaceSummary {
            id,
            name: "ui-in-progress".to_string(),
            path: "/tmp/conduit/.conduit-worktrees/conduit/ui-in-progress".to_string(),
            branch: Some("ui-in-progress".to_string()),
            ahead: Some(0),
            behind: Some(0),
            dirty_files: 0,
            attention: AttentionLevel::None,
            agent_running: true,
            agent_active: false,
            shell_running: false,
            last_activity_unix_ms: 0,
            ssh_host: None,
            repository_id: Some(repo_id),
            base_branch: Some("main".to_string()),
            ready_for_review: false,
            agent: None,
        }
    }

    fn app_with_workspace() -> (TuiApp, protocol::RepositoryId, protocol::WorkspaceId) {
        let repo_id = Uuid::new_v4();
        let ws_id = Uuid::new_v4();
        let mut app = TuiApp::default();
        app.spinner_tick = 2;
        app.set_repositories(vec![repo_summary(repo_id)]);
        app.set_workspaces(vec![workspace_summary(ws_id, repo_id)]);
        (app, repo_id, ws_id)
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn render_sidebar_buffer(app: &TuiApp) -> (Buffer, Rect) {
        let area = Rect::new(0, 0, WIDTH, 6);
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let backend = TestBackend::new(area.width, area.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, area, app);
            })
            .unwrap();
        (terminal.backend().buffer().clone(), inner)
    }

    fn render_popout_buffer(app: &TuiApp) -> (Buffer, Rect) {
        let full = Rect::new(0, 0, 80, 12);
        let rail = Rect::new(0, 0, RAIL_WIDTH, full.height);
        let popout = popout_rect(rail, full, app).unwrap();
        let inner = Block::default().borders(Borders::ALL).inner(popout);
        let backend = TestBackend::new(full.width, full.height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_popout(frame, rail, full, app);
            })
            .unwrap();
        (terminal.backend().buffer().clone(), inner)
    }

    fn assert_row_bg(buffer: &Buffer, inner: Rect, row_offset: u16, expected: Color) {
        let y = inner.y + row_offset;
        for x in inner.left()..inner.right() {
            assert_eq!(buffer[(x, y)].bg, expected, "x={x}, y={y}");
        }
        assert_eq!(buffer[(inner.right() - 1, y)].symbol(), " ");
    }

    fn assert_cell_fg(buffer: &Buffer, x: u16, y: u16, expected: Color) {
        assert_eq!(buffer[(x, y)].fg, expected, "x={x}, y={y}");
    }

    #[test]
    fn active_workspace_shows_spinner_in_status_slot() {
        let (mut app, _repo_id, ws_id) = app_with_workspace();
        app.workspaces[0].agent_active = true;

        let line = workspace_line(&app, ws_id);

        assert_eq!(line.spans[1].content.as_ref(), "⠹ ");
        assert_eq!(line.spans[1].style.fg, Some(AGENT_ACTIVE));
        assert!(text(&line).contains("⠹ ui-in-progress"));
    }

    #[test]
    fn attention_overrides_workspace_spinner() {
        for (attention, marker, color) in [
            (AttentionLevel::NeedsInput, "⚠ ", Color::Yellow),
            (AttentionLevel::Error, "✖ ", Color::Red),
        ] {
            let (mut app, _repo_id, ws_id) = app_with_workspace();
            app.workspaces[0].agent_active = true;
            app.workspaces[0].attention = attention;

            let line = workspace_line(&app, ws_id);

            assert_eq!(line.spans[1].content.as_ref(), marker);
            assert_eq!(line.spans[1].style.fg, Some(color));
            assert!(!text(&line).contains('⠹'));
        }
    }

    #[test]
    fn repository_and_rail_roll_up_active_children() {
        let (mut app, repo_id, _ws_id) = app_with_workspace();
        app.workspaces[0].agent_active = true;

        let repo = repo_line(&app, repo_id);
        assert!(text(&repo).contains("main  ⠹"));

        let (lines, owners) = rail_lines(&app, 3, 0, false);
        assert_eq!(owners.first().copied(), Some(0));
        assert_eq!(text(&lines[0]).trim(), "⠹");
        assert_eq!(lines[0].spans[1].style.fg, Some(AGENT_ACTIVE));
    }

    #[test]
    fn expanded_sidebar_selected_repo_fills_visible_row_background() {
        let (mut app, _repo_id, _ws_id) = app_with_workspace();
        app.sidebar_selected = 0;

        let (buffer, inner) = render_sidebar_buffer(&app);

        assert_row_bg(&buffer, inner, 0, ROW_SELECT_BG_FOCUSED);
        assert_cell_fg(&buffer, inner.x + 2, inner.y, row_selected_fg(true));
    }

    #[test]
    fn expanded_sidebar_selected_workspace_fills_visible_row_background() {
        let (mut app, _repo_id, _ws_id) = app_with_workspace();
        app.sidebar_selected = 1;

        let (buffer, inner) = render_sidebar_buffer(&app);

        assert_row_bg(&buffer, inner, 1, ROW_SELECT_BG_FOCUSED);
        assert_cell_fg(&buffer, inner.x + 4, inner.y + 1, row_selected_fg(true));
    }

    #[test]
    fn selected_workspace_preserves_attention_marker_color() {
        let (mut app, _repo_id, _ws_id) = app_with_workspace();
        app.sidebar_selected = 1;
        app.workspaces[0].attention = AttentionLevel::NeedsInput;

        let (buffer, inner) = render_sidebar_buffer(&app);

        assert_row_bg(&buffer, inner, 1, ROW_SELECT_BG_FOCUSED);
        assert_cell_fg(&buffer, inner.x + 2, inner.y + 1, Color::Yellow);
    }

    #[test]
    fn popout_selected_workspace_fills_row_with_focused_background() {
        let (mut app, repo_id, _ws_id) = app_with_workspace();
        app.sidebar_popout = Some(repo_id);
        app.focus = Focus::WsTerminal;

        let (buffer, inner) = render_popout_buffer(&app);

        assert_row_bg(&buffer, inner, 0, ROW_SELECT_BG_FOCUSED);
        assert_cell_fg(&buffer, inner.x + 4, inner.y, row_selected_fg(true));
    }
}
