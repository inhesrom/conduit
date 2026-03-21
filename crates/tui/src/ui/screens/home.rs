use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::TuiApp;
use crate::ui::footer;
use crate::ui::widgets::tile_grid;
use tile_grid::ORANGE;

/// Renders the home screen: dashboard header, tile grid, footer, and any open modals.
pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let chunks = home_chunks(area);
    render_dashboard(frame, chunks[0], app);

    let grid_area = chunks[1];
    let body_max = (grid_area.width as usize).saturating_sub(6);
    let expanded = &app.home_expanded_tiles;
    let expanded_h = tile_grid::tile_h_expanded(app.settings.preview_lines);
    let preview_lines = app.settings.preview_lines;
    let tile_data: Vec<tile_grid::TileData> = app
        .workspaces
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let num_lines = if expanded.contains(&i) {
                preview_lines
            } else {
                0 // collapsed tiles don't show preview
            };
            tile_grid::TileData {
                summary: ws,
                preview: if num_lines > 0 {
                    app.tile_preview_lines(ws.id, body_max as u16, num_lines)
                } else {
                    Vec::new()
                },
            }
        })
        .collect();

    tile_grid::render(
        frame,
        grid_area,
        &tile_data,
        app.home_selected,
        app.spinner_tick % 2 == 0,
        app.settings.attention_notifications,
        expanded,
        expanded_h,
        app.home_scroll_offset,
    );
    footer::render(frame, chunks[2], app);
    render_modals(frame, area, app);
}

/// Renders the rounded dashboard box with anvil ASCII art and colored status badges.
fn render_dashboard(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let needs_input = app
        .workspaces
        .iter()
        .filter(|w| matches!(w.attention, protocol::AttentionLevel::NeedsInput))
        .count();
    let errors = app
        .workspaces
        .iter()
        .filter(|w| matches!(w.attention, protocol::AttentionLevel::Error))
        .count();
    let dirty = app.workspaces.iter().map(|w| w.dirty_files).sum::<usize>();
    let running_agents = app.workspaces.iter().filter(|w| w.agent_running).count();

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

    let art_lines: Vec<Line> = vec![Line::from(""), Line::from(badge_spans)];

    let dashboard = Paragraph::new(art_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title_top(Line::from(Span::styled(
                " ANVL ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))),
    );
    frame.render_widget(dashboard, area);
}

/// Builds a styled icon+count badge span pair for the dashboard header.
/// Returns dimmed spans when `count` is zero so the layout stays stable.
fn dashboard_badge(count: usize, icon: &str, label: &str, color: Color) -> Vec<Span<'static>> {
    let dim = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::DIM);
    if count > 0 {
        vec![
            Span::styled(
                format!("{} {} ", icon, count),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{}     ", label),
                Style::default().fg(Color::DarkGray),
            ),
        ]
    } else {
        vec![
            Span::styled(format!("{} {} ", icon, count), dim),
            Span::styled(format!("{}     ", label), dim),
        ]
    }
}

/// Renders the add-workspace and delete-confirmation modals when active.
fn render_modals(frame: &mut Frame, area: Rect, app: &TuiApp) {
    if let Some(browser) = &app.dir_browser {
        let modal = centered_rect(area, 70, 20);
        frame.render_widget(Clear, modal);

        let outer_block = Block::default()
            .title(" Browse Directory ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = outer_block.inner(modal);
        frame.render_widget(outer_block, modal);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(inner);

        // Path input section
        let path_style = if browser.editing_path {
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let path_display = if browser.editing_path {
            format!("{}_", browser.path_input)
        } else {
            browser.path_input.clone()
        };
        let path_widget = Paragraph::new(path_display).block(
            Block::default()
                .title(" Path ")
                .borders(Borders::ALL)
                .border_style(path_style),
        );
        frame.render_widget(path_widget, sections[0]);

        // Directory listing section
        if browser.entries.is_empty() {
            let empty = Paragraph::new(Line::from(Span::styled(
                "(no subdirectories)",
                Style::default().fg(Color::DarkGray),
            )))
            .alignment(Alignment::Center);
            frame.render_widget(empty, sections[1]);
        } else {
            let items: Vec<ListItem> = browser
                .entries
                .iter()
                .map(|name| ListItem::new(format!("  {}/", name)))
                .collect();
            let list = List::new(items).highlight_symbol("> ").highlight_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
            let mut list_state = ListState::default();
            list_state.select(Some(browser.selected));
            frame.render_stateful_widget(list, sections[1], &mut list_state);
        }

        // Hint bar section
        let key_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::DarkGray);
        let hints = if browser.editing_path {
            Line::from(vec![
                Span::styled("Tab", key_style),
                Span::styled(" complete  ", desc_style),
                Span::styled("Enter", key_style),
                Span::styled(" browse  ", desc_style),
                Span::styled("Esc", key_style),
                Span::styled(" cancel", desc_style),
            ])
        } else {
            Line::from(vec![
                Span::styled("j/k", key_style),
                Span::styled(" nav  ", desc_style),
                Span::styled("Enter", key_style),
                Span::styled(" open ws  ", desc_style),
                Span::styled("Bksp", key_style),
                Span::styled(" up  ", desc_style),
                Span::styled(".", key_style),
                Span::styled(" hidden  ", desc_style),
                Span::styled("/", key_style),
                Span::styled(" edit path  ", desc_style),
                Span::styled("Tab", key_style),
                Span::styled(" open child  ", desc_style),
                Span::styled("Space", key_style),
                Span::styled(" select child", desc_style),
            ])
        };
        frame.render_widget(Paragraph::new(vec![Line::from(""), hints]), sections[2]);
    }

    if let Some(ref picker) = app.ssh_history_picker {
        let entry_count = app.ssh_history.len();
        let modal_height = (entry_count as u16 + 5).min(20);
        let modal = centered_rect(area, 60, modal_height);
        frame.render_widget(Clear, modal);

        let outer_block = Block::default()
            .title(" Recent SSH Workspaces ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = outer_block.inner(modal);
        frame.render_widget(outer_block, modal);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(inner);

        let items: Vec<ListItem> = app
            .ssh_history
            .iter()
            .map(|entry| {
                let label = if let Some(ref user) = entry.user {
                    format!("  {}@{}:{}", user, entry.host, entry.path)
                } else {
                    format!("  {}:{}", entry.host, entry.path)
                };
                ListItem::new(label)
            })
            .collect();

        let list = List::new(items).highlight_symbol("> ").highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        let mut list_state = ListState::default();
        list_state.select(Some(picker.selected));
        frame.render_stateful_widget(list, sections[0], &mut list_state);

        let key_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::DarkGray);
        let hints = Line::from(vec![
            Span::styled("j/k", key_style),
            Span::styled(" nav  ", desc_style),
            Span::styled("Enter", key_style),
            Span::styled(" select  ", desc_style),
            Span::styled("n", key_style),
            Span::styled(" new  ", desc_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel", desc_style),
        ]);
        frame.render_widget(Paragraph::new(vec![Line::from(""), hints]), sections[1]);
    }

    if let Some(ref ssh_input) = app.ssh_workspace_input {
        let modal = centered_rect(area, 60, 14);
        frame.render_widget(Clear, modal);

        let outer_block = Block::default()
            .title(" Add SSH Workspace ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan));
        let inner = outer_block.inner(modal);
        frame.render_widget(outer_block, modal);

        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .split(inner);

        let fields = [
            ("Host", &ssh_input.host, crate::app::SshField::Host),
            ("User", &ssh_input.user, crate::app::SshField::User),
            ("Path", &ssh_input.path, crate::app::SshField::Path),
        ];

        for (i, (label, value, field)) in fields.iter().enumerate() {
            let is_focused = ssh_input.focused_field == *field;
            let border_style = if is_focused {
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let display = if is_focused {
                format!("{}_", value)
            } else {
                value.to_string()
            };
            let widget = Paragraph::new(display).block(
                Block::default()
                    .title(format!(" {} ", label))
                    .borders(Borders::ALL)
                    .border_style(border_style),
            );
            frame.render_widget(widget, sections[i]);
        }

        let key_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::DarkGray);
        let hints = Line::from(vec![
            Span::styled("Tab", key_style),
            Span::styled(" next field  ", desc_style),
            Span::styled("Enter", key_style),
            Span::styled(" add  ", desc_style),
            Span::styled("Esc", key_style),
            Span::styled(" cancel", desc_style),
        ]);
        frame.render_widget(Paragraph::new(vec![hints]), sections[3]);
    }

    if let Some(id) = app.pending_delete_workspace {
        let name = app
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.name.clone())
            .unwrap_or_else(|| id.to_string());
        let modal = centered_rect(area, 56, 7);
        frame.render_widget(Clear, modal);
        frame.render_widget(
            Paragraph::new(format!("Delete workspace?\n\n{}", name))
                .alignment(Alignment::Left)
                .block(
                    Block::default()
                        .title("Confirm Delete")
                        .borders(Borders::ALL),
                ),
            modal,
        );
    }

    if app.is_renaming_workspace() {
        if let Some(name) = &app.rename_workspace_input {
            let modal = centered_rect(area, 56, 5);
            frame.render_widget(Clear, modal);
            frame.render_widget(
                Paragraph::new(format!("{name}_")).block(
                    Block::default()
                        .title("Rename Workspace (Enter to confirm, Esc to cancel)")
                        .borders(Borders::ALL)
                        .border_style(
                            Style::default()
                                .fg(Color::LightBlue)
                                .add_modifier(Modifier::BOLD),
                        )
                        .border_type(BorderType::Thick),
                ),
                modal,
            );
        }
    }

    if app.is_settings_open() {
        let modal = centered_rect(area, 50, 10);
        frame.render_widget(Clear, modal);

        let key_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::DarkGray);
        let cursor_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);

        let cursor0 = if app.settings_selected == 0 {
            "> "
        } else {
            "  "
        };
        let toggle = render_toggle(app.settings.attention_notifications);
        let row0 = Line::from(vec![
            Span::styled(cursor0, cursor_style),
            Span::raw("Attention notifications   "),
            toggle,
        ]);

        let cursor1 = if app.settings_selected == 1 {
            "> "
        } else {
            "  "
        };
        let row1 = Line::from(vec![
            Span::styled(cursor1, cursor_style),
            Span::raw("Preview lines             "),
            Span::styled(
                format!("\u{25C2} {} \u{25B8}", app.settings.preview_lines),
                Style::default().fg(Color::Cyan),
            ),
        ]);

        let hint = Line::from(vec![
            Span::styled("j/k", key_style),
            Span::styled(" navigate  ", desc_style),
            Span::styled("Space", key_style),
            Span::styled(" toggle  ", desc_style),
            Span::styled("h/l", key_style),
            Span::styled(" adjust  ", desc_style),
            Span::styled("Esc", key_style),
            Span::styled(" close", desc_style),
        ]);
        let body = vec![
            Line::from(""),
            row0,
            row1,
            Line::from(""),
            Line::from(""),
            hint,
        ];

        frame.render_widget(
            Paragraph::new(body).block(
                Block::default()
                    .title(" Settings ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Cyan)),
            ),
            modal,
        );
    }
}

fn render_toggle(enabled: bool) -> Span<'static> {
    if enabled {
        Span::styled(
            "━━● ON ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("OFF ●━━", Style::default().fg(Color::DarkGray))
    }
}

/// Returns a centered rectangle within `area` at `width_pct` width and fixed `height`.
fn centered_rect(area: Rect, width_pct: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(height),
            Constraint::Min(1),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .split(vertical[1]);
    horizontal[1]
}

/// Returns the rectangle used by the add-workspace modal.
pub fn add_modal_rect(area: Rect) -> Rect {
    centered_rect(area, 70, 20)
}

/// Returns the rectangle used by the delete-confirmation modal.
pub fn delete_modal_rect(area: Rect) -> Rect {
    centered_rect(area, 56, 7)
}

/// Returns the rectangle occupied by the tile grid on the home screen.
pub fn grid_rect(area: Rect) -> Rect {
    home_chunks(area)[1]
}

/// Returns the `Rect` of the pane containing the given point, if any.
pub fn pane_rect_at(area: Rect, app: &TuiApp, x: u16, y: u16) -> Option<Rect> {
    let chunks = home_chunks(area);
    let point_inside = |r: Rect| x >= r.x && y >= r.y && x < r.right() && y < r.bottom();

    if point_inside(chunks[0]) {
        return Some(chunks[0]);
    }
    if point_inside(chunks[2]) {
        return Some(chunks[2]);
    }

    // Check individual tile rects (use index_at for scroll-aware hit testing)
    let grid_area = chunks[1];
    let expanded_h = tile_grid::tile_h_expanded(app.settings.preview_lines);
    if let Some(i) = tile_grid::index_at(
        grid_area,
        x,
        y,
        app.workspaces.len(),
        &app.home_expanded_tiles,
        expanded_h,
        app.home_scroll_offset,
    ) {
        let r = tile_grid::tile_rect(
            grid_area,
            i,
            &app.home_expanded_tiles,
            expanded_h,
            app.home_scroll_offset,
        );
        if r.width > 0 && r.height > 0 {
            return Some(r);
        }
    }

    // Fall back to the whole grid area
    if point_inside(grid_area) {
        return Some(grid_area);
    }
    None
}

/// Splits the home screen area into dashboard header, grid, and footer chunks.
fn home_chunks(area: Rect) -> Vec<Rect> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area)
        .to_vec()
}

/// Returns the outer `Rect` of every bordered pane on the home screen.
///
/// Used during text extraction so that border cells can be replaced with spaces.
pub fn border_rects(area: Rect, app: &TuiApp) -> Vec<Rect> {
    let chunks = home_chunks(area);
    let mut rects = vec![
        chunks[0], // dashboard header (Borders::ALL + Rounded)
        chunks[2], // footer (Borders::TOP)
    ];

    // Individual tile rects from the grid
    let grid_area = chunks[1];
    let expanded_h = tile_grid::tile_h_expanded(app.settings.preview_lines);
    for i in 0..app.workspaces.len() {
        let r = tile_grid::tile_rect(
            grid_area,
            i,
            &app.home_expanded_tiles,
            expanded_h,
            app.home_scroll_offset,
        );
        if r.width > 0 && r.height > 0 {
            rects.push(r);
        }
    }

    rects
}

#[cfg(test)]
mod tests {
    use crate::app::TuiApp;
    use protocol::{AttentionLevel, WorkspaceSummary};
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

    fn smoke_render_home(app: &TuiApp, width: u16, height: u16) {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                super::render(frame, area, app);
            })
            .unwrap();
    }

    #[test]
    fn render_home_empty_workspaces() {
        let app = TuiApp::default();
        smoke_render_home(&app, 120, 40);
    }

    #[test]
    fn render_home_with_workspaces() {
        let mut app = TuiApp::default();
        app.set_workspaces(vec![make_ws("alpha"), make_ws("beta"), make_ws("gamma")]);
        smoke_render_home(&app, 120, 40);
    }

    #[test]
    fn render_home_with_delete_modal() {
        let mut app = TuiApp::default();
        let ws = make_ws("doomed");
        let id = ws.id;
        app.set_workspaces(vec![ws]);
        app.pending_delete_workspace = Some(id);
        smoke_render_home(&app, 120, 40);
    }

    #[test]
    fn render_home_with_settings_modal() {
        let mut app = TuiApp::default();
        app.settings_open = true;
        smoke_render_home(&app, 120, 40);
    }

    #[test]
    fn render_home_very_small_terminal() {
        let app = TuiApp::default();
        smoke_render_home(&app, 20, 10);
    }

    // --- border_rects tests ---

    #[test]
    fn border_rects_includes_dashboard_and_footer() {
        let app = TuiApp::default();
        let area = ratatui::layout::Rect::new(0, 0, 120, 40);
        let rects = super::border_rects(area, &app);
        // At minimum: dashboard header + footer = 2
        assert!(rects.len() >= 2, "got {} rects", rects.len());
    }

    #[test]
    fn border_rects_includes_tiles() {
        let mut app = TuiApp::default();
        app.set_workspaces(vec![make_ws("a"), make_ws("b"), make_ws("c")]);
        let area = ratatui::layout::Rect::new(0, 0, 120, 40);
        let rects = super::border_rects(area, &app);
        // dashboard + footer + 3 tiles = 5
        assert!(rects.len() >= 5, "got {} rects", rects.len());
    }
}
