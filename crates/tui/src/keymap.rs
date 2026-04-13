use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn is_quit(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('q'))
}

/// Parse a keybinding string like "ctrl+shift+h" into (KeyCode, KeyModifiers).
/// Supported modifiers: ctrl, shift, alt. The final token is the key character.
pub fn parse_keybinding(s: &str) -> Option<(KeyCode, KeyModifiers)> {
    let lowered = s.trim().to_lowercase();
    let parts: Vec<&str> = lowered.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = KeyModifiers::empty();
    for &part in &parts[..parts.len() - 1] {
        match part {
            "ctrl" => modifiers |= KeyModifiers::CONTROL,
            "shift" => modifiers |= KeyModifiers::SHIFT,
            "alt" => modifiers |= KeyModifiers::ALT,
            _ => return None,
        }
    }

    let key_str = parts.last()?;
    let code = if key_str.len() == 1 {
        KeyCode::Char(key_str.chars().next()?)
    } else {
        match *key_str {
            "end" => KeyCode::End,
            "home" => KeyCode::Home,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdn" => KeyCode::PageDown,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            s if s.starts_with('f') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
                let n: u8 = s[1..].parse().ok()?;
                if (1..=12).contains(&n) {
                    KeyCode::F(n)
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    };

    Some((code, modifiers))
}

/// Check whether a KeyEvent matches a keybinding string.
pub fn matches_keybinding(key: KeyEvent, binding: &str) -> bool {
    let Some((code, modifiers)) = parse_keybinding(binding) else {
        return false;
    };
    key.code == code && key.modifiers.contains(modifiers)
}

/// Convert a live `KeyEvent` into a canonical keybinding string like
/// "ctrl+shift+b" or "ctrl+end". Returns None for events that don't make
/// sense as a binding (e.g. pure modifier presses).
pub fn keybind_from_event(key: KeyEvent) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt");
    }
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        parts.push("shift");
    }

    let base: String = match key.code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_ascii_lowercase().to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab => "tab".to_string(),
        KeyCode::BackTab => "backtab".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Insert => "insert".to_string(),
        KeyCode::F(n) if (1..=12).contains(&n) => format!("f{n}"),
        // Don't capture Esc — it's used to cancel the capture itself.
        _ => return None,
    };

    let mut out = String::new();
    for p in parts {
        out.push_str(p);
        out.push('+');
    }
    out.push_str(&base);
    Some(out)
}
