use protocol::{AttentionLevel, WorkspaceId, WorkspaceSummary};
use ratatui::layout::Rect;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

use super::tile_grid::ORANGE;

/// Builds a single-line status bar showing all workspaces as compact pills.
///
/// The active workspace is highlighted; others show attention state with flash animation.
pub fn build_workspace_bar_line(
    workspaces: &[WorkspaceSummary],
    active_id: Option<WorkspaceId>,
    flash_on: bool,
    attention_enabled: bool,
    max_width: u16,
    selected_idx: Option<usize>,
) -> Line<'static> {
    if workspaces.is_empty() {
        return Line::from("");
    }

    let max_w = max_width as usize;

    // Build pill data for each workspace.
    let mut pills: Vec<PillData> = workspaces
        .iter()
        .enumerate()
        .map(|(i, ws)| {
            let is_active = active_id == Some(ws.id);
            let attention = if attention_enabled {
                ws.attention
            } else {
                AttentionLevel::None
            };
            let is_selected = selected_idx == Some(i);
            PillData {
                name: ws.name.clone(),
                is_active,
                is_selected,
                attention,
                agent_running: ws.agent_running,
            }
        })
        .collect();

    // Calculate total width needed: each pill = " {icon}{name}{dot} ", dividers = " │ " (3 chars)
    let divider_width = 3;
    let total_dividers = pills.len().saturating_sub(1) * divider_width;

    let pill_widths = |pills: &[PillData]| -> usize {
        pills.iter().map(|p| p.display_width()).sum::<usize>() + total_dividers
    };

    // Truncate names if they don't fit.
    if pill_widths(&pills) > max_w {
        truncate_pills(&mut pills, max_w, total_dividers);
    }

    // If still too wide after truncation, show only active + count.
    if pill_widths(&pills) > max_w {
        return build_overflow_line(&pills, flash_on, max_w);
    }

    // Build spans.
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, pill) in pills.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        }
        pill.push_spans(&mut spans, flash_on);
    }

    Line::from(spans)
}

struct PillData {
    name: String,
    is_active: bool,
    is_selected: bool,
    attention: AttentionLevel,
    agent_running: bool,
}

impl PillData {
    fn icon(&self) -> &'static str {
        match self.attention {
            AttentionLevel::NeedsInput => "⚠ ",
            AttentionLevel::Error => "✖ ",
            _ => "",
        }
    }

    fn dot(&self) -> &'static str {
        if self.agent_running {
            " ●"
        } else {
            ""
        }
    }

    /// Total display width of this pill: " {icon}{name}{dot} "
    fn display_width(&self) -> usize {
        // Leading space + icon + name + dot + trailing space
        1 + self.icon().chars().count() + self.name.chars().count() + self.dot().chars().count() + 1
    }

    fn base_style(&self, flash_on: bool) -> Style {
        if self.is_selected && !self.is_active {
            return Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::UNDERLINED);
        }
        if self.is_active {
            return Style::default()
                .fg(Color::LightBlue)
                .add_modifier(Modifier::BOLD);
        }
        match self.attention {
            AttentionLevel::NeedsInput => {
                let s = Style::default().fg(ORANGE);
                if flash_on {
                    s.add_modifier(Modifier::BOLD)
                } else {
                    s
                }
            }
            AttentionLevel::Error => {
                let s = Style::default().fg(Color::Red);
                if flash_on {
                    s.add_modifier(Modifier::BOLD)
                } else {
                    s
                }
            }
            _ => Style::default().fg(Color::DarkGray),
        }
    }

    fn push_spans(&self, spans: &mut Vec<Span<'static>>, flash_on: bool) {
        let style = self.base_style(flash_on);

        // " {icon}{name}"
        let text = format!(" {}{}", self.icon(), self.name);
        spans.push(Span::styled(text, style));

        // Agent dot in green (unless active, where we keep the pill style for the bg)
        if self.agent_running {
            let dot_style = if self.is_active {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Green)
            };
            spans.push(Span::styled(" ●", dot_style));
        }

        // Trailing space
        spans.push(Span::styled(" ", style));
    }
}

/// Progressively truncate the longest non-active names to fit within max_width.
fn truncate_pills(pills: &mut [PillData], max_width: usize, total_dividers: usize) {
    let min_name_len = 3; // "ab…"

    loop {
        let current: usize =
            pills.iter().map(|p| p.display_width()).sum::<usize>() + total_dividers;
        if current <= max_width {
            break;
        }

        // Find the longest non-active name that can still be shortened.
        let target = pills
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.is_active && p.name.chars().count() > min_name_len)
            .max_by_key(|(_, p)| p.name.chars().count());

        match target {
            Some((idx, _)) => {
                let name = &pills[idx].name;
                // Shorten by removing last char and adding ellipsis.
                let mut chars: Vec<char> = name.chars().collect();
                // Remove trailing '…' if already present, then shorten.
                if chars.last() == Some(&'…') {
                    chars.pop();
                }
                if chars.len() > min_name_len - 1 {
                    chars.truncate(chars.len() - 1);
                }
                chars.push('…');
                pills[idx].name = chars.into_iter().collect();
            }
            None => {
                // All non-active names are at minimum; try the active name.
                if let Some(active) = pills.iter_mut().find(|p| p.is_active) {
                    let mut chars: Vec<char> = active.name.chars().collect();
                    if chars.last() == Some(&'…') {
                        chars.pop();
                    }
                    if chars.len() > min_name_len - 1 {
                        chars.truncate(chars.len() - 1);
                        chars.push('…');
                        active.name = chars.into_iter().collect();
                    } else {
                        break; // Can't shrink further.
                    }
                } else {
                    break;
                }
            }
        }
    }
}

/// Fallback when even truncation can't fit: show only the active pill + " +N".
fn build_overflow_line(pills: &[PillData], flash_on: bool, _max_width: usize) -> Line<'static> {
    let mut spans = Vec::new();
    let others = pills.len().saturating_sub(1);

    if let Some(active) = pills.iter().find(|p| p.is_active) {
        active.push_spans(&mut spans, flash_on);
    }

    // Check if any non-active workspace needs attention.
    let has_attention = pills.iter().any(|p| {
        !p.is_active
            && matches!(
                p.attention,
                AttentionLevel::NeedsInput | AttentionLevel::Error
            )
    });

    if others > 0 {
        let badge = format!(" +{}", others);
        let style = if has_attention && flash_on {
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        spans.push(Span::styled(badge, style));
    }

    Line::from(spans)
}

/// Returns the workspace index for the pill at position (x, y) within the bar rect.
/// Used for mouse hit testing.
pub fn pill_index_at(bar: Rect, workspaces: &[WorkspaceSummary], x: u16, y: u16) -> Option<usize> {
    if workspaces.is_empty() || y < bar.y || y >= bar.bottom() || x < bar.x || x >= bar.right() {
        return None;
    }

    let divider_width: u16 = 3;
    let mut cursor = bar.x;

    for (i, ws) in workspaces.iter().enumerate() {
        if i > 0 {
            cursor += divider_width;
        }
        let icon_w: u16 = match ws.attention {
            AttentionLevel::NeedsInput | AttentionLevel::Error => 2,
            _ => 0,
        };
        let dot_w: u16 = if ws.agent_running { 2 } else { 0 };
        let pill_w = 1 + icon_w + ws.name.chars().count() as u16 + dot_w + 1;
        let end = cursor + pill_w;

        if x >= cursor && x < end {
            return Some(i);
        }
        cursor = end;
    }
    None
}
