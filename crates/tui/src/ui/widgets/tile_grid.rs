use protocol::{AttentionLevel, WorkspaceSummary};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
    Frame,
};

use std::collections::HashSet;

/// Collapsed tile height — metadata only (branch, path, stats).
pub const TILE_H: u16 = 7;
pub const ORANGE: Color = Color::Rgb(255, 165, 0);

/// Computes the expanded tile height from the configured preview line count.
/// Layout: 3 info lines + 1 divider + preview_lines + 2 borders.
pub fn tile_h_expanded(preview_lines: u16) -> u16 {
    preview_lines + 6
}

/// Pairs a workspace summary with optional terminal preview lines for tile rendering.
pub struct TileData<'a> {
    pub summary: &'a WorkspaceSummary,
    pub preview: Vec<Line<'static>>,
}

/// Renders the workspace tiles as a single-column vertical list into `area`.
///
/// Each workspace in `items` is displayed as a full-width rounded card.
/// `selected` highlights the focused tile; `flash_on` drives attention pulse.
/// `expanded` contains the indices of tiles shown in expanded mode.
/// `scroll_offset` is the vertical pixel offset for scrolling.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    items: &[TileData],
    selected: usize,
    flash_on: bool,
    attention_enabled: bool,
    expanded: &HashSet<usize>,
    expanded_h: u16,
    scroll_offset: u16,
) {
    if items.is_empty() {
        render_empty_state(frame, area);
        return;
    }

    let total_h = total_height(items.len(), expanded, expanded_h);
    let needs_scrollbar = total_h > area.height;
    let tile_area = if needs_scrollbar {
        Rect {
            width: area.width.saturating_sub(1),
            ..area
        }
    } else {
        area
    };

    let mut virtual_y: u16 = 0;
    for (i, td) in items.iter().enumerate() {
        let is_expanded = expanded.contains(&i);
        let tile_h = if is_expanded { expanded_h } else { TILE_H };

        // Visible range in viewport coordinates
        let vis_top = virtual_y.saturating_sub(scroll_offset);
        let vis_bottom = (virtual_y + tile_h).saturating_sub(scroll_offset);

        virtual_y += tile_h;

        // Skip tiles entirely above or below the viewport
        if vis_bottom == 0 || vis_top >= tile_area.height {
            continue;
        }

        let tile = Rect {
            x: tile_area.x,
            y: tile_area.y + vis_top,
            width: tile_area.width,
            height: vis_bottom
                .saturating_sub(vis_top)
                .min(tile_area.height - vis_top),
        };

        if tile.width < 8 || tile.height < TILE_H.min(tile_h) {
            continue;
        }
        render_tile(
            frame,
            tile,
            td.summary,
            &td.preview,
            i == selected,
            is_expanded,
            flash_on,
            attention_enabled,
        );
    }

    // Scrollbar
    if needs_scrollbar {
        let max_offset = total_h.saturating_sub(area.height);
        let mut scrollbar_state =
            ScrollbarState::new(max_offset as usize).position(scroll_offset as usize);
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(Style::default().fg(Color::DarkGray))
                .thumb_style(Style::default().fg(Color::White)),
            area,
            &mut scrollbar_state,
        );
    }
}

/// Returns the tile index at pixel coordinate (`x`, `y`) within `area`,
/// or `None` if the coordinate falls outside all tiles.
pub fn index_at(
    area: Rect,
    x: u16,
    y: u16,
    item_count: usize,
    expanded: &HashSet<usize>,
    expanded_h: u16,
    scroll_offset: u16,
) -> Option<usize> {
    if item_count == 0 {
        return None;
    }
    if x < area.x || y < area.y || x >= area.right() || y >= area.bottom() {
        return None;
    }
    let rel_y = (y - area.y) + scroll_offset;
    let mut virtual_y: u16 = 0;
    for i in 0..item_count {
        let tile_h = if expanded.contains(&i) {
            expanded_h
        } else {
            TILE_H
        };
        if rel_y >= virtual_y && rel_y < virtual_y + tile_h {
            return Some(i);
        }
        virtual_y += tile_h;
    }
    None
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

/// Returns the virtual Y offset of the tile at `index` (before scroll).
pub fn tile_y_offset(index: usize, expanded: &HashSet<usize>, expanded_h: u16) -> u16 {
    let mut y: u16 = 0;
    for i in 0..index {
        y += if expanded.contains(&i) {
            expanded_h
        } else {
            TILE_H
        };
    }
    y
}

/// Returns the total virtual height of all tiles.
pub fn total_height(item_count: usize, expanded: &HashSet<usize>, expanded_h: u16) -> u16 {
    tile_y_offset(item_count, expanded, expanded_h)
}

/// Computes the `Rect` for tile at `index` in the visible viewport,
/// accounting for expansion and scroll offset.
pub fn tile_rect(
    area: Rect,
    index: usize,
    expanded: &HashSet<usize>,
    expanded_h: u16,
    scroll_offset: u16,
) -> Rect {
    let virtual_y = tile_y_offset(index, expanded, expanded_h);
    let tile_h = if expanded.contains(&index) {
        expanded_h
    } else {
        TILE_H
    };
    let vis_top = virtual_y.saturating_sub(scroll_offset);
    let vis_bottom = (virtual_y + tile_h).saturating_sub(scroll_offset);
    Rect {
        x: area.x,
        y: area.y + vis_top,
        width: area.width,
        height: vis_bottom
            .saturating_sub(vis_top)
            .min(area.height.saturating_sub(vis_top)),
    }
}

/// Renders a single workspace tile into `tile`.
fn render_tile(
    frame: &mut Frame,
    tile: Rect,
    ws: &WorkspaceSummary,
    preview: &[Line<'static>],
    is_selected: bool,
    is_expanded: bool,
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
    let interior_rows = tile.height.saturating_sub(2) as usize;
    let body_lines = build_body_lines(ws, body_max, preview, is_expanded, interior_rows);

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

/// Builds the interior body lines for a workspace tile.
///
/// In collapsed mode (5 interior lines): branch, path, stats, and padding.
/// In expanded mode: branch, path, stats, divider, preview lines, padding.
fn build_body_lines(
    ws: &WorkspaceSummary,
    body_max: usize,
    preview: &[Line<'static>],
    is_expanded: bool,
    interior_rows: usize,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        build_branch_line(ws, body_max),
        build_path_line(ws, body_max),
        build_stats_line(ws),
    ];

    if is_expanded {
        lines.push(Line::from(Span::styled("╶".repeat(body_max), dim_style())));

        let preview_slots = interior_rows.saturating_sub(4); // 3 info + 1 divider
        if preview.is_empty() {
            lines.push(Line::from(Span::styled(" No output yet.", dim_style())));
        } else {
            for i in 0..preview_slots.min(preview.len()) {
                lines.push(truncate_line(&preview[i], body_max));
            }
        }
    }

    // Pad to fill interior
    while lines.len() < interior_rows {
        lines.push(Line::from(""));
    }

    lines
}

/// Truncates a styled `Line` to fit within `max_width` visible characters.
fn truncate_line(line: &Line<'static>, max_width: usize) -> Line<'static> {
    let mut result_spans = Vec::new();
    let mut width = 0;
    for span in &line.spans {
        if width >= max_width {
            break;
        }
        let remaining = max_width - width;
        let char_count = span.content.chars().count();
        if char_count <= remaining {
            result_spans.push(span.clone());
            width += char_count;
        } else {
            let truncated: String = span.content.chars().take(remaining).collect();
            width += remaining;
            result_spans.push(Span::styled(truncated, span.style));
        }
    }
    Line::from(result_spans)
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

    fn no_expand() -> HashSet<usize> {
        HashSet::new()
    }

    // --- index_at ---

    #[test]
    fn index_at_first_tile() {
        assert_eq!(index_at(area(), 1, 1, 6, &no_expand(), 18, 0), Some(0));
    }

    #[test]
    fn index_at_second_tile() {
        assert_eq!(
            index_at(area(), 1, TILE_H + 1, 6, &no_expand(), 18, 0),
            Some(1)
        );
    }

    #[test]
    fn index_at_outside_area() {
        assert_eq!(index_at(area(), 200, 200, 6, &no_expand(), 18, 0), None);
    }

    #[test]
    fn index_at_zero_items() {
        assert_eq!(index_at(area(), 1, 1, 0, &no_expand(), 18, 0), None);
    }

    #[test]
    fn index_at_beyond_item_count() {
        // Click where a 4th tile would be, but only 3 items exist
        assert_eq!(
            index_at(area(), 1, TILE_H * 3 + 1, 3, &no_expand(), 18, 0),
            None
        );
    }

    #[test]
    fn index_at_offset_area() {
        let a = Rect::new(10, 5, 90, 36);
        // Click before the area
        assert_eq!(index_at(a, 5, 5, 3, &no_expand(), 18, 0), None);
        // Click inside the area
        assert_eq!(index_at(a, 11, 6, 3, &no_expand(), 18, 0), Some(0));
    }

    #[test]
    fn index_at_with_scroll() {
        // Scroll past the first tile; clicking at y=1 should hit tile 1
        assert_eq!(index_at(area(), 1, 1, 6, &no_expand(), 18, TILE_H), Some(1));
    }

    // --- tile_rect ---

    #[test]
    fn tile_rect_first() {
        let a = area();
        let r = tile_rect(a, 0, &no_expand(), 18, 0);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, 0);
        assert_eq!(r.width, a.width);
        assert_eq!(r.height, TILE_H);
    }

    #[test]
    fn tile_rect_second() {
        let a = area();
        let r = tile_rect(a, 1, &no_expand(), 18, 0);
        assert_eq!(r.x, 0);
        assert_eq!(r.y, TILE_H);
        assert_eq!(r.width, a.width);
    }

    #[test]
    fn tile_rect_with_offset_area() {
        let a = Rect::new(10, 5, 90, 36);
        let r = tile_rect(a, 1, &no_expand(), 18, 0);
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 5 + TILE_H);
    }

    // --- expansion ---

    #[test]
    fn expanded_tile_gets_taller_height() {
        let a = area();
        let exp = HashSet::from([0]);
        let r = tile_rect(a, 0, &exp, 18, 0);
        assert_eq!(r.height, 18);
    }

    #[test]
    fn tile_after_expanded_is_offset() {
        let a = area();
        let exp = HashSet::from([0]);
        let r = tile_rect(a, 1, &exp, 18, 0);
        assert_eq!(r.y, 18);
        assert_eq!(r.height, TILE_H);
    }

    // --- total_height / tile_y_offset ---

    #[test]
    fn total_height_no_expansion() {
        assert_eq!(total_height(3, &no_expand(), 18), TILE_H * 3);
    }

    #[test]
    fn total_height_with_expansion() {
        let exp = HashSet::from([1]);
        assert_eq!(total_height(3, &exp, 18), TILE_H + 18 + TILE_H);
    }

    #[test]
    fn tile_y_offset_first() {
        assert_eq!(tile_y_offset(0, &no_expand(), 18), 0);
    }

    #[test]
    fn tile_y_offset_after_expanded() {
        let exp = HashSet::from([0]);
        assert_eq!(tile_y_offset(1, &exp, 18), 18);
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

    // --- build_body_lines tests ---

    #[test]
    fn body_lines_collapsed_no_preview() {
        let ws = make_ws_summary(AttentionLevel::None);
        let lines = build_body_lines(&ws, 30, &[], false, 5);
        assert_eq!(lines.len(), 5);
        // No divider in collapsed mode
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.to_string()))
            .collect();
        assert!(!all_text.contains("╶"));
    }

    #[test]
    fn body_lines_expanded_with_preview() {
        let ws = make_ws_summary(AttentionLevel::None);
        let preview = vec![
            Line::from("line 1"),
            Line::from("line 2"),
            Line::from("line 3"),
        ];
        let lines = build_body_lines(&ws, 30, &preview, true, 16);
        assert_eq!(lines.len(), 16);
        // Should contain divider
        let divider_text: String = lines[3]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(divider_text.contains("╶"));
        // Should contain preview content
        let preview_text: String = lines[4]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(preview_text.contains("line 1"));
    }

    #[test]
    fn body_lines_expanded_empty_preview() {
        let ws = make_ws_summary(AttentionLevel::None);
        let lines = build_body_lines(&ws, 30, &[], true, 16);
        assert_eq!(lines.len(), 16);
        let no_output_text: String = lines[4]
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect();
        assert!(no_output_text.contains("No output yet."));
    }
}
