use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::{Focus, TuiApp};
use protocol::Route;

/// Returns a bold white span for a keybinding label.
fn key(k: &str) -> Span<'static> {
    Span::styled(
        k.to_string(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
}

/// Returns a dark-gray span for a keybinding description.
fn desc(d: &str) -> Span<'static> {
    Span::styled(d.to_string(), Style::default().fg(Color::DarkGray))
}

/// Returns a two-space gap span used to separate hint groups.
fn gap() -> Span<'static> {
    Span::raw("  ")
}

/// Builds the context-sensitive key hint line displayed in the application footer.
///
/// Returns a `Line` whose spans vary based on the current route and focus state in `app`.
pub fn build_footer_hints(app: &TuiApp) -> Line<'static> {
    let spans = match app.route {
        Route::Home => {
            if app.ssh_history_picker.is_some() {
                vec![
                    key("j/k"),
                    desc(" navigate"),
                    gap(),
                    key("Enter"),
                    desc(" select"),
                    gap(),
                    key("n"),
                    desc(" new"),
                    gap(),
                    key("Esc"),
                    desc(" cancel"),
                ]
            } else if app.is_adding_ssh_workspace() {
                vec![
                    key("Tab"),
                    desc(" next field"),
                    gap(),
                    key("Enter"),
                    desc(" add"),
                    gap(),
                    key("Esc"),
                    desc(" cancel"),
                ]
            } else if app.is_adding_workspace() {
                vec![key("Esc"), desc(" cancel")]
            } else if app.is_settings_open() {
                vec![
                    key("j/k"),
                    desc(" navigate"),
                    gap(),
                    key("Space"),
                    desc(" toggle"),
                    gap(),
                    key("Esc"),
                    desc(" close"),
                ]
            } else if app.is_confirming_delete() {
                vec![
                    key("Y"),
                    desc(" confirm delete"),
                    gap(),
                    key("N"),
                    desc(" cancel"),
                ]
            } else if app.is_renaming_workspace() {
                vec![
                    key("Enter"),
                    desc(" confirm"),
                    gap(),
                    key("Esc"),
                    desc(" cancel"),
                ]
            } else if app.moving_workspace {
                vec![
                    key("j/k"),
                    desc(" move"),
                    gap(),
                    key("Enter"),
                    desc(" done"),
                    gap(),
                    key("Esc"),
                    desc(" done"),
                ]
            } else {
                vec![
                    key("Enter"),
                    desc(" open"),
                    gap(),
                    key("n"),
                    desc(" new"),
                    gap(),
                    key("R"),
                    desc(" ssh"),
                    gap(),
                    key("e"),
                    desc(" rename"),
                    gap(),
                    key("D"),
                    desc(" delete"),
                    gap(),
                    key("Space"),
                    desc(" expand"),
                    gap(),
                    key("!"),
                    desc(" attention"),
                    gap(),
                    key("M"),
                    desc(" move"),
                    gap(),
                    key("S"),
                    desc(" settings"),
                    gap(),
                    key("q"),
                    desc(" quit"),
                ]
            }
        }
        Route::Workspace { .. } => match app.focus {
            Focus::WsBar => vec![
                key("h/l"),
                desc(" select"),
                gap(),
                key("Enter"),
                desc(" switch"),
                gap(),
                key("Esc"),
                desc(" cancel"),
            ],
            Focus::WsTerminalTabs => vec![
                key("h/l"),
                desc(" switch tab"),
                gap(),
                key("n"),
                desc(" new tab"),
                gap(),
                key("x"),
                desc(" close"),
                gap(),
                key("r"),
                desc(" rename"),
                gap(),
                key("Y"),
                desc(" yolo"),
                gap(),
                key("F"),
                desc(" fullscreen"),
                gap(),
                key("Tab"),
                desc(" next pane"),
                gap(),
                key("Shift+Tab"),
                desc(" previous pane"),
                gap(),
                key("Esc"),
                desc(" home"),
            ],
            Focus::WsTerminal if app.terminal_command_mode() => vec![
                desc("(command mode)"),
                gap(),
                key(&app.settings.passthrough_key),
                desc(" terminal"),
                gap(),
                key("F"),
                desc(" fullscreen"),
                gap(),
                key(&app.settings.scroll_to_bottom_key),
                desc(" scroll to bottom"),
                gap(),
                key("Tab"),
                desc(" next pane"),
                gap(),
                key("Shift+Tab"),
                desc(" previous pane"),
                gap(),
                key("Esc"),
                desc(" unfocus"),
            ],
            Focus::WsTerminal => vec![
                desc("(keys sent to terminal)"),
                gap(),
                key(&app.settings.passthrough_key),
                desc(" command mode"),
            ],
            Focus::WsBranches => vec![
                key("j/k"),
                desc(" navigate"),
                gap(),
                key("[/]"),
                desc(" local/remote"),
                gap(),
                key("Space"),
                desc(" checkout"),
                gap(),
                key("c"),
                desc(" create"),
                gap(),
                key("D"),
                desc(" delete"),
                gap(),
                key("p"),
                desc(" pull"),
                gap(),
                key("f"),
                desc(" fetch"),
                gap(),
                key("P"),
                desc(" push"),
                gap(),
                key("F"),
                desc(" fullscreen"),
                gap(),
                key("Tab"),
                desc(" next pane"),
                gap(),
                key("Shift+Tab"),
                desc(" previous pane"),
                gap(),
                key("Esc"),
                desc(" home"),
            ],
            Focus::WsLog => match app.log_item_at(app.ws_selected_commit) {
                crate::app::LogItem::UncommittedHeader => vec![
                    key("j/k"),
                    desc(" navigate"),
                    gap(),
                    key("Enter"),
                    desc(" expand/collapse"),
                    gap(),
                    key("+/-"),
                    desc(" stage all"),
                    gap(),
                    key("c"),
                    desc(" commit"),
                    gap(),
                    key("s"),
                    desc(" stash"),
                    gap(),
                    key("t"),
                    desc(" tags"),
                    gap(),
                    key("F"),
                    desc(" fullscreen"),
                    gap(),
                    key("Tab"),
                    desc(" next pane"),
                    gap(),
                    key("Shift+Tab"),
                    desc(" previous pane"),
                    gap(),
                    key("Esc"),
                    desc(" home"),
                ],
                crate::app::LogItem::ChangedFile(_) => vec![
                    key("j/k"),
                    desc(" navigate"),
                    gap(),
                    key("Space"),
                    desc(" stage/unstage"),
                    gap(),
                    key("+/-"),
                    desc(" all"),
                    gap(),
                    key("c"),
                    desc(" commit"),
                    gap(),
                    key("d"),
                    desc(" discard"),
                    gap(),
                    key("s"),
                    desc(" stash"),
                    gap(),
                    key("Enter"),
                    desc(" diff"),
                    gap(),
                    key("t"),
                    desc(" tags"),
                    gap(),
                    key("F"),
                    desc(" fullscreen"),
                    gap(),
                    key("Tab"),
                    desc(" next pane"),
                    gap(),
                    key("Shift+Tab"),
                    desc(" previous pane"),
                    gap(),
                    key("Esc"),
                    desc(" home"),
                ],
                crate::app::LogItem::Commit(_) => vec![
                    key("j/k"),
                    desc(" navigate"),
                    gap(),
                    key("Enter"),
                    desc(" expand/collapse"),
                    gap(),
                    key("t"),
                    desc(" tags"),
                    gap(),
                    key("F"),
                    desc(" fullscreen"),
                    gap(),
                    key("Tab"),
                    desc(" next pane"),
                    gap(),
                    key("Shift+Tab"),
                    desc(" previous pane"),
                    gap(),
                    key("Esc"),
                    desc(" home"),
                ],
                crate::app::LogItem::CommitFile(_, _) => vec![
                    key("j/k"),
                    desc(" navigate"),
                    gap(),
                    key("Enter"),
                    desc(" diff"),
                    gap(),
                    key("t"),
                    desc(" tags"),
                    gap(),
                    key("F"),
                    desc(" fullscreen"),
                    gap(),
                    key("Tab"),
                    desc(" next pane"),
                    gap(),
                    key("Shift+Tab"),
                    desc(" previous pane"),
                    gap(),
                    key("Esc"),
                    desc(" home"),
                ],
            },
            Focus::WsDiff => vec![
                key("j/k"),
                desc(" scroll"),
                gap(),
                key("F"),
                desc(" fullscreen"),
                gap(),
                key("Tab"),
                desc(" next pane"),
                gap(),
                key("Shift+Tab"),
                desc(" previous pane"),
                gap(),
                key("Esc"),
                desc(" home"),
            ],
            _ => vec![
                key("F"),
                desc(" fullscreen"),
                gap(),
                key("Tab"),
                desc(" next pane"),
                gap(),
                key("Shift+Tab"),
                desc(" previous pane"),
                gap(),
                key("Esc"),
                desc(" home"),
            ],
        },
    };

    Line::from(spans)
}

/// Renders the context-sensitive key hint footer into `area`.
pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let mut hints = build_footer_hints(app);
    if app.settings.show_frame_counter {
        hints.spans.push(Span::styled(
            format!("  [FPS: {}]", app.debug_fps),
            Style::default().fg(Color::DarkGray),
        ));
    }
    frame.render_widget(
        Paragraph::new(hints)
            .block(Block::default().borders(Borders::TOP))
            .style(Style::default().fg(Color::Gray)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{DirBrowserState, Focus, SshHistoryPicker, TuiApp};
    use protocol::{AttentionLevel, ChangedFile, GitState, WorkspaceSummary};
    use uuid::Uuid;

    fn hints_contain(line: &Line, keyword: &str) -> bool {
        line.spans.iter().any(|s| s.content.contains(keyword))
    }

    fn make_ws() -> WorkspaceSummary {
        WorkspaceSummary {
            id: Uuid::new_v4(),
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

    fn app_with_workspace() -> (TuiApp, Uuid) {
        let mut app = TuiApp::default();
        let ws = make_ws();
        let id = ws.id;
        app.set_workspaces(vec![ws]);
        app.open_workspace(id);
        (app, id)
    }

    #[test]
    fn home_default_hints() {
        let app = TuiApp::default();
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "open"));
        assert!(hints_contain(&line, "new"));
        assert!(hints_contain(&line, "quit"));
    }

    #[test]
    fn home_adding_workspace_hints() {
        let mut app = TuiApp::default();
        app.dir_browser = Some(DirBrowserState {
            path_input: "/tmp".to_string(),
            entries: vec![],
            selected: 0,
            show_hidden: false,
            editing_path: false,
        });
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "cancel"));
    }

    #[test]
    fn home_confirming_delete_hints() {
        let mut app = TuiApp::default();
        app.pending_delete_workspace = Some(Uuid::new_v4());
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "confirm delete"));
    }

    #[test]
    fn home_ssh_history_picker_hints() {
        let mut app = TuiApp::default();
        app.ssh_history_picker = Some(SshHistoryPicker { selected: 0 });
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "navigate"));
        assert!(hints_contain(&line, "select"));
    }

    #[test]
    fn workspace_terminal_focus_hints() {
        let (mut app, _id) = app_with_workspace();
        app.settings.passthrough_key = "ctrl+shift+p".to_string();
        // open_workspace sets focus to WsTerminal
        assert_eq!(app.focus, Focus::WsTerminal);
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "command mode"));
        assert!(hints_contain(&line, "ctrl+shift+p"));
    }

    #[test]
    fn workspace_log_uncommitted_header_hints() {
        let (mut app, _id) = app_with_workspace();
        app.focus = Focus::WsLog;
        app.ws_selected_commit = 0;
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "stage all"));
        assert!(hints_contain(&line, "commit"));
    }

    #[test]
    fn workspace_log_changed_file_hints() {
        let (mut app, id) = app_with_workspace();
        app.focus = Focus::WsLog;
        // Insert git state with a changed file
        let git = GitState {
            changed: vec![ChangedFile {
                path: "foo.rs".into(),
                index_status: 'M',
                worktree_status: ' ',
            }],
            ..GitState::default()
        };
        app.workspace_git.insert(id, git);
        // Expand uncommitted section and select the first file (index 1)
        app.ws_uncommitted_expanded = true;
        app.ws_selected_commit = 1;
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "discard"));
    }

    #[test]
    fn workspace_branches_focus_hints() {
        let (mut app, _id) = app_with_workspace();
        app.focus = Focus::WsBranches;
        let line = build_footer_hints(&app);
        assert!(hints_contain(&line, "checkout"));
        assert!(hints_contain(&line, "pull"));
        assert!(hints_contain(&line, "push"));
    }
}
