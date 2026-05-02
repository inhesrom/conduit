use protocol::SavedCommand;

pub fn resurrect_input(cmd: &SavedCommand) -> String {
    let argv = cmd
        .argv
        .iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    if cmd.cwd.is_empty() {
        format!("{argv}\n")
    } else {
        format!("cd {} && {argv}\n", shell_quote(&cmd.cwd))
    }
}

pub fn preview_line(cmd: &SavedCommand) -> String {
    let argv = cmd
        .argv
        .iter()
        .map(|a| shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    if cmd.cwd.is_empty() {
        argv
    } else {
        format!("cd {} && {argv}", shell_quote(&cmd.cwd))
    }
}

fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"@%+=:,./-_".contains(&b))
    {
        return s.into();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_plain() {
        assert_eq!(shell_quote("python"), "python");
        assert_eq!(shell_quote("/usr/bin/python3"), "/usr/bin/python3");
        assert_eq!(shell_quote("foo-bar_baz"), "foo-bar_baz");
    }

    #[test]
    fn quote_with_space() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
    }

    #[test]
    fn quote_with_single_quote() {
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn quote_empty() {
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn resurrect_input_basic() {
        let cmd = SavedCommand {
            argv: vec!["python".into(), "tests.py".into(), "--flag=hello world".into()],
            cwd: "/home/me/work".into(),
        };
        assert_eq!(
            resurrect_input(&cmd),
            "cd /home/me/work && python tests.py '--flag=hello world'\n"
        );
    }

    #[test]
    fn resurrect_input_no_cwd() {
        let cmd = SavedCommand {
            argv: vec!["vim".into()],
            cwd: "".into(),
        };
        assert_eq!(resurrect_input(&cmd), "vim\n");
    }

    #[test]
    fn preview_includes_cd() {
        let cmd = SavedCommand {
            argv: vec!["python".into(), "tests.py".into()],
            cwd: "/repo/src".into(),
        };
        assert_eq!(preview_line(&cmd), "cd /repo/src && python tests.py");
    }
}
