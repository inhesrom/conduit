use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    if key.modifiers.contains(KeyModifiers::SHIFT) && key.code != KeyCode::BackTab {
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
