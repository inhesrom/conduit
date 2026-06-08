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
    render_welcome_body(frame, chunks[1], app);
    footer::render(frame, chunks[2], app);
    render_modals(frame, area, app);
}

/// The detail pane shown while the Sidebar is focused (no workspace open):
/// a short keymap and details of the sidebar-selected item.
fn render_welcome_body(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(Color::DarkGray);
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Select a workspace in the sidebar, Enter to open.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  n", key),
            Span::styled(" new workspace    ", desc),
            Span::styled("N", key),
            Span::styled(" add repository    ", desc),
            Span::styled("ctrl+b", key),
            Span::styled(" toggle sidebar", desc),
        ]),
        Line::from(vec![
            Span::styled("  Enter", key),
            Span::styled(" open    ", desc),
            Span::styled("R", key),
            Span::styled(" review    ", desc),
            Span::styled("Space", key),
            Span::styled(" mark ready    ", desc),
            Span::styled("D", key),
            Span::styled(" delete    ", desc),
            Span::styled("f", key),
            Span::styled(" filter ready", desc),
        ]),
        Line::from(""),
    ];

    match app.selected_sidebar_row() {
        Some(crate::app::SidebarRow::Repo(id)) => {
            if let Some(repo) = app.repositories.iter().find(|r| r.id == id) {
                lines.push(Line::from(Span::styled(
                    format!("  Repository: {}", repo.name),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(format!("  {}", repo.path), desc)));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  default branch: {}    workspaces: {}",
                        repo.default_branch.as_deref().unwrap_or("?"),
                        repo.workspace_count
                    ),
                    desc,
                )));
            }
        }
        Some(crate::app::SidebarRow::Workspace(wid)) => {
            if let Some(ws) = app.workspaces.iter().find(|w| w.id == wid) {
                lines.push(Line::from(Span::styled(
                    format!("  Workspace: {}", ws.name),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  branch: {}", ws.branch.as_deref().unwrap_or("?")),
                    desc,
                )));
                if ws.ready_for_review {
                    lines.push(Line::from(Span::styled(
                        "  ◆ ready for review",
                        Style::default().fg(Color::Magenta),
                    )));
                }
            }
        }
        None => {}
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Conduit ");
    frame.render_widget(Paragraph::new(lines).block(block), area);
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
                " CONDUIT",
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
    // The quick-create modal is drawn route-independently by the caller (see
    // `render_quick_create_modal`) so a workspace can be created from the
    // sidebar while another workspace is open.
    if app.quick_create.is_some() {
        return;
    }
    if let Some(browser) = &app.dir_browser {
        let modal = centered_rect(area, 70, 20);
        frame.render_widget(Clear, modal);

        let outer_block = Block::default()
            .title(" Add Repository ")
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
                Span::styled(" add repo  ", desc_style),
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

    // The delete-confirmation modal is drawn route-independently by the caller
    // (see `render_delete_modal`) so it works while a workspace is open too.

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
        let modal = centered_rect(area, 56, 19);
        frame.render_widget(Clear, modal);

        let key_style = Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::DarkGray);
        let cursor_style = Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        let edit_style = Style::default().fg(Color::Yellow);
        let sub_label_style = Style::default().fg(Color::DarkGray);

        let cursor_str = |idx: usize| -> &'static str {
            if app.settings_selected == idx {
                "> "
            } else {
                "  "
            }
        };

        let active_agent = app.settings.active_agent();

        // Row 0: Default agent (cycle with h/l)
        let agent_name = active_agent.map(|a| a.name.as_str()).unwrap_or("(none)");
        let row0 = Line::from(vec![
            Span::styled(cursor_str(0), cursor_style),
            Span::raw("Default agent             "),
            Span::styled(
                format!("\u{25C2} {} \u{25B8}", agent_name),
                Style::default().fg(Color::Cyan),
            ),
        ]);

        // Row 1: Agent command (editable, shows active agent's command)
        let cmd_display = active_agent.map(|a| a.command.as_str()).unwrap_or("");
        let cmd_val = if app.settings_selected == 1 {
            if let Some(buf) = &app.settings_edit_buffer {
                Span::styled(format!("{}▏", buf), edit_style)
            } else {
                Span::styled(cmd_display.to_string(), Style::default().fg(Color::Cyan))
            }
        } else {
            Span::styled(cmd_display.to_string(), Style::default().fg(Color::Cyan))
        };
        let row1 = Line::from(vec![
            Span::styled(cursor_str(1), cursor_style),
            Span::styled("  Command                 ", sub_label_style),
            cmd_val,
        ]);

        // Row 2: Agent YOLO flags (editable, shows active agent's yolo flags)
        let yolo_display = active_agent
            .map(|a| a.yolo_flags.join(" "))
            .unwrap_or_default();
        let yolo_val = if app.settings_selected == 2 {
            if let Some(buf) = &app.settings_edit_buffer {
                Span::styled(format!("{}▏", buf), edit_style)
            } else {
                Span::styled(yolo_display.clone(), Style::default().fg(Color::Cyan))
            }
        } else {
            Span::styled(yolo_display, Style::default().fg(Color::Cyan))
        };
        let row2 = Line::from(vec![
            Span::styled(cursor_str(2), cursor_style),
            Span::styled("  YOLO flags              ", sub_label_style),
            yolo_val,
        ]);

        // Row 3: Attention notifications
        let toggle = render_toggle(app.settings.attention_notifications);
        let row3 = Line::from(vec![
            Span::styled(cursor_str(3), cursor_style),
            Span::raw("Attention notifications   "),
            toggle,
        ]);

        // Row 4: Preview lines
        let row4 = Line::from(vec![
            Span::styled(cursor_str(4), cursor_style),
            Span::raw("Preview lines             "),
            Span::styled(
                format!("\u{25C2} {} \u{25B8}", app.settings.preview_lines),
                Style::default().fg(Color::Cyan),
            ),
        ]);

        // Row 5: Show frame counter
        let fc_toggle = render_toggle(app.settings.show_frame_counter);
        let row5 = Line::from(vec![
            Span::styled(cursor_str(5), cursor_style),
            Span::raw("Show frame counter        "),
            fc_toggle,
        ]);

        let keybind_val = |idx: usize, current: &str| -> Span<'static> {
            if app.settings_selected == idx && app.is_editing_keybind() {
                Span::styled("Press any key (Esc cancels)…".to_string(), edit_style)
            } else {
                Span::styled(current.to_string(), Style::default().fg(Color::Cyan))
            }
        };

        // Row 6: Prev workspace hotkey
        let row6 = Line::from(vec![
            Span::styled(cursor_str(6), cursor_style),
            Span::raw("Prev workspace key        "),
            keybind_val(6, &app.settings.prev_workspace_key),
        ]);

        // Row 7: Next workspace hotkey
        let row7 = Line::from(vec![
            Span::styled(cursor_str(7), cursor_style),
            Span::raw("Next workspace key        "),
            keybind_val(7, &app.settings.next_workspace_key),
        ]);

        // Row 8: Terminal command-mode hotkey
        let row8 = Line::from(vec![
            Span::styled(cursor_str(8), cursor_style),
            Span::raw("Command mode key         "),
            keybind_val(8, &app.settings.passthrough_key),
        ]);

        // Row 9: Scroll-to-bottom hotkey
        let row9 = Line::from(vec![
            Span::styled(cursor_str(9), cursor_style),
            Span::raw("Scroll to bottom key      "),
            keybind_val(9, &app.settings.scroll_to_bottom_key),
        ]);

        // Row 10: Terminal parser core
        let row10 = Line::from(vec![
            Span::styled(cursor_str(10), cursor_style),
            Span::raw("Terminal core            "),
            Span::styled(
                format!("\u{25C2} {} \u{25B8}", app.settings.terminal_core.label()),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(" experimental", desc_style),
        ]);

        let (title, body) = if app.confirming_delete_agent {
            let agent_name = app
                .settings
                .active_agent()
                .map(|a| a.name.as_str())
                .unwrap_or("(unknown)");
            let warn_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
            let mut lines = vec![
                Line::from(""),
                Line::from(""),
                Line::from(vec![
                    Span::styled("  Delete agent ", desc_style),
                    Span::styled(agent_name.to_string(), warn_style),
                    Span::styled("?", desc_style),
                ]),
                Line::from(""),
            ];
            while lines.len() < 11 {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(vec![
                Span::styled("y", key_style),
                Span::styled(" yes  ", desc_style),
                Span::styled("any other key", key_style),
                Span::styled(" no", desc_style),
            ]));
            (" Delete Agent ", lines)
        } else if let Some((step, profile, buf)) = &app.new_agent_wizard {
            let done_style = Style::default().fg(Color::Green);
            let prompt_style = Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD);
            let mut lines = vec![Line::from("")];

            // Show completed steps
            if *step > 0 {
                lines.push(Line::from(vec![
                    Span::styled("  Name      ", desc_style),
                    Span::styled(&profile.name, done_style),
                ]));
            }
            if *step > 1 {
                lines.push(Line::from(vec![
                    Span::styled("  Command   ", desc_style),
                    Span::styled(&profile.command, done_style),
                ]));
            }

            // Current step prompt
            let label = match step {
                0 => "  Name      ",
                1 => "  Command   ",
                _ => "  YOLO flags",
            };
            lines.push(Line::from(vec![
                Span::styled(label, prompt_style),
                Span::styled(format!("{}▏", buf), edit_style),
            ]));

            // Pad to fill modal
            while lines.len() < 11 {
                lines.push(Line::from(""));
            }

            let step_label = match step {
                0 => "name",
                1 => "command",
                _ => "YOLO flags (optional)",
            };
            lines.push(Line::from(vec![
                Span::styled("Enter", key_style),
                Span::styled(format!(" next ({})  ", step_label), desc_style),
                Span::styled("Esc", key_style),
                Span::styled(" cancel", desc_style),
            ]));

            (" New Agent ", lines)
        } else {
            let hint = if app.is_editing_keybind() {
                Line::from(vec![
                    Span::styled("Any key", key_style),
                    Span::styled(" capture  ", desc_style),
                    Span::styled("Esc", key_style),
                    Span::styled(" cancel", desc_style),
                ])
            } else if app.is_editing_setting() {
                Line::from(vec![
                    Span::styled("Enter", key_style),
                    Span::styled(" confirm  ", desc_style),
                    Span::styled("Esc", key_style),
                    Span::styled(" cancel", desc_style),
                ])
            } else {
                Line::from(vec![
                    Span::styled("j/k", key_style),
                    Span::styled(" navigate  ", desc_style),
                    Span::styled("Enter", key_style),
                    Span::styled(" edit/toggle  ", desc_style),
                    Span::styled("h/l", key_style),
                    Span::styled(" adjust  ", desc_style),
                    Span::styled("n", key_style),
                    Span::styled(" new  ", desc_style),
                    Span::styled("d", key_style),
                    Span::styled(" delete  ", desc_style),
                    Span::styled("Esc", key_style),
                    Span::styled(" close", desc_style),
                ])
            };
            (
                " Settings ",
                vec![
                    Line::from(""),
                    row0,
                    row1,
                    row2,
                    row3,
                    row4,
                    row5,
                    row6,
                    row7,
                    row8,
                    row9,
                    row10,
                    Line::from(""),
                    hint,
                ],
            )
        };

        frame.render_widget(
            Paragraph::new(body).block(
                Block::default()
                    .title(title)
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
fn render_quick_create(frame: &mut Frame, area: Rect, qc: &crate::app::QuickCreateState) {
    use crate::app::QuickCreateField;
    let height = if qc.expanded { 15 } else { 9 };
    let modal = centered_rect(area, 60, height);
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .title(format!(" New Workspace — {} ", qc.repo_name))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let desc = Style::default().fg(Color::DarkGray);
    let label = Style::default().fg(Color::Gray);
    let focused = Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::BOLD);

    let slug = protocol::branch_slug(&qc.name);
    let slug_display = if slug.is_empty() {
        "ws-…".to_string()
    } else {
        slug
    };

    let field_line = |active: bool, lbl: &str, val: &str| -> Line<'static> {
        let lstyle = if active { focused } else { label };
        Line::from(vec![
            Span::styled(format!("  {lbl:<8}"), lstyle),
            Span::styled(val.to_string(), Style::default().fg(Color::White)),
            if active {
                Span::styled("▏", focused)
            } else {
                Span::raw("")
            },
        ])
    };

    let mut lines = vec![
        Line::from(""),
        field_line(matches!(qc.field, QuickCreateField::Name), "task", &qc.name),
        Line::from(vec![
            Span::styled("           branch: ", desc),
            Span::styled(slug_display, Style::default().fg(Color::Green)),
        ]),
    ];
    if qc.expanded {
        lines.push(field_line(
            matches!(qc.field, QuickCreateField::BaseBranch),
            "base",
            if qc.base_branch.is_empty() {
                "(repo default)"
            } else {
                &qc.base_branch
            },
        ));
        // Agent field. Selection mode shows `◂ value ▸` (←/→ cycles, Enter
        // expands to an editable command); edit mode shows the raw launch
        // command as a plain editable field.
        let agent_active = matches!(qc.field, QuickCreateField::Agent);
        let alabel = if agent_active { focused } else { label };
        if qc.agent_command_edit {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<8}", "agent"), alabel),
                Span::styled(qc.agent.clone(), Style::default().fg(Color::White)),
                if agent_active {
                    Span::styled("▏", focused)
                } else {
                    Span::raw("")
                },
            ]));
        } else {
            let arrows = if agent_active { focused } else { desc };
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<8}", "agent"), alabel),
                Span::styled("◂ ", arrows),
                Span::styled(qc.agent.clone(), Style::default().fg(Color::White)),
                Span::styled(" ▸", arrows),
                if agent_active {
                    Span::styled("▏", focused)
                } else {
                    Span::raw("")
                },
            ]));
        }
        lines.push(field_line(
            matches!(qc.field, QuickCreateField::Prompt),
            "prompt",
            &qc.initial_prompt,
        ));
    }
    lines.push(Line::from(""));
    let more = if qc.expanded {
        " next field"
    } else {
        " more options"
    };
    // On the agent selector, Enter expands to an editable command rather than
    // creating; everywhere else it creates.
    let agent_selecting = matches!(qc.field, QuickCreateField::Agent) && !qc.agent_command_edit;
    let enter_hint = if agent_selecting {
        " edit command  "
    } else {
        " create  "
    };
    let mut footer = vec![
        Span::styled("Enter", key),
        Span::styled(enter_hint, desc),
        Span::styled("Tab", key),
        Span::styled(more, desc),
    ];
    if agent_selecting {
        footer.push(Span::styled("  ←/→", key));
        footer.push(Span::styled(" agent", desc));
    }
    footer.push(Span::styled("  Esc", key));
    footer.push(Span::styled(" cancel", desc));
    lines.push(Line::from(footer));
    frame.render_widget(Paragraph::new(lines), inner);
}

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

/// Renders the quick-create ("New Workspace") modal when active. Drawn
/// route-independently because it can be opened from the sidebar while a
/// workspace is already open.
pub fn render_quick_create_modal(frame: &mut Frame, area: Rect, app: &TuiApp) {
    if let Some(qc) = &app.quick_create {
        render_quick_create(frame, area, qc);
    }
}

/// Renders the delete-confirmation modal for whichever item is pending — a
/// workspace or a repository. Drawn route-independently because the deletion
/// can be raised from the sidebar while a workspace is open.
pub fn render_delete_modal(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let body = if let Some(id) = app.pending_delete_workspace {
        let name = app
            .workspaces
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.name.clone())
            .unwrap_or_else(|| id.to_string());
        format!("Delete workspace?\n\n{}", name)
    } else if let Some(repo_id) = app.pending_delete_repo {
        let name = app
            .repositories
            .iter()
            .find(|r| r.id == repo_id)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| repo_id.to_string());
        format!("Remove from sidebar?\n\n{}\n\n(unregisters only — files on disk are untouched)", name)
    } else {
        return;
    };
    let modal = delete_modal_rect(area);
    frame.render_widget(Clear, modal);
    frame.render_widget(
        Paragraph::new(body).alignment(Alignment::Left).block(
            Block::default()
                .title("Confirm Delete")
                .borders(Borders::ALL),
        ),
        modal,
    );
}

/// Returns the rectangle occupied by the tile grid on the home screen.
pub fn grid_rect(area: Rect) -> Rect {
    home_chunks(area)[1]
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
