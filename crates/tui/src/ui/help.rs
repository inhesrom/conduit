use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::app::{Settings, TuiApp};
use crate::shortcuts::{self, HelpSection, HelpState, HelpView};

fn overlay_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(100).max(1);
    let height = area.height.saturating_sub(2).min(34).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

fn help_lines(sections: &[HelpSection], key_column_width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (section_index, section) in sections.iter().enumerate() {
        if section_index > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            section.title.clone(),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for shortcut in &section.shortcuts {
            let key_width = UnicodeWidthStr::width(shortcut.keys.as_str());
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "  {}{}",
                        shortcut.keys,
                        " ".repeat(key_column_width.saturating_sub(key_width))
                    ),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(shortcut.label, Style::default().fg(Color::Gray)),
            ]));
        }
    }
    lines
}

fn help_controls(settings: &Settings) -> String {
    let controls =
        shortcuts::resolved_shortcuts_for_context(settings, shortcuts::ShortcutContext::Help);
    let keys = |id| {
        controls
            .iter()
            .find(|shortcut| shortcut.id == id)
            .map(|shortcut| shortcut.keys.as_str())
            .unwrap_or_default()
    };
    let view = keys(shortcuts::ShortcutId::ToggleHelpView);
    let down = keys(shortcuts::ShortcutId::HelpScrollDown);
    let up = keys(shortcuts::ShortcutId::HelpScrollUp);
    let page_down = keys(shortcuts::ShortcutId::HelpPageDown);
    let page_up = keys(shortcuts::ShortcutId::HelpPageUp);
    let top = keys(shortcuts::ShortcutId::HelpTop);
    let bottom = keys(shortcuts::ShortcutId::HelpBottom);
    let close = keys(shortcuts::ShortcutId::CloseHelp);
    format!(
        " {view} view · {down}/{up} scroll · {page_down}/{page_up} page · {top}/{bottom} · {close} close "
    )
}

fn render_sections(
    frame: &mut Frame,
    area: Rect,
    state: &HelpState,
    settings: &Settings,
    sections: Vec<HelpSection>,
) {
    let modal = overlay_rect(area);
    frame.render_widget(Clear, modal);

    let view_label = match state.view {
        HelpView::Current => "Current",
        HelpView::All => "All",
    };
    let inactive = Style::default().fg(Color::DarkGray);
    let active = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let title = Line::from(vec![
        Span::styled(
            " Shortcuts ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " Current ",
            if state.view == HelpView::Current {
                active
            } else {
                inactive
            },
        ),
        Span::styled(
            " All ",
            if state.view == HelpView::All {
                active
            } else {
                inactive
            },
        ),
    ]);
    let block = Block::default()
        .title(title)
        .title_bottom(
            Line::from(format!(" {view_label} · {}", help_controls(settings)))
                .alignment(Alignment::Right),
        )
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let key_column_width = if inner.width < 60 { 14 } else { 22 };
    let lines = help_lines(&sections, key_column_width);
    let max_scroll = lines.len().saturating_sub(inner.height as usize) as u16;
    let scroll = state.scroll.min(max_scroll);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .style(Style::default().fg(Color::Gray)),
        inner,
    );
}

pub fn render(frame: &mut Frame, area: Rect, app: &TuiApp) {
    let Some(state) = &app.help else {
        return;
    };
    let sections = match state.view {
        HelpView::Current => shortcuts::current_help_sections(app, state.context),
        HelpView::All => shortcuts::all_help_sections(&app.settings),
    };
    render_sections(frame, area, state, &app.settings, sections);
}

pub fn render_for_context(frame: &mut Frame, area: Rect, state: &HelpState, settings: &Settings) {
    let sections = match state.view {
        HelpView::Current => shortcuts::current_help_sections_for_context(settings, state.context),
        HelpView::All => shortcuts::all_help_sections(settings),
    };
    render_sections(frame, area, state, settings, sections);
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;
    use crate::shortcuts::ShortcutContext;

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn current_help_names_the_captured_context_and_actions() {
        let state = HelpState::new(ShortcutContext::ChooserBrowse);
        for width in [40, 80, 120] {
            let backend = TestBackend::new(width, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| render_for_context(frame, frame.area(), &state, &Settings::default()))
                .unwrap();
            let rendered = buffer_text(&terminal);
            assert!(rendered.contains("Session chooser"), "width {width}");
            assert!(rendered.contains("Attach or revive"), "width {width}");
            assert!(!rendered.contains("Terminal tabs"), "width {width}");
        }
    }

    #[test]
    fn all_help_is_scrollable_and_contains_late_sections() {
        let mut state = HelpState::new(ShortcutContext::ChooserBrowse);
        state.view = HelpView::All;
        state.scroll = u16::MAX;
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_for_context(frame, frame.area(), &state, &Settings::default()))
            .unwrap();
        let rendered = buffer_text(&terminal);
        assert!(rendered.contains("Session chooser"));
        assert!(rendered.contains("Create and attach"));
    }
}
