use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;

use anyhow::{bail, Result};
use protocol::SshTarget;
use tokio::process::Command;

/// Builds a `Command` that either runs locally or tunnels through SSH.
///
/// For local execution (`ssh == None`), returns `Command::new(program)` with `current_dir(cwd)`.
/// For SSH execution, returns `ssh` with ControlMaster args that runs the command
/// through the remote login shell when available, with sane shell fallbacks.
pub fn build_command(ssh: Option<&SshTarget>, cwd: &Path, program: &str, args: &[&str]) -> Command {
    match ssh {
        None => {
            let mut cmd = Command::new(program);
            cmd.args(args).current_dir(cwd);
            cmd
        }
        Some(target) => {
            let mut cmd = Command::new(ssh_program());
            append_ssh_args(&mut cmd, target);
            cmd.arg("-o").arg("BatchMode=yes");
            cmd.arg(ssh_destination(target));
            cmd.arg(remote_command(cwd, program, args));
            cmd
        }
    }
}

/// Validates SSH connectivity and that the remote path exists.
pub async fn validate_ssh_connection(target: &SshTarget, path: &Path) -> Result<()> {
    let mut cmd = Command::new(ssh_program());
    append_ssh_args(&mut cmd, target);
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg("-o").arg("ConnectTimeout=5");
    cmd.arg(ssh_destination(target));
    cmd.arg(wrap_login_shell_script(&format!(
        "test -d {}",
        shell_quote(&path.display().to_string())
    )));

    let out = cmd.output().await?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        bail!(
            "SSH connection to {} failed or path {} does not exist: {}",
            ssh_destination(target),
            path.display(),
            stderr.trim()
        );
    }
    Ok(())
}

/// Single-quote wraps a string for safe use in remote shell commands.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Returns the `user@host` or just `host` SSH destination string.
pub fn ssh_destination(target: &SshTarget) -> String {
    match &target.user {
        Some(user) => format!("{}@{}", user, target.host),
        None => target.host.clone(),
    }
}

/// Returns a deterministic control socket path for ControlMaster multiplexing.
fn control_socket_path(target: &SshTarget) -> String {
    let mut hasher = DefaultHasher::new();
    target.host.hash(&mut hasher);
    target.user.hash(&mut hasher);
    target.port.hash(&mut hasher);
    let hash = hasher.finish();
    format!("/tmp/conduit-ssh-{:x}", hash)
}

/// Appends common SSH arguments (ControlMaster, port) to a command.
fn append_ssh_args(cmd: &mut Command, target: &SshTarget) {
    if let Some(port) = target.port {
        cmd.arg("-p").arg(port.to_string());
    }
    let socket = control_socket_path(target);
    cmd.arg("-o").arg("ControlMaster=auto");
    cmd.arg("-o").arg(format!("ControlPath={}", socket));
    cmd.arg("-o").arg("ControlPersist=600");
}

/// Delimiter used to separate output sections in a batched SSH command.
pub const BATCH_DELIM: &str = "---CONDUIT_BATCH_DELIM---";

/// Builds a single SSH `Command` that runs multiple shell commands on the remote,
/// separated by `BATCH_DELIM` markers so the caller can split the combined stdout.
pub fn build_batch_command(target: &SshTarget, cwd: &Path, commands: &[String]) -> Command {
    let joined = commands
        .iter()
        .map(|c| format!("{{ {}; }}", c))
        .collect::<Vec<_>>()
        .join(&format!(" ; echo '{}' ; ", BATCH_DELIM));

    let script = format!(
        "cd {} && {{ {}; }}",
        shell_quote(&cwd.display().to_string()),
        joined
    );

    let mut cmd = Command::new(ssh_program());
    append_ssh_args(&mut cmd, target);
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg(ssh_destination(target));
    cmd.arg(wrap_login_shell_script(&script));
    cmd
}

/// Builds SSH args as a Vec<String> for use with CommandBuilder (terminals).
pub fn ssh_args_for_terminal(target: &SshTarget, cwd: &Path) -> Vec<String> {
    let mut args = vec![ssh_program(), "-t".to_string()];
    if let Some(port) = target.port {
        args.push("-p".to_string());
        args.push(port.to_string());
    }
    let socket = control_socket_path(target);
    args.push("-o".to_string());
    args.push("ControlMaster=auto".to_string());
    args.push("-o".to_string());
    args.push(format!("ControlPath={}", socket));
    args.push("-o".to_string());
    args.push("ControlPersist=600".to_string());
    args.push(ssh_destination(target));
    args.push(remote_terminal_shell_command(cwd));
    args
}

fn ssh_program() -> String {
    std::env::var("CONDUIT_SSH_BIN").unwrap_or_else(|_| "ssh".to_string())
}

fn remote_command(cwd: &Path, program: &str, args: &[&str]) -> String {
    let script = format!(
        "cd {} && {}",
        shell_quote(&cwd.display().to_string()),
        shell_words(program, args)
    );
    wrap_login_shell_script(&script)
}

fn shell_words(program: &str, args: &[&str]) -> String {
    std::iter::once(shell_quote(program))
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn wrap_login_shell_script(script: &str) -> String {
    let quoted = shell_quote(script);
    format!(
        "if [ -n \"${{SHELL:-}}\" ]; then exec \"$SHELL\" -lc {quoted}; \
elif command -v bash >/dev/null 2>&1; then exec bash -lc {quoted}; \
elif command -v zsh >/dev/null 2>&1; then exec zsh -lc {quoted}; \
elif command -v fish >/dev/null 2>&1; then exec fish -lc {quoted}; \
else exec sh -c {quoted}; fi"
    )
}

fn remote_terminal_shell_command(cwd: &Path) -> String {
    format!(
        "cd {} && export TERM=xterm-256color COLORTERM=truecolor && \
if [ -n \"${{SHELL:-}}\" ]; then exec \"$SHELL\" -l; \
elif command -v bash >/dev/null 2>&1; then exec bash -l; \
elif command -v zsh >/dev/null 2>&1; then exec zsh -l; \
elif command -v fish >/dev/null 2>&1; then exec fish -l; \
else exec sh; fi",
        shell_quote(&cwd.display().to_string())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::SshTarget;
    use std::path::Path;

    #[test]
    fn shell_quote_basic() {
        assert_eq!(shell_quote("hello"), "'hello'");
    }

    #[test]
    fn shell_quote_with_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn shell_quote_empty() {
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn shell_quote_spaces_and_special() {
        assert_eq!(shell_quote("a b$c"), "'a b$c'");
    }

    #[test]
    fn ssh_destination_with_user() {
        let target = SshTarget {
            host: "example.com".into(),
            user: Some("admin".into()),
            port: None,
        };
        assert_eq!(ssh_destination(&target), "admin@example.com");
    }

    #[test]
    fn ssh_destination_without_user() {
        let target = SshTarget {
            host: "example.com".into(),
            user: None,
            port: None,
        };
        assert_eq!(ssh_destination(&target), "example.com");
    }

    #[test]
    fn control_socket_deterministic() {
        let t1 = SshTarget {
            host: "h".into(),
            user: Some("u".into()),
            port: Some(22),
        };
        let t2 = SshTarget {
            host: "h".into(),
            user: Some("u".into()),
            port: Some(22),
        };
        assert_eq!(control_socket_path(&t1), control_socket_path(&t2));
    }

    #[test]
    fn control_socket_different_targets() {
        let t1 = SshTarget {
            host: "host1".into(),
            user: None,
            port: None,
        };
        let t2 = SshTarget {
            host: "host2".into(),
            user: None,
            port: None,
        };
        assert_ne!(control_socket_path(&t1), control_socket_path(&t2));
    }

    #[test]
    fn control_socket_starts_with_expected_prefix() {
        let t = SshTarget {
            host: "h".into(),
            user: None,
            port: None,
        };
        assert!(control_socket_path(&t).starts_with("/tmp/conduit-ssh-"));
    }

    #[test]
    fn ssh_args_for_terminal_basic() {
        let target = SshTarget {
            host: "example.com".into(),
            user: Some("admin".into()),
            port: None,
        };
        let args = ssh_args_for_terminal(&target, Path::new("/home/user/project"));
        assert_eq!(args[0], "ssh");
        assert_eq!(args[1], "-t");
        assert!(args.contains(&"admin@example.com".to_string()));
        // Should contain the shell fallback wrapper at the end.
        let last = args.last().unwrap();
        assert!(last.starts_with("cd "));
        assert!(last.contains("export TERM=xterm-256color COLORTERM=truecolor"));
        assert!(last.contains("exec \"$SHELL\" -l"));
        assert!(last.contains("exec bash -l"));
        assert!(last.contains("/home/user/project"));
    }

    #[test]
    fn ssh_args_for_terminal_with_port() {
        let target = SshTarget {
            host: "h".into(),
            user: None,
            port: Some(2222),
        };
        let args = ssh_args_for_terminal(&target, Path::new("/tmp"));
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
    }

    #[test]
    fn shell_words_quotes_program_and_args() {
        assert_eq!(
            shell_words("git", &["status", "--porcelain=v1"]),
            "'git' 'status' '--porcelain=v1'"
        );
    }

    #[test]
    fn login_shell_wrapper_uses_fallbacks() {
        let wrapped = wrap_login_shell_script("cd '/repo' && 'git' 'status'");
        assert!(wrapped.contains("exec \"$SHELL\" -lc"));
        assert!(wrapped.contains("exec bash -lc"));
        assert!(wrapped.contains("exec zsh -lc"));
        assert!(wrapped.contains("exec fish -lc"));
        assert!(wrapped.contains("exec sh -c"));
    }

    #[test]
    fn remote_command_wraps_script_in_login_shell() {
        let cmd = remote_command(Path::new("/tmp/repo"), "git", &["status"]);
        assert!(cmd.contains("exec \"$SHELL\" -lc"));
        assert!(cmd.contains("cd '\\''/tmp/repo'\\'' && '\\''git'\\'' '\\''status'\\'''"));
    }

    #[test]
    fn remote_terminal_command_has_fallback_shells() {
        let cmd = remote_terminal_shell_command(Path::new("/tmp/repo"));
        assert!(cmd.starts_with("cd "));
        assert!(cmd.contains("export TERM=xterm-256color COLORTERM=truecolor"));
        assert!(cmd.contains("exec \"$SHELL\" -l"));
        assert!(cmd.contains("exec bash -l"));
        assert!(cmd.contains("exec zsh -l"));
        assert!(cmd.contains("exec fish -l"));
        assert!(cmd.contains("else exec sh; fi"));
    }
}
