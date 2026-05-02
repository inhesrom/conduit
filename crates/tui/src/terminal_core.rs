use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alacritty_terminal::{
    event::{Event as AlacrittyEvent, EventListener, WindowSize},
    grid::{Dimensions, Scroll},
    term::{
        cell::{Cell as AlacrittyCell, Flags as AlacrittyFlags},
        color::{Colors as AlacrittyColors, COUNT as ALACRITTY_COLOR_COUNT},
        point_to_viewport, Config as AlacrittyConfig, Term as AlacrittyTerm,
    },
    vte::ansi::{
        Color as AlacrittyColor, NamedColor as AlacrittyNamedColor,
        Processor as AlacrittyProcessor, Rgb as AlacrittyRgb,
    },
};
use ratatui::{
    style::{Color as TuiColor, Modifier, Style},
    text::{Line, Span},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

const DEFAULT_COLS: u16 = 120;
const DEFAULT_ROWS: u16 = 24;
const ALACRITTY_SCROLLBACK: usize = 8000;
const INNER_DEFAULT_FG: AlacrittyRgb = AlacrittyRgb {
    r: 229,
    g: 229,
    b: 229,
};
const INNER_DEFAULT_BG: AlacrittyRgb = AlacrittyRgb { r: 0, g: 0, b: 0 };
/// Maximum raw PTY bytes retained so terminal state can be rebuilt after resize or core changes.
pub const MAX_TERMINAL_HISTORY_BYTES: usize = 2 * 1024 * 1024;

/// Selects the parser/emulator used to turn raw PTY bytes into renderable terminal cells.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalCoreKind {
    /// Alacritty's terminal core, used by default for higher-fidelity ANSI emulation.
    Alacritty,
    /// The previous vt100 parser, retained as an experimental fallback.
    Vt100,
}

impl Default for TerminalCoreKind {
    fn default() -> Self {
        Self::Alacritty
    }
}

impl TerminalCoreKind {
    /// Returns the settings label shown to users.
    pub fn label(self) -> &'static str {
        match self {
            Self::Alacritty => "alacritty",
            Self::Vt100 => "vt100",
        }
    }

    /// Cycles to the next available core.
    pub fn cycle(self, delta: i16) -> Self {
        match (self, delta >= 0) {
            (Self::Alacritty, true) | (Self::Alacritty, false) => Self::Vt100,
            (Self::Vt100, true) | (Self::Vt100, false) => Self::Alacritty,
        }
    }
}

#[derive(Clone, Copy)]
struct TerminalSize {
    cols: u16,
    rows: u16,
}

impl TerminalSize {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }

    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows,
            num_cols: self.cols,
            cell_width: 1,
            cell_height: 1,
        }
    }
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn columns(&self) -> usize {
        self.cols as usize
    }
}

#[derive(Clone, Default)]
struct AlacrittyEventProxy {
    events: Arc<Mutex<Vec<AlacrittyEvent>>>,
}

impl AlacrittyEventProxy {
    fn drain(&self) -> Vec<AlacrittyEvent> {
        self.events
            .lock()
            .map(|mut events| events.drain(..).collect())
            .unwrap_or_default()
    }
}

impl EventListener for AlacrittyEventProxy {
    fn send_event(&self, event: AlacrittyEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push(event);
        }
    }
}

/// Holds terminal emulation state for every tab in a workspace.
pub struct WorkspaceTerminalState {
    /// Per-tab render buffers keyed by terminal tab id.
    pub tabs: HashMap<String, TerminalBufferState>,
}

impl WorkspaceTerminalState {
    /// Creates the default agent and shell buffers for a workspace.
    pub fn new(kind: TerminalCoreKind) -> Self {
        let mut tabs = HashMap::new();
        tabs.insert("agent".to_string(), TerminalBufferState::new(kind));
        tabs.insert("shell".to_string(), TerminalBufferState::new(kind));
        Self { tabs }
    }

    /// Returns an existing tab buffer or creates one using the current terminal core.
    pub fn tab_mut(&mut self, tab_id: &str, kind: TerminalCoreKind) -> &mut TerminalBufferState {
        let tab = self
            .tabs
            .entry(tab_id.to_string())
            .or_insert_with(|| TerminalBufferState::new(kind));
        tab.ensure_core_kind(kind);
        tab
    }

    /// Rebuilds all tab buffers when the configured terminal core changes.
    pub fn ensure_core_kind(&mut self, kind: TerminalCoreKind) {
        for tab in self.tabs.values_mut() {
            tab.ensure_core_kind(kind);
        }
    }
}

/// Retains raw PTY history and a live terminal emulator for one tab.
pub struct TerminalBufferState {
    core_kind: TerminalCoreKind,
    core: TerminalCore,
    history: Vec<u8>,
    cols: u16,
    rows: u16,
}

impl TerminalBufferState {
    /// Creates an empty terminal buffer using the requested core.
    pub fn new(kind: TerminalCoreKind) -> Self {
        Self {
            core_kind: kind,
            core: TerminalCore::new(kind, DEFAULT_COLS, DEFAULT_ROWS),
            history: Vec::new(),
            cols: DEFAULT_COLS,
            rows: DEFAULT_ROWS,
        }
    }

    /// Replays retained PTY history into a new core if the selected core has changed.
    pub fn ensure_core_kind(&mut self, kind: TerminalCoreKind) {
        if self.core_kind != kind {
            self.core_kind = kind;
            self.rebuild();
        }
    }

    /// Appends raw PTY bytes, updates the emulator, and returns response bytes for the PTY.
    pub fn append_bytes(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.history.extend_from_slice(bytes);
        if self.history.len() > MAX_TERMINAL_HISTORY_BYTES {
            let trim = self.history.len() - MAX_TERMINAL_HISTORY_BYTES;
            self.history.drain(..trim);
        }
        self.core.process(bytes)
    }

    /// Clears the emulator and retained replay history.
    pub fn reset(&mut self) {
        self.core = TerminalCore::new(self.core_kind, self.cols, self.rows);
        self.history.clear();
    }

    /// Rebuilds the emulator at a new cell size using retained replay history.
    pub fn rebuild_for_size(&mut self, cols: u16, rows: u16) {
        self.cols = cols.max(1);
        self.rows = rows.max(1);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.core = TerminalCore::new(self.core_kind, self.cols, self.rows);
        let _ = self.core.process(&self.history);
    }

    /// Scrolls the emulator's display viewport by terminal rows.
    pub fn scroll_scrollback(&mut self, delta: isize) {
        self.core.scroll_scrollback(delta);
    }

    /// Returns the emulator viewport to the live bottom of the terminal.
    pub fn reset_scrollback(&mut self) {
        self.core.reset_scrollback();
    }

    /// Returns true when the viewport is not at the live bottom of the terminal.
    pub fn scrollback_active(&self) -> bool {
        self.core.scrollback_active()
    }

    /// Renders the visible terminal rows as styled Ratatui lines.
    pub fn lines(&self, url_re: &Regex) -> Vec<Line<'static>> {
        self.core.lines(url_re)
    }

    /// Returns a plain-text row from the visible terminal viewport.
    pub fn plain_row(&self, row: u16) -> Option<String> {
        self.core.plain_row(row)
    }

    /// Returns the last visible row as plain text.
    pub fn last_row_text(&self) -> String {
        self.core.last_row_text()
    }

    /// Returns the most recent terminal title reported by the parser.
    pub fn title(&self) -> &str {
        self.core.title()
    }

    /// Returns the active mouse reporting mode, encoding, and alternate-screen state.
    pub fn mouse_state(&self) -> (MouseProtocolMode, MouseProtocolEncoding, bool) {
        self.core.mouse_state()
    }

    /// Extracts recent non-empty styled lines for workspace previews.
    pub fn preview_lines(&self, max_cols: u16, num_lines: u16) -> Vec<Line<'static>> {
        self.core.preview_lines(max_cols, num_lines)
    }
}

enum TerminalCore {
    Vt100(Vt100Terminal),
    Alacritty(AlacrittyTerminal),
}

impl TerminalCore {
    fn new(kind: TerminalCoreKind, cols: u16, rows: u16) -> Self {
        match kind {
            TerminalCoreKind::Vt100 => Self::Vt100(Vt100Terminal::new(cols, rows)),
            TerminalCoreKind::Alacritty => Self::Alacritty(AlacrittyTerminal::new(cols, rows)),
        }
    }

    fn process(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        match self {
            Self::Vt100(core) => core.process(bytes),
            Self::Alacritty(core) => core.process(bytes),
        }
    }

    fn scroll_scrollback(&mut self, delta: isize) {
        match self {
            Self::Vt100(core) => core.scroll_scrollback(delta),
            Self::Alacritty(core) => core.scroll_scrollback(delta),
        }
    }

    fn reset_scrollback(&mut self) {
        match self {
            Self::Vt100(core) => core.reset_scrollback(),
            Self::Alacritty(core) => core.reset_scrollback(),
        }
    }

    fn scrollback_active(&self) -> bool {
        match self {
            Self::Vt100(core) => core.scrollback_active(),
            Self::Alacritty(core) => core.scrollback_active(),
        }
    }

    fn lines(&self, url_re: &Regex) -> Vec<Line<'static>> {
        match self {
            Self::Vt100(core) => core.lines(url_re),
            Self::Alacritty(core) => core.lines(url_re),
        }
    }

    fn plain_row(&self, row: u16) -> Option<String> {
        match self {
            Self::Vt100(core) => core.plain_row(row),
            Self::Alacritty(core) => core.plain_row(row),
        }
    }

    fn last_row_text(&self) -> String {
        match self {
            Self::Vt100(core) => core.last_row_text(),
            Self::Alacritty(core) => core.last_row_text(),
        }
    }

    fn title(&self) -> &str {
        match self {
            Self::Vt100(core) => core.title(),
            Self::Alacritty(core) => core.title(),
        }
    }

    fn mouse_state(&self) -> (MouseProtocolMode, MouseProtocolEncoding, bool) {
        match self {
            Self::Vt100(core) => core.mouse_state(),
            Self::Alacritty(core) => core.mouse_state(),
        }
    }

    fn preview_lines(&self, max_cols: u16, num_lines: u16) -> Vec<Line<'static>> {
        match self {
            Self::Vt100(core) => core.preview_lines(max_cols, num_lines),
            Self::Alacritty(core) => core.preview_lines(max_cols, num_lines),
        }
    }
}

struct Vt100Terminal {
    parser: vt100::Parser,
}

impl Vt100Terminal {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows.max(1), cols.max(1), ALACRITTY_SCROLLBACK),
        }
    }

    fn process(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.parser.process(bytes);
        let mut responses = Vec::new();
        if bytes.windows(4).any(|w| w == b"\x1b[6n") {
            let (row, col) = self.parser.screen().cursor_position();
            responses.push(format!("\x1b[{};{}R", row + 1, col + 1).into_bytes());
        }
        responses
    }

    fn scroll_scrollback(&mut self, delta: isize) {
        let current = self.parser.screen().scrollback() as isize;
        let next = (current + delta).max(0) as usize;
        self.parser.set_scrollback(next);
    }

    fn reset_scrollback(&mut self) {
        self.parser.set_scrollback(0);
    }

    fn scrollback_active(&self) -> bool {
        self.parser.screen().scrollback() > 0
    }

    fn lines(&self, url_re: &Regex) -> Vec<Line<'static>> {
        let screen = self.parser.screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        // Always render the cursor — many TUIs hide the OS cursor with \e[?25l
        // and don't draw a visible caret of their own, leaving users unable
        // to see where they're typing.
        let show_cursor = true;
        let (rows, cols) = screen.size();
        let row_texts: Vec<String> = screen.rows(0, cols).collect();

        let mut lines = Vec::with_capacity(rows as usize);
        for r in 0..rows {
            let url_ranges = row_texts
                .get(r as usize)
                .map(|row| url_ranges(url_re, row))
                .unwrap_or_default();

            let mut spans = Vec::with_capacity(cols as usize);
            for c in 0..cols {
                let Some(cell) = screen.cell(r, c) else {
                    continue;
                };
                if cell.is_wide_continuation() {
                    continue;
                }
                let mut style = vt100_cell_style(cell);
                if show_cursor && r == cursor_row && c == cursor_col {
                    style = toggle_reverse_video(style);
                }
                if url_ranges.iter().any(|&(s, e)| c >= s && c < e) {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                let text = if cell.has_contents() {
                    cell.contents()
                } else {
                    " ".to_string()
                };
                spans.push(Span::styled(text, style));
            }
            lines.push(Line::from(spans));
        }
        lines
    }

    fn plain_row(&self, row: u16) -> Option<String> {
        let screen = self.parser.screen();
        let (_rows, cols) = screen.size();
        screen.rows(0, cols).nth(row as usize)
    }

    fn last_row_text(&self) -> String {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let last_row = rows.saturating_sub(1);
        let mut text = String::new();
        for c in 0..cols {
            if let Some(cell) = screen.cell(last_row, c) {
                if cell.has_contents() {
                    text.push_str(&cell.contents());
                } else {
                    text.push(' ');
                }
            }
        }
        text
    }

    fn title(&self) -> &str {
        self.parser.screen().title()
    }

    fn mouse_state(&self) -> (MouseProtocolMode, MouseProtocolEncoding, bool) {
        let screen = self.parser.screen();
        (
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
            screen.alternate_screen(),
        )
    }

    fn preview_lines(&self, max_cols: u16, num_lines: u16) -> Vec<Line<'static>> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let use_cols = cols.min(max_cols);
        let scrollback_count = screen.scrollback_count();
        let needed = num_lines as usize;
        let mut collected: Vec<Line<'static>> = Vec::new();

        for r in (0..rows).rev() {
            if collected.len() >= needed {
                break;
            }
            if vt100_screen_row_has_content(screen, r, use_cols) {
                collected.push(vt100_screen_row(screen, r, use_cols));
            }
        }

        if collected.len() < needed {
            for offset in 0..scrollback_count {
                if collected.len() >= needed {
                    break;
                }
                if vt100_scrollback_row_has_content(screen, offset, use_cols) {
                    collected.push(vt100_scrollback_row(screen, offset, use_cols));
                }
            }
        }

        collected.truncate(needed);
        collected.reverse();
        collected
    }
}

struct AlacrittyTerminal {
    parser: AlacrittyProcessor,
    term: AlacrittyTerm<AlacrittyEventProxy>,
    proxy: AlacrittyEventProxy,
    title: String,
    size: TerminalSize,
}

impl AlacrittyTerminal {
    fn new(cols: u16, rows: u16) -> Self {
        let size = TerminalSize::new(cols, rows);
        let proxy = AlacrittyEventProxy::default();
        let config = AlacrittyConfig {
            scrolling_history: ALACRITTY_SCROLLBACK,
            kitty_keyboard: true,
            ..AlacrittyConfig::default()
        };
        Self {
            parser: AlacrittyProcessor::new(),
            term: AlacrittyTerm::new(config, &size, proxy.clone()),
            proxy,
            title: String::new(),
            size,
        }
    }

    fn process(&mut self, bytes: &[u8]) -> Vec<Vec<u8>> {
        self.parser.advance(&mut self.term, bytes);
        self.drain_events()
    }

    fn drain_events(&mut self) -> Vec<Vec<u8>> {
        let mut responses = Vec::new();
        for event in self.proxy.drain() {
            match event {
                AlacrittyEvent::PtyWrite(text) => responses.push(text.into_bytes()),
                AlacrittyEvent::Title(title) => self.title = title,
                AlacrittyEvent::ResetTitle => self.title.clear(),
                AlacrittyEvent::ColorRequest(index, formatter) => {
                    let color = if index < ALACRITTY_COLOR_COUNT {
                        self.term.colors()[index].unwrap_or_else(|| default_alacritty_rgb(index))
                    } else {
                        default_alacritty_rgb(index)
                    };
                    responses.push(formatter(color).into_bytes());
                }
                AlacrittyEvent::TextAreaSizeRequest(formatter) => {
                    responses.push(formatter(self.size.window_size()).into_bytes());
                }
                _ => {}
            }
        }
        responses
    }

    fn scroll_scrollback(&mut self, delta: isize) {
        let delta = delta.clamp(i32::MIN as isize, i32::MAX as isize) as i32;
        self.term.scroll_display(Scroll::Delta(delta));
    }

    fn reset_scrollback(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    fn scrollback_active(&self) -> bool {
        self.term.grid().display_offset() > 0
    }

    fn lines(&self, url_re: &Regex) -> Vec<Line<'static>> {
        self.render_rows(url_re)
            .into_iter()
            .map(|row| row.line)
            .collect()
    }

    fn plain_row(&self, row: u16) -> Option<String> {
        self.render_rows_without_urls()
            .get(row as usize)
            .map(|row| row.text.clone())
    }

    fn last_row_text(&self) -> String {
        self.render_rows_without_urls()
            .last()
            .map(|row| row.text.clone())
            .unwrap_or_default()
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn mouse_state(&self) -> (MouseProtocolMode, MouseProtocolEncoding, bool) {
        let mode = *self.term.mode();
        let mouse_mode = if mode.contains(alacritty_terminal::term::TermMode::MOUSE_MOTION) {
            MouseProtocolMode::AnyMotion
        } else if mode.contains(alacritty_terminal::term::TermMode::MOUSE_DRAG) {
            MouseProtocolMode::ButtonMotion
        } else if mode.contains(alacritty_terminal::term::TermMode::MOUSE_REPORT_CLICK) {
            MouseProtocolMode::PressRelease
        } else {
            MouseProtocolMode::None
        };
        let encoding = if mode.contains(alacritty_terminal::term::TermMode::SGR_MOUSE) {
            MouseProtocolEncoding::Sgr
        } else if mode.contains(alacritty_terminal::term::TermMode::UTF8_MOUSE) {
            MouseProtocolEncoding::Utf8
        } else {
            MouseProtocolEncoding::Default
        };
        (
            mouse_mode,
            encoding,
            mode.contains(alacritty_terminal::term::TermMode::ALT_SCREEN),
        )
    }

    fn preview_lines(&self, max_cols: u16, num_lines: u16) -> Vec<Line<'static>> {
        let mut rows = self
            .render_rows_without_urls()
            .into_iter()
            .filter(|row| !row.text.trim().is_empty())
            .collect::<Vec<_>>();
        rows.truncate(self.size.rows as usize);
        let take = num_lines as usize;
        let start = rows.len().saturating_sub(take);
        rows.into_iter()
            .skip(start)
            .map(|row| truncate_line(row.line, max_cols))
            .collect()
    }

    fn render_rows_without_urls(&self) -> Vec<RenderedRow> {
        self.render_rows_with_url_re(None)
    }

    fn render_rows(&self, url_re: &Regex) -> Vec<RenderedRow> {
        self.render_rows_with_url_re(Some(url_re))
    }

    fn render_rows_with_url_re(&self, url_re: Option<&Regex>) -> Vec<RenderedRow> {
        let rows = self.term.screen_lines();
        let cols = self.term.columns();
        let mut cells = (0..rows)
            .map(|_| {
                (0..cols)
                    .map(|_| RenderCell {
                        text: " ".to_string(),
                        style: Style::default(),
                        skip: false,
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let content = self.term.renderable_content();
        for indexed in content.display_iter {
            let Some(point) = point_to_viewport(content.display_offset, indexed.point) else {
                continue;
            };
            if point.line >= rows || point.column.0 >= cols {
                continue;
            }
            let cell = indexed.cell;
            let flags = cell.flags;
            let row = point.line;
            let col = point.column.0;
            let target = &mut cells[row][col];
            target.skip = flags.intersects(
                AlacrittyFlags::WIDE_CHAR_SPACER | AlacrittyFlags::LEADING_WIDE_CHAR_SPACER,
            );
            target.style = alacritty_cell_style(cell, content.colors);
            target.text = alacritty_cell_text(cell);
        }

        // Always render the cursor — many TUIs hide the OS cursor with \e[?25l
        // and don't draw a visible caret of their own.
        if let Some(cursor) = point_to_viewport(content.display_offset, content.cursor.point) {
            if cursor.line < rows && cursor.column.0 < cols {
                let cell = &mut cells[cursor.line][cursor.column.0];
                cell.style = toggle_reverse_video(cell.style);
            }
        }

        let mut rendered = Vec::with_capacity(rows);
        for row in cells {
            let text = row
                .iter()
                .filter(|cell| !cell.skip)
                .map(|cell| cell.text.as_str())
                .collect::<String>();
            let ranges = url_re.map(|re| url_ranges(re, &text)).unwrap_or_default();
            let mut display_col = 0u16;
            let mut spans = Vec::with_capacity(cols);
            for mut cell in row {
                if cell.skip {
                    continue;
                }
                let width = unicode_width::UnicodeWidthStr::width(cell.text.as_str()).max(1) as u16;
                if ranges
                    .iter()
                    .any(|&(s, e)| display_col < e && display_col + width > s)
                {
                    cell.style = cell.style.add_modifier(Modifier::UNDERLINED);
                }
                display_col = display_col.saturating_add(width);
                spans.push(Span::styled(cell.text, cell.style));
            }
            rendered.push(RenderedRow {
                line: Line::from(spans),
                text,
            });
        }
        rendered
    }
}

struct RenderCell {
    text: String,
    style: Style,
    skip: bool,
}

struct RenderedRow {
    line: Line<'static>,
    text: String,
}

fn url_ranges(re: &Regex, row_text: &str) -> Vec<(u16, u16)> {
    re.find_iter(row_text)
        .map(|m| {
            let col_start = row_text[..m.start()].chars().count() as u16;
            let col_end = col_start + row_text[m.start()..m.end()].chars().count() as u16;
            (col_start, col_end)
        })
        .collect()
}

fn toggle_reverse_video(style: Style) -> Style {
    if style.add_modifier.contains(Modifier::REVERSED) {
        style.remove_modifier(Modifier::REVERSED)
    } else {
        style.add_modifier(Modifier::REVERSED)
    }
}

fn vt100_cell_style(cell: &vt100::Cell) -> Style {
    let fg = map_vt100_color(cell.fgcolor());
    let bg = map_vt100_color(cell.bgcolor());
    let mut style = Style::default().fg(fg).bg(bg);
    if cell.bold() {
        style = style.add_modifier(Modifier::BOLD);
    }
    if cell.italic() {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if cell.underline() {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if cell.inverse() {
        style = toggle_reverse_video(style);
    }
    style
}

fn map_vt100_color(color: vt100::Color) -> TuiColor {
    match color {
        vt100::Color::Default => TuiColor::Reset,
        vt100::Color::Idx(i) => TuiColor::Indexed(i),
        vt100::Color::Rgb(r, g, b) => TuiColor::Rgb(r, g, b),
    }
}

fn alacritty_cell_style(cell: &AlacrittyCell, colors: &AlacrittyColors) -> Style {
    let flags = cell.flags;
    let mut style = Style::default()
        .fg(map_alacritty_color(cell.fg, colors))
        .bg(map_alacritty_color(cell.bg, colors));
    if flags.contains(AlacrittyFlags::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if flags.contains(AlacrittyFlags::DIM) {
        style = style.add_modifier(Modifier::DIM);
    }
    if flags.contains(AlacrittyFlags::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if flags.intersects(AlacrittyFlags::ALL_UNDERLINES) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if flags.contains(AlacrittyFlags::INVERSE) {
        style = toggle_reverse_video(style);
    }
    if flags.contains(AlacrittyFlags::HIDDEN) {
        style = style.add_modifier(Modifier::HIDDEN);
    }
    if flags.contains(AlacrittyFlags::STRIKEOUT) {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
}

fn alacritty_cell_text(cell: &AlacrittyCell) -> String {
    if cell.flags.contains(AlacrittyFlags::HIDDEN) {
        return " ".to_string();
    }
    let mut text = cell.c.to_string();
    if let Some(zerowidth) = cell.zerowidth() {
        text.extend(zerowidth.iter().copied());
    }
    text
}

fn map_alacritty_color(color: AlacrittyColor, colors: &AlacrittyColors) -> TuiColor {
    match color {
        AlacrittyColor::Named(named) => colors[named]
            .map(rgb_to_tui)
            .unwrap_or_else(|| fallback_named_color(named)),
        AlacrittyColor::Spec(rgb) => rgb_to_tui(rgb),
        AlacrittyColor::Indexed(i) => TuiColor::Indexed(i),
    }
}

fn rgb_to_tui(rgb: AlacrittyRgb) -> TuiColor {
    TuiColor::Rgb(rgb.r, rgb.g, rgb.b)
}

fn fallback_named_color(named: AlacrittyNamedColor) -> TuiColor {
    if let Some(index) = named_ansi_index(named) {
        return TuiColor::Indexed(index);
    }

    match named {
        AlacrittyNamedColor::DimBlack => TuiColor::DarkGray,
        AlacrittyNamedColor::DimRed => TuiColor::Red,
        AlacrittyNamedColor::DimGreen => TuiColor::Green,
        AlacrittyNamedColor::DimYellow => TuiColor::Yellow,
        AlacrittyNamedColor::DimBlue => TuiColor::Blue,
        AlacrittyNamedColor::DimMagenta => TuiColor::Magenta,
        AlacrittyNamedColor::DimCyan => TuiColor::Cyan,
        AlacrittyNamedColor::DimWhite => TuiColor::Gray,
        AlacrittyNamedColor::Foreground
        | AlacrittyNamedColor::Background
        | AlacrittyNamedColor::Cursor
        | AlacrittyNamedColor::BrightForeground
        | AlacrittyNamedColor::DimForeground => TuiColor::Reset,
        _ => TuiColor::Reset,
    }
}

fn named_ansi_index(named: AlacrittyNamedColor) -> Option<u8> {
    match named {
        AlacrittyNamedColor::Black => Some(0),
        AlacrittyNamedColor::Red => Some(1),
        AlacrittyNamedColor::Green => Some(2),
        AlacrittyNamedColor::Yellow => Some(3),
        AlacrittyNamedColor::Blue => Some(4),
        AlacrittyNamedColor::Magenta => Some(5),
        AlacrittyNamedColor::Cyan => Some(6),
        AlacrittyNamedColor::White => Some(7),
        AlacrittyNamedColor::BrightBlack => Some(8),
        AlacrittyNamedColor::BrightRed => Some(9),
        AlacrittyNamedColor::BrightGreen => Some(10),
        AlacrittyNamedColor::BrightYellow => Some(11),
        AlacrittyNamedColor::BrightBlue => Some(12),
        AlacrittyNamedColor::BrightMagenta => Some(13),
        AlacrittyNamedColor::BrightCyan => Some(14),
        AlacrittyNamedColor::BrightWhite => Some(15),
        _ => None,
    }
}

fn default_alacritty_rgb(index: usize) -> AlacrittyRgb {
    match index {
        256 | 267 => INNER_DEFAULT_FG,
        257 | 268 => INNER_DEFAULT_BG,
        258 => INNER_DEFAULT_FG,
        0..=15 => ansi16_rgb(index as u8),
        16..=231 => color_cube_rgb(index as u8),
        232..=255 => grayscale_rgb(index as u8),
        _ => INNER_DEFAULT_FG,
    }
}

fn ansi16_rgb(index: u8) -> AlacrittyRgb {
    let (r, g, b) = match index {
        0 => (0, 0, 0),
        1 => (205, 0, 0),
        2 => (0, 205, 0),
        3 => (205, 205, 0),
        4 => (0, 0, 238),
        5 => (205, 0, 205),
        6 => (0, 205, 205),
        7 => (229, 229, 229),
        8 => (127, 127, 127),
        9 => (255, 0, 0),
        10 => (0, 255, 0),
        11 => (255, 255, 0),
        12 => (92, 92, 255),
        13 => (255, 0, 255),
        14 => (0, 255, 255),
        15 => (255, 255, 255),
        _ => (229, 229, 229),
    };
    AlacrittyRgb { r, g, b }
}

fn color_cube_rgb(index: u8) -> AlacrittyRgb {
    let offset = index.saturating_sub(16);
    let red = offset / 36;
    let green = (offset % 36) / 6;
    let blue = offset % 6;
    AlacrittyRgb {
        r: color_cube_component(red),
        g: color_cube_component(green),
        b: color_cube_component(blue),
    }
}

fn color_cube_component(value: u8) -> u8 {
    if value == 0 {
        0
    } else {
        55 + value * 40
    }
}

fn grayscale_rgb(index: u8) -> AlacrittyRgb {
    let value = 8 + index.saturating_sub(232) * 10;
    AlacrittyRgb {
        r: value,
        g: value,
        b: value,
    }
}

fn vt100_screen_row_has_content(screen: &vt100::Screen, row: u16, cols: u16) -> bool {
    (0..cols).any(|c| {
        screen
            .cell(row, c)
            .map(|cell| cell.has_contents() && !cell.contents().trim().is_empty())
            .unwrap_or(false)
    })
}

fn vt100_scrollback_row_has_content(screen: &vt100::Screen, offset: usize, cols: u16) -> bool {
    (0..cols).any(|c| {
        screen
            .scrollback_cell(offset, c)
            .map(|cell| cell.has_contents() && !cell.contents().trim().is_empty())
            .unwrap_or(false)
    })
}

fn vt100_screen_row(screen: &vt100::Screen, row: u16, use_cols: u16) -> Line<'static> {
    let mut spans = Vec::new();
    for c in 0..use_cols {
        let Some(cell) = screen.cell(row, c) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        spans.push(vt100_span_from_cell(cell));
    }
    Line::from(spans)
}

fn vt100_scrollback_row(screen: &vt100::Screen, offset: usize, use_cols: u16) -> Line<'static> {
    let mut spans = Vec::new();
    for c in 0..use_cols {
        let Some(cell) = screen.scrollback_cell(offset, c) else {
            continue;
        };
        if cell.is_wide_continuation() {
            continue;
        }
        spans.push(vt100_span_from_cell(cell));
    }
    Line::from(spans)
}

fn vt100_span_from_cell(cell: &vt100::Cell) -> Span<'static> {
    let text = if cell.has_contents() {
        cell.contents()
    } else {
        " ".to_string()
    };
    Span::styled(text, vt100_cell_style(cell))
}

fn truncate_line(line: Line<'static>, max_cols: u16) -> Line<'static> {
    let mut width = 0usize;
    let limit = max_cols as usize;
    let mut spans = Vec::new();
    for span in line.spans {
        if width >= limit {
            break;
        }
        let content = span.content.to_string();
        let remaining = limit - width;
        let mut taken = String::new();
        for ch in content.chars() {
            let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
            if width + ch_width > limit || taken.chars().count() >= remaining {
                break;
            }
            taken.push(ch);
            width += ch_width;
        }
        if !taken.is_empty() {
            spans.push(Span::styled(taken, span.style));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alacritty_replays_history_after_resize() {
        let mut buffer = TerminalBufferState::new(TerminalCoreKind::Alacritty);
        buffer.append_bytes(b"hello");
        buffer.rebuild_for_size(20, 5);

        assert!(buffer.plain_row(0).unwrap_or_default().contains("hello"));
    }

    #[test]
    fn switching_core_replays_history() {
        let mut buffer = TerminalBufferState::new(TerminalCoreKind::Alacritty);
        buffer.append_bytes(b"before");
        buffer.ensure_core_kind(TerminalCoreKind::Vt100);

        assert!(buffer.plain_row(0).unwrap_or_default().contains("before"));
    }

    #[test]
    fn alacritty_answers_cursor_position_request() {
        let mut buffer = TerminalBufferState::new(TerminalCoreKind::Alacritty);
        let responses = buffer.append_bytes(b"\x1b[6n");

        assert!(responses
            .iter()
            .any(|response| response.starts_with(b"\x1b[") && response.ends_with(b"R")));
    }

    #[test]
    fn vt100_answers_cursor_position_request() {
        let mut buffer = TerminalBufferState::new(TerminalCoreKind::Vt100);
        let responses = buffer.append_bytes(b"\x1b[6n");

        assert!(responses
            .iter()
            .any(|response| response.starts_with(b"\x1b[") && response.ends_with(b"R")));
    }

    fn first_cell_style(kind: TerminalCoreKind, bytes: &[u8]) -> Style {
        let url_re = Regex::new("$^").unwrap();
        let mut buffer = TerminalBufferState::new(kind);
        buffer.append_bytes(bytes);
        buffer.lines(&url_re)[0].spans[0].style
    }

    #[test]
    fn reverse_video_preserves_default_terminal_colors() {
        let style = toggle_reverse_video(Style::default().fg(TuiColor::Reset).bg(TuiColor::Reset));

        assert_eq!(style.fg, Some(TuiColor::Reset));
        assert_eq!(style.bg, Some(TuiColor::Reset));
        assert!(style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn reverse_video_rendering_preserves_default_terminal_colors() {
        for kind in [TerminalCoreKind::Alacritty, TerminalCoreKind::Vt100] {
            let style = first_cell_style(kind, b"\x1b[7mX");

            assert_eq!(style.fg, Some(TuiColor::Reset), "{kind:?}");
            assert_eq!(style.bg, Some(TuiColor::Reset), "{kind:?}");
            assert!(style.add_modifier.contains(Modifier::REVERSED), "{kind:?}");
        }
    }

    #[test]
    fn ansi_white_background_uses_terminal_palette_index() {
        for kind in [TerminalCoreKind::Alacritty, TerminalCoreKind::Vt100] {
            let style = first_cell_style(kind, b"\x1b[48;5;7mX");

            assert_eq!(style.bg, Some(TuiColor::Indexed(7)), "{kind:?}");
        }
    }

    #[test]
    fn bright_white_background_stays_bright_white() {
        for kind in [TerminalCoreKind::Alacritty, TerminalCoreKind::Vt100] {
            let style = first_cell_style(kind, b"\x1b[48;5;15mX");

            assert_eq!(style.bg, Some(TuiColor::Indexed(15)), "{kind:?}");
        }
    }

    #[test]
    fn color_queries_report_dark_default_background() {
        let mut buffer = TerminalBufferState::new(TerminalCoreKind::Alacritty);
        let responses = buffer.append_bytes(b"\x1b]11;?\x07");

        assert!(responses
            .iter()
            .any(|response| response == b"\x1b]11;rgb:0000/0000/0000\x07"));
    }

    #[test]
    fn color_queries_report_light_default_foreground() {
        let mut buffer = TerminalBufferState::new(TerminalCoreKind::Alacritty);
        let responses = buffer.append_bytes(b"\x1b]10;?\x07");

        assert!(responses
            .iter()
            .any(|response| response == b"\x1b]10;rgb:e5e5/e5e5/e5e5\x07"));
    }
}
