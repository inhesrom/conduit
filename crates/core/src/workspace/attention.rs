use protocol::AttentionLevel;

const MAX_BUFFER: usize = 2048;
const TAIL_WINDOW: usize = 512;

const PATTERNS: &[&str] = &[
    "this command requires approval",
    "yes, and don't ask again",
    "yes, and dont ask again",
    "esc to cancel",
    "tab to amend",
    "[y/n]",
    "(y/n)",
    "waiting for input",
    "waiting for your input",
    "requires your input",
    "do you want to proceed?",
    "allow once",
    "allow always",
    "press enter to continue",
    "press return to continue",
    "approve command",
];

pub struct AttentionDetector {
    buffer: String,
}

impl AttentionDetector {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    /// Normalize and append PTY output to the rolling buffer.
    /// Returns `true` if any non-empty content was appended.
    pub fn append(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return false;
        }
        let chunk = String::from_utf8_lossy(bytes);
        let cleaned = normalize_for_match(&chunk);
        if cleaned.is_empty() {
            return false;
        }
        if !self.buffer.is_empty() {
            self.buffer.push(' ');
        }
        self.buffer.push_str(&cleaned);
        if self.buffer.len() > MAX_BUFFER {
            trim_to_last_bytes_at_char_boundary(&mut self.buffer, MAX_BUFFER);
        }
        true
    }

    /// Check the last [`TAIL_WINDOW`] bytes of the buffer for prompt patterns.
    /// If a pattern matches, the buffer is cleared and `true` is returned.
    pub fn check_for_prompt(&mut self) -> bool {
        let tail = tail_str(&self.buffer, TAIL_WINDOW);
        if PATTERNS.iter().any(|p| tail.contains(p)) {
            self.buffer.clear();
            return true;
        }
        // Fallback: agent asked a question and output has settled
        if tail.ends_with('?') {
            self.buffer.clear();
            return true;
        }
        false
    }

    /// Clear internal state (called when attention is externally cleared).
    pub fn reset(&mut self) {
        self.buffer.clear();
    }
}

pub fn needs_flash(level: AttentionLevel) -> bool {
    matches!(level, AttentionLevel::NeedsInput | AttentionLevel::Error)
}

fn tail_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len() - max_bytes;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn normalize_for_match(input: &str) -> String {
    let no_ansi = strip_ansi(input);
    no_ansi
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek().copied() {
                Some('[') => {
                    // CSI: ESC [ ... (terminator in @..~)
                    let _ = chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('X') | Some('^') | Some('_') => {
                    // OSC / DCS / SOS / PM / APC: consume until BEL or ST (ESC \)
                    let _ = chars.next();
                    for c in chars.by_ref() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                let _ = chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    let _ = chars.next();
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn trim_to_last_bytes_at_char_boundary(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut start = s.len().saturating_sub(max_bytes);
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    if start >= s.len() {
        s.clear();
        return;
    }
    s.drain(..start);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_normalizes() {
        let mut det = AttentionDetector::new();
        assert!(det.append(b"Hello  World\n"));
        assert_eq!(det.buffer, "hello world");
    }

    #[test]
    fn append_empty_returns_false() {
        let mut det = AttentionDetector::new();
        assert!(!det.append(b""));
    }

    #[test]
    fn append_strips_ansi() {
        let mut det = AttentionDetector::new();
        det.append(b"\x1b[32mgreen\x1b[0m text");
        assert_eq!(det.buffer, "green text");
    }

    #[test]
    fn check_for_prompt_matches_yn() {
        let mut det = AttentionDetector::new();
        det.append(b"Proceed? [y/n]");
        assert!(det.check_for_prompt());
        // Buffer should be cleared after match
        assert!(det.buffer.is_empty());
    }

    #[test]
    fn check_for_prompt_matches_approval() {
        let mut det = AttentionDetector::new();
        det.append(b"This command requires approval");
        assert!(det.check_for_prompt());
    }

    #[test]
    fn check_for_prompt_no_false_positive_confirm() {
        let mut det = AttentionDetector::new();
        det.append(b"I can confirm that the code is working");
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn check_for_prompt_no_false_positive_continue() {
        let mut det = AttentionDetector::new();
        det.append(b"Let me continue with the implementation");
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn check_for_prompt_matches_allow_once() {
        let mut det = AttentionDetector::new();
        det.append(b"Allow once  Allow always");
        assert!(det.check_for_prompt());
    }

    #[test]
    fn check_for_prompt_matches_do_you_want_to_proceed_with_question_mark() {
        let mut det = AttentionDetector::new();
        det.append(b"Do you want to proceed?");
        assert!(det.check_for_prompt());
    }

    #[test]
    fn check_for_prompt_no_match_do_you_want_to_proceed_without_question_mark() {
        let mut det = AttentionDetector::new();
        det.append(b"do you want to proceed with the task");
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn check_for_prompt_approve_command() {
        let mut det = AttentionDetector::new();
        det.append(b"approve command");
        assert!(det.check_for_prompt());
    }

    #[test]
    fn only_checks_tail_window() {
        let mut det = AttentionDetector::new();
        // Push a pattern, then push enough filler to push it out of the tail window
        det.append(b"[y/n]");
        let filler = "x".repeat(TAIL_WINDOW + 100);
        det.append(filler.as_bytes());
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn reset_clears_buffer() {
        let mut det = AttentionDetector::new();
        det.append(b"some text");
        det.reset();
        assert!(det.buffer.is_empty());
    }

    #[test]
    fn buffer_truncates_at_max() {
        let mut det = AttentionDetector::new();
        let big = "a".repeat(MAX_BUFFER + 500);
        det.append(big.as_bytes());
        assert!(det.buffer.len() <= MAX_BUFFER);
    }

    #[test]
    fn needs_flash_correct() {
        assert!(needs_flash(AttentionLevel::NeedsInput));
        assert!(needs_flash(AttentionLevel::Error));
        assert!(!needs_flash(AttentionLevel::None));
    }

    #[test]
    fn press_enter_to_continue_matches() {
        let mut det = AttentionDetector::new();
        det.append(b"Press Enter to continue");
        assert!(det.check_for_prompt());
    }

    #[test]
    fn generic_press_enter_no_match() {
        let mut det = AttentionDetector::new();
        det.append(b"press enter");
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn trailing_question_mark_matches() {
        let mut det = AttentionDetector::new();
        det.append(b"Should I commit with this message?");
        assert!(det.check_for_prompt());
        assert!(det.buffer.is_empty());
    }

    #[test]
    fn no_question_mark_no_match() {
        let mut det = AttentionDetector::new();
        det.append(b"The answer is 42.");
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn question_mark_not_at_end_no_match() {
        let mut det = AttentionDetector::new();
        det.append(b"What went wrong? Let me investigate");
        assert!(!det.check_for_prompt());
    }

    #[test]
    fn question_mark_with_osc_suffix_matches() {
        let mut det = AttentionDetector::new();
        det.append(b"Should I commit?\x1b]133;D\x07");
        assert!(det.check_for_prompt());
    }

    #[test]
    fn strip_ansi_handles_osc_with_st() {
        let result = strip_ansi("hello\x1b]0;title\x1b\\world");
        assert_eq!(result, "helloworld");
    }

    #[test]
    fn strip_ansi_handles_dcs() {
        let result = strip_ansi("before\x1bPq#0;2;0;0;0\x1b\\after");
        assert_eq!(result, "beforeafter");
    }
}
