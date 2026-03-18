use protocol::{AttentionLevel, WorkspaceSummary};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub const COLS: u16 = 3;
pub const TILE_H: u16 = 9;
pub const ORANGE: Color = Color::Rgb(255, 165, 0);

/// Renders the workspace tile grid into `area`.
///
/// Each workspace in `items` is displayed as a fixed-size rounded card.
/// `selected` highlights the focused tile; `flash_on` drives attention pulse.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    items: &[WorkspaceSummary],
    selected: usize,
    flash_on: bool,
    attention_enabled: bool,
) {
    if items.is_empty() {
        render_empty_state(frame, area);
        return;
    }

    let tile_w = area.width / COLS;
    let cols = COLS as usize;
    for (i, ws) in items.iter().enumerate() {
        let tile = tile_rect(area, i, cols, tile_w);
        if tile.width < 8 || tile.height < 9 {
            continue;
        }
        render_tile(frame, tile, ws, i == selected, flash_on, attention_enabled);
    }
}

/// Returns the tile index at pixel coordinate (`x`, `y`) within `area`,
/// or `None` if the coordinate falls outside all tiles.
pub fn index_at(area: Rect, x: u16, y: u16, item_count: usize) -> Option<usize> {
    if item_count == 0 {
        return None;
    }
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return None;
    }
    let rel_x = x - area.x;
    let rel_y = y - area.y;
    let tile_w = area.width / COLS;
    let cols = COLS as usize;
    let col = (rel_x / tile_w) as usize;
    let row = (rel_y / TILE_H) as usize;
    let idx = row * cols + col;
    (idx < item_count).then_some(idx)
}

/// Draws the placeholder shown when there are no workspaces.
fn render_empty_state(frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title("Workspaces")
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::White));
    frame.render_widget(
        Paragraph::new("No workspaces yet. Press `n` to add current directory.").block(block),
        area,
    );
}

/// Computes the `Rect` for tile at grid position `index` given `cols` columns.
pub fn tile_rect(area: Rect, index: usize, cols: usize, tile_w: u16) -> Rect {
    let row = index / cols;
    let col = index % cols;
    Rect {
        x: area.x + (col as u16 * tile_w),
        y: area.y + (row as u16 * TILE_H),
        width: tile_w.min(area.width.saturating_sub(col as u16 * tile_w)),
        height: TILE_H.min(area.height.saturating_sub(row as u16 * TILE_H)),
    }
}

/// Renders a single workspace tile into `tile`.
fn render_tile(
    frame: &mut Frame,
    tile: Rect,
    ws: &WorkspaceSummary,
    is_selected: bool,
    flash_on: bool,
    attention_enabled: bool,
) {
    let border_style = tile_border_style(ws, is_selected, flash_on, attention_enabled);
    let border_type = if is_selected {
        BorderType::Thick
    } else {
        BorderType::Rounded
    };
    let title_left = Line::from(Span::styled(
        format!(" {} ", ws.name),
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    ));
    let title_right = build_status_badge(&ws.attention, flash_on, attention_enabled);
    let body_max = (tile.width as usize).saturating_sub(6);
    let body_lines = build_body_lines(ws, body_max);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title_top(title_left)
        .title_top(title_right.right_aligned());

    frame.render_widget(Paragraph::new(body_lines).block(block), tile);
}

/// Computes the border style based on attention level, selection, and flash phase.
fn tile_border_style(
    ws: &WorkspaceSummary,
    is_selected: bool,
    flash_on: bool,
    attention_enabled: bool,
) -> Style {
    if !attention_enabled {
        return if is_selected {
            Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
    }
    let base = match ws.attention {
        AttentionLevel::Error => {
            if flash_on {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::LightRed)
            }
        }
        AttentionLevel::NeedsInput => {
            if flash_on {
                Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            }
        }
        _ => Style::default().fg(Color::White),
    };

    if !is_selected {
        return base;
    }

    let needs_attention = matches!(
        ws.attention,
        AttentionLevel::NeedsInput | AttentionLevel::Error
    );
    if needs_attention && flash_on {
        base
    } else {
        Style::default()
            .fg(Color::LightBlue)
            .add_modifier(Modifier::BOLD)
    }
}

/// Builds the right-aligned status badge for attention states.
/// Returns an empty line for non-attention tiles.
fn build_status_badge(
    attention: &AttentionLevel,
    flash_on: bool,
    attention_enabled: bool,
) -> Line<'static> {
    if !attention_enabled {
        return Line::from("");
    }
    match attention {
        AttentionLevel::NeedsInput => {
            let style = Style::default().fg(ORANGE);
            Line::from(Span::styled(" ⚠ input ", flash_bold(style, flash_on)))
        }
        AttentionLevel::Error => {
            let style = Style::default().fg(Color::Red);
            Line::from(Span::styled(" ✖ error ", flash_bold(style, flash_on)))
        }
        _ => Line::from(""),
    }
}

/// Builds the 7 inner body lines displayed inside a workspace tile.
///
/// The count is fixed at 7 to fill `TILE_H - 2` rows (tile height minus
/// the top and bottom border lines).
fn build_body_lines(ws: &WorkspaceSummary, body_max: usize) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        build_branch_line(ws, body_max),
        build_path_line(ws, body_max),
        Line::from(""),
        build_stats_line(ws),
        Line::from(""),
        Line::from(""),
    ]
}

fn build_branch_line(ws: &WorkspaceSummary, body_max: usize) -> Line<'static> {
    let branch = ws.branch.as_deref().unwrap_or("-");
    let ab = match (ws.ahead, ws.behind) {
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
    let ab_style = if ws.ahead.unwrap_or(0) > 0 || ws.behind.unwrap_or(0) > 0 {
        Style::default().fg(Color::Cyan)
    } else {
        dim_style()
    };
    Line::from(vec![
        Span::styled("  ⎇ ", dim_style()),
        Span::styled(
            truncate_end(branch, body_max.saturating_sub(ab.len())),
            Style::default().fg(Color::White),
        ),
        Span::styled(ab, ab_style),
    ])
}

fn build_path_line(ws: &WorkspaceSummary, body_max: usize) -> Line<'static> {
    let dim = dim_style();
    let display_path = if let Some(ref host) = ws.ssh_host {
        format!("{}:{}", host, ws.path)
    } else {
        ws.path.clone()
    };
    Line::from(vec![
        Span::styled("  ", dim),
        Span::styled(truncate_end(&display_path, body_max), dim),
    ])
}

fn build_stats_line(ws: &WorkspaceSummary) -> Line<'static> {
    let dim = dim_style();
    Line::from(vec![
        Span::styled("  ◈ ", dim),
        Span::styled(
            format!("{} changes", ws.dirty_files),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled("    ● ", dim),
        Span::styled(
            if ws.agent_running { "agent" } else { "off" },
            running_style(ws.agent_running),
        ),
        Span::styled("    ⌀ ", dim),
        Span::styled(
            if ws.shell_running { "shell" } else { "off" },
            running_style(ws.shell_running),
        ),
    ])
}

#[inline]
fn dim_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn running_style(running: bool) -> Style {
    if running {
        Style::default().fg(Color::Green)
    } else {
        dim_style()
    }
}

/// Returns `style` with `BOLD` added when `flash_on` is true.
fn flash_bold(style: Style, flash_on: bool) -> Style {
    if flash_on {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// Truncates `input` to at most `max` characters, appending `…` if shortened.
fn truncate_end(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    if max <= 1 {
        return "…".to_string();
    }
    let mut s: String = input.chars().take(max - 1).collect();
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn area() -> Rect {
        Rect::new(0, 0, 90, 36)
    }

    // --- index_at ---

    #[test]
    fn index_at_first_tile() {
        assert_eq!(index_at(area(), 1, 1, 6), Some(0));
    }

    #[test]
    fn index_at_second_column() {
        let a = area();
        let tile_w = a.width / COLS;
        assert_eq!(index_at(a, tile_w + 1, 1, 6), Some(1));
    }

    #[test]
    fn index_at_second_row() {
        assert_eq!(index_at(area(), 1, TILE_H + 1, 6), Some(3));
    }

    #[test]
    fn index_at_outside_area() {
        assert_eq!(index_at(area(), 200, 200, 6), None);
    }

    #[test]
    fn index_at_zero_items() {
        assert_eq!(index_at(area(), 1, 1, 0), None);
    }

    #[test]
    fn index_at_beyond_item_count() {
        // Click where a 7th tile would be, but only 3 items exist
        assert_eq!(index_at(area(), 1, TILE_H + 1, 3), None);
    }

    #[test]
    fn index_at_offset_area() {
        let a = Rect::new(10, 5, 90, 36);
        // Click before the area
        assert_eq!(index_at(a, 5, 5, 3), None);
        // Click inside the area
        assert_eq!(index_at(a, 11, 6, 3), Some(0));
    }

    // --- tile_rect ---

    #[test]
    fn tile_rect_first() {
        let a = area();
        let tile_w = a.width / COLS;
        let r = tile_rect(a, 0, COLS as usize, tile_w);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, tile_w);
        assert_eq!(r.height, TILE_H);
    }

    #[test]
    fn tile_rect_second_row_first_col() {
        let a = area();
        let tile_w = a.width / COLS;
        let r = tile_rect(a, 3, COLS as usize, tile_w);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, TILE_H);
    }

    #[test]
    fn tile_rect_with_offset_area() {
        let a = Rect::new(10, 5, 90, 36);
        let tile_w = a.width / COLS;
        let r = tile_rect(a, 1, COLS as usize, tile_w);
        assert_eq!(r.x, 10 + tile_w);
        assert_eq!(r.y, 5);
    }

    // --- truncate_end ---

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate_end("hello", 10), "hello");
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(truncate_end("hello", 5), "hello");
    }

    #[test]
    fn truncate_long_string() {
        assert_eq!(truncate_end("hello world", 6), "hello…");
    }

    #[test]
    fn truncate_max_zero() {
        assert_eq!(truncate_end("hello", 0), "…");
    }

    #[test]
    fn truncate_max_one() {
        assert_eq!(truncate_end("hello", 1), "…");
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(truncate_end("", 5), "");
    }

    // --- make_ws_summary helper ---

    fn make_ws_summary(attention: AttentionLevel) -> WorkspaceSummary {
        WorkspaceSummary {
            id: uuid::Uuid::new_v4(),
            name: "test".into(),
            path: "/tmp/test".into(),
            branch: Some("main".into()),
            ahead: Some(0),
            behind: Some(0),
            dirty_files: 0,
            attention,
            agent_running: false,
            shell_running: false,
            last_activity_unix_ms: 0,
            ssh_host: None,
        }
    }

    // --- tile_border_style tests ---

    #[test]
    fn tile_border_attention_disabled_selected() {
        let ws = make_ws_summary(AttentionLevel::None);
        let style = tile_border_style(&ws, true, false, false);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tile_border_attention_disabled_not_selected() {
        let ws = make_ws_summary(AttentionLevel::None);
        let style = tile_border_style(&ws, false, false, false);
        assert_eq!(style.fg, Some(Color::White));
    }

    #[test]
    fn tile_border_error_flash_on_not_selected() {
        let ws = make_ws_summary(AttentionLevel::Error);
        let style = tile_border_style(&ws, false, true, true);
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tile_border_error_flash_off_not_selected() {
        let ws = make_ws_summary(AttentionLevel::Error);
        let style = tile_border_style(&ws, false, false, true);
        assert_eq!(style.fg, Some(Color::LightRed));
    }

    #[test]
    fn tile_border_needs_input_flash_on_not_selected() {
        let ws = make_ws_summary(AttentionLevel::NeedsInput);
        let style = tile_border_style(&ws, false, true, true);
        assert_eq!(style.fg, Some(ORANGE));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tile_border_needs_input_flash_off_not_selected() {
        let ws = make_ws_summary(AttentionLevel::NeedsInput);
        let style = tile_border_style(&ws, false, false, true);
        assert_eq!(style.fg, Some(Color::White));
    }

    #[test]
    fn tile_border_none_selected_attention_enabled() {
        let ws = make_ws_summary(AttentionLevel::None);
        let style = tile_border_style(&ws, true, false, true);
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tile_border_error_flash_on_selected() {
        let ws = make_ws_summary(AttentionLevel::Error);
        let style = tile_border_style(&ws, true, true, true);
        // Attention overrides selection when flashing
        assert_eq!(style.fg, Some(Color::Red));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn tile_border_error_flash_off_selected() {
        let ws = make_ws_summary(AttentionLevel::Error);
        let style = tile_border_style(&ws, true, false, true);
        // Selection wins when not flashing
        assert_eq!(style.fg, Some(Color::LightBlue));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    // --- build_status_badge tests ---

    #[test]
    fn status_badge_needs_input() {
        let line = build_status_badge(&AttentionLevel::NeedsInput, true, true);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("input"));
    }

    #[test]
    fn status_badge_error() {
        let line = build_status_badge(&AttentionLevel::Error, true, true);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("error"));
    }

    #[test]
    fn status_badge_none_is_empty() {
        let line = build_status_badge(&AttentionLevel::None, true, true);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "");
    }

    #[test]
    fn status_badge_attention_disabled_always_empty() {
        let line = build_status_badge(&AttentionLevel::Error, true, false);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert_eq!(text, "");
    }

    // --- build_branch_line tests ---

    #[test]
    fn branch_line_ahead() {
        let mut ws = make_ws_summary(AttentionLevel::None);
        ws.ahead = Some(2);
        ws.behind = Some(0);
        let line = build_branch_line(&ws, 40);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("\u{2191}2")); // ↑2
    }

    #[test]
    fn branch_line_behind() {
        let mut ws = make_ws_summary(AttentionLevel::None);
        ws.ahead = Some(0);
        ws.behind = Some(3);
        let line = build_branch_line(&ws, 40);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("\u{2193}3")); // ↓3
    }

    #[test]
    fn branch_line_in_sync() {
        let ws = make_ws_summary(AttentionLevel::None);
        let line = build_branch_line(&ws, 40);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("="));
    }

    #[test]
    fn branch_line_no_tracking() {
        let mut ws = make_ws_summary(AttentionLevel::None);
        ws.ahead = None;
        ws.behind = None;
        let line = build_branch_line(&ws, 40);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        // No ahead/behind indicators when tracking info is absent
        assert!(!text.contains("\u{2191}")); // no ↑
        assert!(!text.contains("\u{2193}")); // no ↓
        assert!(!text.contains("="));
    }

    // --- build_path_line tests ---

    #[test]
    fn path_line_local() {
        let ws = make_ws_summary(AttentionLevel::None);
        let line = build_path_line(&ws, 40);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("/tmp/test"));
        assert!(!text.contains(":")); // no ssh prefix
    }

    #[test]
    fn path_line_ssh() {
        let mut ws = make_ws_summary(AttentionLevel::None);
        ws.ssh_host = Some("server".to_string());
        let line = build_path_line(&ws, 60);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("server:"));
    }
}
