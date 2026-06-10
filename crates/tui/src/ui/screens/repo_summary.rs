//! The repository status summary shown in the detail pane (`Route::Repo`).
//! Lists every workspace belonging to one repository with its live status, and
//! is navigable: the selected row can be opened into a full workspace view.

use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use protocol::{AttentionLevel, RepositorySummary, WorkspaceSummary};

use crate::app::TuiApp;
use crate::ui::footer;
use crate::ui::screens::home::dashboard_badge;
use crate::ui::widgets::sidebar::{spinner_frame, AGENT_ACTIVE, REVIEW};
use crate::ui::widgets::tile_grid::ORANGE;

/// Renders the repo summary: a header with aggregate badges, the navigable
/// workspace list, and the footer.
pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = summary_chunks(area);

    let repo = app
        .repo_summary_repo_id()
        .and_then(|id| app.repositories.iter().find(|r| r.id == id));
    let workspaces: Vec<&WorkspaceSummary> = match repo {
        Some(repo) => app
            .workspaces
            .iter()
            .filter(|w| w.repository_id == Some(repo.id))
            .collect(),
        None => Vec::new(),
    };

    render_header(frame, chunks[0], repo, &workspaces);
    render_workspaces(frame, chunks[1], app, &workspaces);
    footer::render(frame, chunks[2], app);
}

/// Header box: repo name (title), path, counts, and status badges aggregated
/// over this repo's workspaces.
fn render_header(
    frame: &mut Frame,
    area: Rect,
    repo: Option<&RepositorySummary>,
    workspaces: &[&WorkspaceSummary],
) {
    let desc = Style::default().fg(Color::DarkGray);

    let needs_input = workspaces
        .iter()
        .filter(|w| matches!(w.attention, AttentionLevel::NeedsInput))
        .count();
    let errors = workspaces
        .iter()
        .filter(|w| matches!(w.attention, AttentionLevel::Error))
        .count();
    let dirty = workspaces.iter().map(|w| w.dirty_files).sum::<usize>();
    let running_agents = workspaces.iter().filter(|w| w.agent_running).count();
    let ready = workspaces.iter().filter(|w| w.ready_for_review).count();

    let mut badge_spans = Vec::new();
    badge_spans.extend(dashboard_badge(needs_input, "\u{26A0}", "input", ORANGE));
    badge_spans.extend(dashboard_badge(errors, "\u{2716}", "error", Color::Red));
    badge_spans.extend(dashboard_badge(dirty, "\u{25C8}", "changes", Color::Yellow));
    badge_spans.extend(dashboard_badge(
        running_agents,
        "\u{25CF}",
        "agents",
        Color::Green,
    ));
    badge_spans.extend(dashboard_badge(ready, "\u{25C6}", "ready", REVIEW));

    let (title, lines) = match repo {
        Some(repo) => {
            let title = format!(" {} ", repo.name);
            let lines = vec![
                Line::from(Span::styled(format!("  {}", repo.path), desc)),
                Line::from(Span::styled(
                    format!(
                        "  default branch: {}    workspaces: {}",
                        repo.default_branch.as_deref().unwrap_or("?"),
                        repo.workspace_count
                    ),
                    desc,
                )),
                Line::from(badge_spans),
            ];
            (title, lines)
        }
        None => (
            " Repository ".to_string(),
            vec![Line::from(Span::styled("  (repository not found)", desc))],
        ),
    };

    let header = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title_top(Line::from(Span::styled(
                title,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))),
    );
    frame.render_widget(header, area);
}

/// The navigable list of workspaces with per-row status, or an empty-state hint.
fn render_workspaces(
    frame: &mut Frame,
    area: Rect,
    app: &TuiApp,
    workspaces: &[&WorkspaceSummary],
) {
    let block = Block::default()
        .title(" Workspaces ")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);

    if workspaces.is_empty() {
        let hint = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "No workspaces yet — press n to create one.",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .alignment(Alignment::Center)
        .block(block);
        frame.render_widget(hint, area);
        return;
    }

    let items: Vec<ListItem> = workspaces
        .iter()
        .map(|ws| ListItem::new(workspace_row(app, ws)))
        .collect();

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let mut state = ListState::default();
    let selected = app
        .repo_summary_selected
        .min(workspaces.len().saturating_sub(1));
    state.select(Some(selected));
    frame.render_stateful_widget(list, area, &mut state);
}

/// One workspace row: status marker, name, branch, ahead/behind, dirty count,
/// and the ready-for-review badge.
fn workspace_row(app: &TuiApp, ws: &WorkspaceSummary) -> Line<'static> {
    let marker = match ws.attention {
        AttentionLevel::NeedsInput => Span::styled("⚠ ", Style::default().fg(Color::Yellow)),
        AttentionLevel::Error => Span::styled("✖ ", Style::default().fg(Color::Red)),
        _ if ws.agent_active => Span::styled(
            format!("{} ", spinner_frame(app.spinner_tick)),
            Style::default().fg(AGENT_ACTIVE),
        ),
        _ => Span::raw("  "),
    };

    let name_style = if ws.agent_running {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    };

    let mut spans = vec![
        Span::raw("  "),
        marker,
        Span::styled(ws.name.clone(), name_style),
    ];

    if let Some(branch) = ws.branch.as_deref() {
        spans.push(Span::styled(
            format!("  {branch}"),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let ahead = ws.ahead.unwrap_or(0);
    let behind = ws.behind.unwrap_or(0);
    if ahead > 0 {
        spans.push(Span::styled(
            format!(" ↑{ahead}"),
            Style::default().fg(Color::Green),
        ));
    }
    if behind > 0 {
        spans.push(Span::styled(
            format!(" ↓{behind}"),
            Style::default().fg(Color::Red),
        ));
    }

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

/// Splits the summary area into header, workspace list, and footer chunks.
fn summary_chunks(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(area)
        .to_vec()
}

/// Outer `Rect` of every bordered widget the summary draws — used during
/// clipboard text extraction so border cells become spaces.
pub fn border_rects(area: Rect) -> Vec<Rect> {
    let chunks = summary_chunks(area);
    vec![chunks[0], chunks[1], chunks[2]]
}

/// Returns the `summary_chunks` section (header, list, or footer) containing the
/// given point, if any. Used to confine a mouse selection to one section.
pub fn chunk_at(area: Rect, x: u16, y: u16) -> Option<Rect> {
    summary_chunks(area)
        .into_iter()
        .find(|r| x >= r.x && y >= r.y && x < r.right() && y < r.bottom())
}

#[cfg(test)]
mod tests {
    use crate::app::TuiApp;
    use protocol::{AttentionLevel, RepositorySummary, WorkspaceSummary};
    use uuid::Uuid;

    fn make_repo(id: Uuid, name: &str) -> RepositorySummary {
        RepositorySummary {
            id,
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            default_branch: Some("main".into()),
            worktree_root: None,
            default_agent: None,
            ssh_host: None,
            workspace_count: 0,
            ready_for_review_count: 0,
        }
    }

    fn make_ws(name: &str, repo: Uuid) -> WorkspaceSummary {
        WorkspaceSummary {
            id: Uuid::new_v4(),
            name: name.to_string(),
            path: format!("/tmp/{name}"),
            branch: Some("feature".into()),
            ahead: Some(2),
            behind: Some(1),
            dirty_files: 3,
            attention: AttentionLevel::None,
            agent_running: true,
            agent_active: false,
            shell_running: false,
            last_activity_unix_ms: 0,
            ssh_host: None,
            repository_id: Some(repo),
            base_branch: Some("main".into()),
            ready_for_review: true,
            agent: None,
        }
    }

    fn smoke_render(app: &TuiApp, width: u16, height: u16) {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| super::render(frame, frame.area(), app))
            .unwrap();
    }

    #[test]
    fn render_repo_summary_with_workspaces() {
        let mut app = TuiApp::default();
        let repo_id = Uuid::new_v4();
        app.set_repositories(vec![make_repo(repo_id, "conduit")]);
        app.set_workspaces(vec![
            make_ws("alpha", repo_id),
            make_ws("beta", repo_id),
        ]);
        app.open_repo_summary(repo_id);
        smoke_render(&app, 120, 40);
    }

    #[test]
    fn render_repo_summary_empty() {
        let mut app = TuiApp::default();
        let repo_id = Uuid::new_v4();
        app.set_repositories(vec![make_repo(repo_id, "conduit")]);
        app.open_repo_summary(repo_id);
        smoke_render(&app, 120, 40);
    }

    #[test]
    fn render_repo_summary_very_small_terminal() {
        let mut app = TuiApp::default();
        let repo_id = Uuid::new_v4();
        app.set_repositories(vec![make_repo(repo_id, "conduit")]);
        app.open_repo_summary(repo_id);
        smoke_render(&app, 20, 8);
    }

    #[test]
    fn summary_navigation_filters_clamps_and_opens() {
        let mut app = TuiApp::default();
        let repo_id = Uuid::new_v4();
        let other_repo = Uuid::new_v4();
        app.set_repositories(vec![make_repo(repo_id, "conduit")]);
        let alpha = make_ws("alpha", repo_id);
        let beta = make_ws("beta", repo_id);
        let alpha_id = alpha.id;
        let beta_id = beta.id;
        // A workspace in another repo must be excluded from the summary.
        app.set_workspaces(vec![alpha, beta, make_ws("other", other_repo)]);
        app.open_repo_summary(repo_id);

        assert_eq!(app.repo_summary_workspace_ids(), vec![alpha_id, beta_id]);
        assert_eq!(app.selected_repo_summary_workspace_id(), Some(alpha_id));

        app.move_repo_summary_selection(1);
        assert_eq!(app.selected_repo_summary_workspace_id(), Some(beta_id));
        // Moving past the end clamps to the last row.
        app.move_repo_summary_selection(5);
        assert_eq!(app.selected_repo_summary_workspace_id(), Some(beta_id));

        // If the list shrinks below the selection, Enter still targets a valid row.
        app.set_workspaces(vec![make_ws("only", repo_id)]);
        assert!(app.selected_repo_summary_workspace_id().is_some());
    }
}
