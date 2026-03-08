use protocol::{AttentionLevel, WorkspaceSummary};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub const TILE_W: u16 = 56;
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

    let cols = (area.width / TILE_W).max(1) as usize;
    for (i, ws) in items.iter().enumerate() {
        let tile = tile_rect(area, i, cols);
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
    let cols = (area.width / TILE_W).max(1) as usize;
    let col = (rel_x / TILE_W) as usize;
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
fn tile_rect(area: Rect, index: usize, cols: usize) -> Rect {
    let row = index / cols;
    let col = index % cols;
    Rect {
        x: area.x + (col as u16 * TILE_W),
        y: area.y + (row as u16 * TILE_H),
        width: TILE_W.min(area.width.saturating_sub(col as u16 * TILE_W)),
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
    let body_lines = build_body_lines(ws);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .border_style(border_style)
        .title_top(title_left)
        .title_top(title_right.right_aligned());

    frame.render_widget(Paragraph::new(body_lines).block(block), tile);
}

/// Computes the border style based on attention level, selection, and flash phase.
fn tile_border_style(ws: &WorkspaceSummary, is_selected: bool, flash_on: bool, attention_enabled: bool) -> Style {
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
fn build_status_badge(attention: &AttentionLevel, flash_on: bool, attention_enabled: bool) -> Line<'static> {
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
fn build_body_lines(ws: &WorkspaceSummary) -> Vec<Line<'static>> {
    vec![
        Line::from(""),
        build_branch_line(ws),
        build_path_line(ws),
        Line::from(""),
        build_stats_line(ws),
        Line::from(""),
        Line::from(""),
    ]
}

/// Max text width inside a tile: TILE_W minus 2 border columns and 4 icon-prefix columns.
const BODY_TEXT_MAX: usize = (TILE_W as usize) - 6;

fn build_branch_line(ws: &WorkspaceSummary) -> Line<'static> {
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
            truncate_end(branch, BODY_TEXT_MAX.saturating_sub(ab.len())),
            Style::default().fg(Color::White),
        ),
        Span::styled(ab, ab_style),
    ])
}

fn build_path_line(ws: &WorkspaceSummary) -> Line<'static> {
    let dim = dim_style();
    Line::from(vec![
        Span::styled("  ", dim),
        Span::styled(truncate_end(&ws.path, BODY_TEXT_MAX), dim),
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
