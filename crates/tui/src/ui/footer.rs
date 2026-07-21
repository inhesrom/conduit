use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{Settings, TuiApp};
use crate::shortcuts::{self, HintSurface, ResolvedShortcut, ShortcutContext, ShortcutId};

fn key(value: impl Into<String>) -> Span<'static> {
    Span::styled(
        value.into(),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )
}

fn description(value: impl Into<String>) -> Span<'static> {
    Span::styled(value.into(), Style::default().fg(Color::DarkGray))
}

fn visible_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn concise_label(shortcut: &ResolvedShortcut) -> &'static str {
    match shortcut.id {
        ShortcutId::OpenHelp => "Help",
        ShortcutId::PreviousWorkspace => "previous workspace",
        ShortcutId::NextWorkspace => "next workspace",
        ShortcutId::CycleSidebar => "sidebar",
        ShortcutId::MoveDown | ShortcutId::MoveUp => "navigate",
        ShortcutId::Open => "open",
        ShortcutId::Back => "back",
        ShortcutId::NewWorkspace => "new workspace",
        ShortcutId::AddRepository => "add repository",
        ShortcutId::AddSshRepository => "add SSH",
        ShortcutId::Delete => "delete",
        ShortcutId::Review => "review",
        ShortcutId::ToggleReady => "ready",
        ShortcutId::ToggleReviewFilter => "filter",
        ShortcutId::RefreshGit => "refresh",
        ShortcutId::OpenSettings => "settings",
        ShortcutId::Collapse => "collapse",
        ShortcutId::Expand => "expand",
        ShortcutId::ExpandOrOpen => "expand/open",
        ShortcutId::ClosePopout => "close",
        ShortcutId::Confirm => "confirm",
        ShortcutId::Cancel => "cancel",
        ShortcutId::NextField => "next field",
        ShortcutId::PreviousField => "previous field",
        ShortcutId::PreviousChoice | ShortcutId::NextChoice => "choice",
        ShortcutId::MoveWorkspace => "done",
        ShortcutId::ToggleSetting => "edit/toggle",
        ShortcutId::AdjustSettingLeft | ShortcutId::AdjustSettingRight => "adjust",
        ShortcutId::NewAgent => "new agent",
        ShortcutId::DeleteAgent => "delete agent",
        ShortcutId::SelectNewSsh => "new",
        ShortcutId::GoToParent => "parent",
        ShortcutId::ToggleHidden => "hidden",
        ShortcutId::EditPath => "edit path",
        ShortcutId::EnterDirectory => "enter directory",
        ShortcutId::SelectDirectory => "select directory",
        ShortcutId::ToggleTerminalCommandMode => "command mode",
        ShortcutId::ScrollTerminalToBottom => "bottom",
        ShortcutId::ToggleFullscreen => "fullscreen",
        ShortcutId::ToggleYolo => "YOLO",
        ShortcutId::NextPane => "next pane",
        ShortcutId::PreviousPane => "previous pane",
        ShortcutId::OpenWorkspaceCommand => "command",
        ShortcutId::SelectFirstTab | ShortcutId::SelectSecondTab => "tab",
        ShortcutId::NextTerminalTab | ShortcutId::PreviousTerminalTab => "switch tab",
        ShortcutId::NewShellTab => "new tab",
        ShortcutId::CloseTerminalTab => "close tab",
        ShortcutId::RenameTerminalTab => "rename tab",
        ShortcutId::SwitchAgent => "agent",
        ShortcutId::StartActiveTerminal => "start",
        ShortcutId::StopActiveTerminal => "stop",
        ShortcutId::ToggleLogItem => "expand/open",
        ShortcutId::ToggleStage => "stage/unstage",
        ShortcutId::StageAll => "stage all",
        ShortcutId::UnstageAll => "unstage all",
        ShortcutId::Commit => "commit",
        ShortcutId::Discard => "discard",
        ShortcutId::DiscardAll => "discard all",
        ShortcutId::Stash => "stash",
        ShortcutId::StashAll => "stash all",
        ShortcutId::ToggleTags => "tags",
        ShortcutId::SelectLocalBranches | ShortcutId::SelectRemoteBranches => "branch pane",
        ShortcutId::CheckoutBranch => "checkout",
        ShortcutId::CreateBranch => "create",
        ShortcutId::DeleteBranch => "delete",
        ShortcutId::Pull => "pull",
        ShortcutId::Fetch => "fetch",
        ShortcutId::Push => "push",
        ShortcutId::ToggleReviewPane => "switch pane",
        ShortcutId::OpenReviewFile => "open diff",
        ShortcutId::ReviewPageDown | ShortcutId::ReviewPageUp => "scroll 10",
        ShortcutId::OpenPullRequest => "open PR",
        ShortcutId::RunCommand => "run",
        ShortcutId::OutputUp | ShortcutId::OutputDown => "output",
        ShortcutId::OutputPageUp | ShortcutId::OutputPageDown => "output page",
        ShortcutId::AttachSession => "attach",
        ShortcutId::NewSession => "new",
        ShortcutId::DeleteSession => "delete",
        ShortcutId::RefreshSessions => "refresh",
        ShortcutId::CloseHelp
        | ShortcutId::ToggleHelpView
        | ShortcutId::HelpScrollDown
        | ShortcutId::HelpScrollUp
        | ShortcutId::HelpPageDown
        | ShortcutId::HelpPageUp
        | ShortcutId::HelpTop
        | ShortcutId::HelpBottom
        | ShortcutId::Quit => shortcut.label,
    }
}

fn footer_shortcuts(app: &TuiApp, context: ShortcutContext) -> Vec<ResolvedShortcut> {
    let mut shortcuts = shortcuts::resolved_shortcuts(app, context)
        .into_iter()
        .filter(|shortcut| shortcut.id != ShortcutId::OpenHelp)
        .filter(|shortcut| shortcut.surfaces.contains(&HintSurface::Footer))
        .filter(|shortcut| shortcut.footer_priority.is_some())
        .collect::<Vec<_>>();
    let empty_home = matches!(
        context,
        ShortcutContext::HomeSidebar | ShortcutContext::HomeRail
    ) && app.repositories.is_empty()
        && app.workspaces.is_empty();
    if empty_home {
        shortcuts
            .retain(|shortcut| !matches!(shortcut.id, ShortcutId::MoveDown | ShortcutId::MoveUp));
        if let Some(add_repository) = shortcuts
            .iter_mut()
            .find(|shortcut| shortcut.id == ShortcutId::AddRepository)
        {
            add_repository.footer_priority = Some(1);
        }
    }
    shortcuts.sort_by_key(|shortcut| shortcut.footer_priority.unwrap_or(u8::MAX));
    shortcuts
}

fn push_group(spans: &mut Vec<Span<'static>>, shortcut: &ResolvedShortcut) {
    if !spans.is_empty() {
        spans.push(Span::raw("  "));
    }
    spans.push(key(shortcut.keys.clone()));
    spans.push(description(format!(" {}", concise_label(shortcut))));
}

/// Builds a complete, width-aware footer line for the active shortcut context.
pub fn build_footer_hints_for_width(app: &TuiApp, width: u16) -> Line<'static> {
    let context = shortcuts::active_context(app);
    let show_help = context.help_available();
    let help_width = if show_help {
        UnicodeWidthStr::width("? Help")
    } else {
        0
    };
    let reserved_help = if show_help { help_width + 2 } else { 0 };
    let available = width as usize;
    let control_limit = available.saturating_sub(reserved_help);
    let mut spans = Vec::new();

    for shortcut in footer_shortcuts(app, context) {
        let separator = usize::from(!spans.is_empty()) * 2;
        let group_width = UnicodeWidthStr::width(shortcut.keys.as_str())
            + 1
            + UnicodeWidthStr::width(concise_label(&shortcut));
        if visible_width(&spans) + separator + group_width <= control_limit {
            push_group(&mut spans, &shortcut);
        }
    }

    if app.settings.show_frame_counter {
        let fps = format!("FPS {}", app.debug_fps);
        let separator = usize::from(!spans.is_empty()) * 2;
        if visible_width(&spans) + separator + fps.len() <= control_limit {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            spans.push(description(fps));
        }
    }

    if show_help && available >= help_width {
        let padding = available.saturating_sub(visible_width(&spans) + help_width);
        spans.push(Span::raw(" ".repeat(padding)));
        spans.push(key("?"));
        spans.push(description(" Help"));
    }

    Line::from(spans)
}

/// Builds a footer for a catalog context that is not backed by [`TuiApp`], such as the
/// pre-attach session chooser.
pub fn build_context_hints_for_width(
    settings: &Settings,
    context: ShortcutContext,
    width: u16,
) -> Line<'static> {
    let show_help = context.help_available();
    let help_width = if show_help {
        UnicodeWidthStr::width("? Help")
    } else {
        0
    };
    let reserved_help = if show_help { help_width + 2 } else { 0 };
    let available = width as usize;
    let control_limit = available.saturating_sub(reserved_help);
    let mut shortcuts = shortcuts::resolved_shortcuts_for_context(settings, context)
        .into_iter()
        .filter(|shortcut| shortcut.id != ShortcutId::OpenHelp)
        .filter(|shortcut| shortcut.footer_priority.is_some())
        .collect::<Vec<_>>();
    shortcuts.sort_by_key(|shortcut| shortcut.footer_priority.unwrap_or(u8::MAX));
    let mut spans = Vec::new();
    for shortcut in shortcuts {
        let separator = usize::from(!spans.is_empty()) * 2;
        let group_width = UnicodeWidthStr::width(shortcut.keys.as_str())
            + 1
            + UnicodeWidthStr::width(concise_label(&shortcut));
        if visible_width(&spans) + separator + group_width <= control_limit {
            push_group(&mut spans, &shortcut);
        }
    }
    if show_help && available >= help_width {
        let padding = available.saturating_sub(visible_width(&spans) + help_width);
        spans.push(Span::raw(" ".repeat(padding)));
        spans.push(key("?"));
        spans.push(description(" Help"));
    }
    Line::from(spans)
}

/// Builds a catalog-derived hint line for a modal's local context.
pub fn build_modal_hints(app: &TuiApp, context: ShortcutContext, width: u16) -> Line<'static> {
    let mut spans = Vec::new();
    let mut shortcuts = shortcuts::resolved_shortcuts(app, context)
        .into_iter()
        .filter(|shortcut| shortcut.surfaces.contains(&HintSurface::Modal))
        .filter(|shortcut| shortcut.footer_priority.is_some())
        .collect::<Vec<_>>();
    shortcuts.sort_by_key(|shortcut| shortcut.footer_priority.unwrap_or(u8::MAX));
    for shortcut in shortcuts {
        let separator = usize::from(!spans.is_empty()) * 2;
        let group_width = UnicodeWidthStr::width(shortcut.keys.as_str())
            + 1
            + UnicodeWidthStr::width(concise_label(&shortcut));
        if visible_width(&spans) + separator + group_width > width as usize {
            continue;
        }
        push_group(&mut spans, &shortcut);
    }
    Line::from(spans)
}

pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    frame.render_widget(
        Paragraph::new(build_footer_hints_for_width(app, area.width))
            .block(Block::default().borders(Borders::TOP))
            .style(Style::default().fg(Color::Gray)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Focus, TuiApp};
    use protocol::{AttentionLevel, WorkspaceSummary};
    use uuid::Uuid;

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn app_with_workspace() -> TuiApp {
        let mut app = TuiApp::default();
        let id = Uuid::new_v4();
        app.set_workspaces(vec![WorkspaceSummary {
            id,
            name: "test".into(),
            path: "/tmp/test".into(),
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
            adopted: false,
        }]);
        app.open_workspace(id);
        app
    }

    #[test]
    fn footer_keeps_help_right_aligned_at_common_widths() {
        for width in [40, 80, 120] {
            let line = build_footer_hints_for_width(&TuiApp::default(), width);
            let rendered = text(&line);
            assert_eq!(UnicodeWidthStr::width(rendered.as_str()), width as usize);
            assert!(rendered.ends_with("? Help"));
        }
        let narrow = text(&build_footer_hints_for_width(&TuiApp::default(), 40));
        assert!(narrow.contains("N/a add repository"), "{narrow}");
        assert!(!narrow.contains("navigate"), "{narrow}");
    }

    #[test]
    fn terminal_passthrough_advertises_switching_but_not_help() {
        let mut app = app_with_workspace();
        app.settings.prev_workspace_key = "alt+left".to_string();
        app.settings.next_workspace_key = "alt+right".to_string();
        let rendered = text(&build_footer_hints_for_width(&app, 120));
        assert!(rendered.contains("Alt+← previous workspace"), "{rendered}");
        assert!(rendered.contains("Alt+→ next workspace"), "{rendered}");
        assert!(rendered.contains("command mode"));
        assert!(!rendered.contains("? Help"));
    }

    #[test]
    fn modal_context_replaces_underlying_workspace_hints() {
        let mut app = app_with_workspace();
        app.pending_delete_workspace = Some(Uuid::new_v4());
        let rendered = text(&build_footer_hints_for_width(&app, 80));
        assert!(rendered.contains("y/Y confirm"), "{rendered}");
        assert!(rendered.contains("n/N/Esc cancel"), "{rendered}");
        assert!(!rendered.contains("switch tab"));
    }

    #[test]
    fn modal_hint_rows_use_catalog_order_and_fit_whole_actions() {
        let app = TuiApp::default();
        let full = text(&build_modal_hints(
            &app,
            ShortcutContext::DiscardConfirm,
            60,
        ));
        assert_eq!(full, "y/Enter confirm  n/Esc cancel");

        let narrow = text(&build_modal_hints(
            &app,
            ShortcutContext::DiscardConfirm,
            17,
        ));
        assert_eq!(narrow, "y/Enter confirm");
    }

    #[test]
    fn command_mode_uses_configured_keys() {
        let mut app = app_with_workspace();
        app.focus = Focus::WsTerminal;
        app.toggle_terminal_command_mode();
        app.settings.passthrough_key = "ctrl+shift+p".to_string();
        app.settings.terminal_fullscreen_key = "alt+enter".to_string();
        let rendered = text(&build_footer_hints_for_width(&app, 120));
        assert!(rendered.contains("Ctrl+Shift+P command mode"), "{rendered}");
        assert!(rendered.contains("Alt+Enter fullscreen"), "{rendered}");
    }
}
