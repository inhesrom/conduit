//! Framework-neutral terminal grid model shared by the GPUI and Iced fronts.
//!
//! Wraps Conduit's vendored `vt100` parser, feeds it the raw PTY bytes that
//! arrive via `Event::TerminalOutput`, and produces a renderable snapshot of
//! cells (text + resolved style) plus a key→bytes encoder for input. Each front
//! only has to translate `CellSnap` colors into its own primitives and map its
//! native key events onto `Key`.

/// A terminal color, mirrored from `vt100::Color` so fronts don't depend on vt100 directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Default,
    Idx(u8),
    Rgb(u8, u8, u8),
}

impl From<vt100::Color> for Color {
    fn from(c: vt100::Color) -> Self {
        match c {
            vt100::Color::Default => Color::Default,
            vt100::Color::Idx(i) => Color::Idx(i),
            vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
        }
    }
}

/// One rendered cell with style already resolved (cursor/inverse swap applied).
#[derive(Clone, Debug)]
pub struct CellSnap {
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}

/// A vt100-backed terminal fed by external PTY bytes (core owns the real PTY).
pub struct Term {
    parser: vt100::Parser,
    rows: u16,
    cols: u16,
}

impl Term {
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, 5000),
            rows,
            cols,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if rows != self.rows || cols != self.cols {
            self.parser.set_size(rows, cols);
            self.rows = rows;
            self.cols = cols;
        }
    }

    pub fn size(&self) -> (u16, u16) {
        (self.rows, self.cols)
    }

    /// Snapshot every visible row, resolving inverse + cursor into swapped colors.
    pub fn snapshot(&self) -> Vec<Vec<CellSnap>> {
        let screen = self.parser.screen();
        let (crow, ccol) = screen.cursor_position();
        let mut rows = Vec::with_capacity(self.rows as usize);
        for r in 0..self.rows {
            let mut row = Vec::with_capacity(self.cols as usize);
            for c in 0..self.cols {
                let cursor_here = r == crow && c == ccol;
                match screen.cell(r, c) {
                    Some(cell) => {
                        // The continuation half of a wide char carries no glyph; skip it
                        // so a coalesced run stays visually aligned for the common ASCII case.
                        if cell.is_wide_continuation() {
                            continue;
                        }
                        let mut fg: Color = cell.fgcolor().into();
                        let mut bg: Color = cell.bgcolor().into();
                        if cell.inverse() ^ cursor_here {
                            std::mem::swap(&mut fg, &mut bg);
                        }
                        let text = if cell.has_contents() {
                            cell.contents()
                        } else {
                            " ".to_string()
                        };
                        row.push(CellSnap {
                            text,
                            fg,
                            bg,
                            bold: cell.bold(),
                            italic: cell.italic(),
                            underline: cell.underline(),
                        });
                    }
                    None => {
                        // Empty cell. Cursor on an empty cell renders as a swapped block.
                        let (fg, bg) = if cursor_here {
                            (Color::Default, Color::Idx(7))
                        } else {
                            (Color::Default, Color::Default)
                        };
                        row.push(CellSnap {
                            text: " ".to_string(),
                            fg,
                            bg,
                            bold: false,
                            italic: false,
                            underline: false,
                        });
                    }
                }
            }
            rows.push(row);
        }
        rows
    }
}

/// The standard xterm 256-color palette as RGB. Fronts resolve `Color::Idx`
/// through this; `Color::Default` is resolved by each front to its theme colors.
pub fn idx_to_rgb(i: u8) -> (u8, u8, u8) {
    const ANSI16: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match i {
        0..=15 => ANSI16[i as usize],
        16..=231 => {
            let i = i - 16;
            let levels = [0u8, 95, 135, 175, 215, 255];
            let r = levels[(i / 36) as usize];
            let g = levels[((i / 6) % 6) as usize];
            let b = levels[(i % 6) as usize];
            (r, g, b)
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
    }
}

/// A framework-neutral key, mapped from each front's native key event.
#[derive(Clone, Copy, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
}

/// Encode a key press into the bytes to send to the PTY (a subset of xterm
/// sequences — enough to drive an interactive shell in the spike).
pub fn key_to_bytes(key: Key, ctrl: bool, alt: bool) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    if alt {
        out.push(0x1b); // Meta/Alt prefix
    }
    match key {
        Key::Char(c) => {
            if ctrl {
                out.push(ctrl_byte(c)?);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
        Key::Enter => out.push(b'\r'),
        Key::Backspace => out.push(0x7f),
        Key::Tab => out.push(b'\t'),
        Key::Escape => out.push(0x1b),
        Key::Up => out.extend_from_slice(b"\x1b[A"),
        Key::Down => out.extend_from_slice(b"\x1b[B"),
        Key::Right => out.extend_from_slice(b"\x1b[C"),
        Key::Left => out.extend_from_slice(b"\x1b[D"),
        Key::Home => out.extend_from_slice(b"\x1b[H"),
        Key::End => out.extend_from_slice(b"\x1b[F"),
        Key::PageUp => out.extend_from_slice(b"\x1b[5~"),
        Key::PageDown => out.extend_from_slice(b"\x1b[6~"),
        Key::Delete => out.extend_from_slice(b"\x1b[3~"),
    }
    Some(out)
}

fn ctrl_byte(c: char) -> Option<u8> {
    let lower = c.to_ascii_lowercase();
    if lower.is_ascii_alphabetic() {
        Some(lower as u8 - b'a' + 1) // Ctrl-A => 0x01
    } else {
        match c {
            ' ' | '2' | '@' => Some(0),
            '[' => Some(0x1b),
            '\\' => Some(0x1c),
            ']' => Some(0x1d),
            '^' => Some(0x1e),
            '_' => Some(0x1f),
            _ => None,
        }
    }
}
