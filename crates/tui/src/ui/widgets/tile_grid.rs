//! Tile geometry helpers retained after the home tile-grid was replaced by the
//! sidebar. The rendering pipeline is gone; these layout utilities still back the
//! home scroll clamp in `TuiApp::ensure_home_selected_visible`, and `ORANGE` is
//! the shared attention colour.

use ratatui::style::Color;
use std::collections::HashSet;

/// Collapsed tile height.
pub const TILE_H: u16 = 7;
/// Shared attention colour (orange), used across the sidebar, workspace bar,
/// and detail panes.
pub const ORANGE: Color = Color::Rgb(255, 165, 0);

/// Computes the expanded tile height from the configured preview line count.
pub fn tile_h_expanded(preview_lines: u16) -> u16 {
    preview_lines + 6
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

#[cfg(test)]
mod tests {
    use super::*;

    fn no_expand() -> HashSet<usize> {
        HashSet::new()
    }

    #[test]
    fn tile_y_offset_stacks_by_tile_height() {
        assert_eq!(tile_y_offset(0, &no_expand(), 18), 0);
        assert_eq!(tile_y_offset(1, &no_expand(), 18), TILE_H);
        assert_eq!(tile_y_offset(3, &no_expand(), 18), TILE_H * 3);
    }

    #[test]
    fn tile_y_offset_accounts_for_expanded() {
        let mut exp = HashSet::new();
        exp.insert(0usize);
        assert_eq!(tile_y_offset(1, &exp, 18), 18);
    }
}
