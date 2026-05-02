use anyhow::{bail, Result};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use protocol::SshTarget;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tokio::sync::mpsc;

use super::ssh;

#[derive(Default)]
pub struct WorkspaceTerminals {
    pub agent: Option<TerminalSession>,
    pub shells: HashMap<String, TerminalSession>,
}

pub struct TerminalSession {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send>>>,
}

pub enum TerminalOutput {
    Bytes(Vec<u8>),
    Exited(Option<i32>),
}

impl TerminalSession {
    pub fn is_alive(&self) -> bool {
        match self.child.lock().try_wait() {
            Ok(None) => true,
            Ok(Some(_)) => false,
            Err(_) => false,
        }
    }

    pub async fn send_input(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock();
        writer.write_all(bytes)?;
        Ok(())
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        self.master.lock().resize(size)?;
        Ok(())
    }

    pub async fn stop(self) -> Result<()> {
        self.child.lock().kill()?;
        Ok(())
    }

    pub fn shell_pid(&self) -> Option<u32> {
        self.child.lock().process_id()
    }

    pub fn foreground_pgid(&self) -> Option<libc::pid_t> {
        self.master.lock().process_group_leader()
    }
}

pub async fn start_terminal(
    cwd: PathBuf,
    cmd: Vec<String>,
    ssh_target: Option<&SshTarget>,
) -> Result<(TerminalSession, mpsc::Receiver<TerminalOutput>)> {
    let effective_cmd = if let Some(target) = ssh_target {
        if cmd.is_empty() || is_default_shell_cmd(&cmd) {
            ssh::ssh_args_for_terminal(target, &cwd)
        } else {
            cmd
        }
    } else {
        cmd
    };

    let Some(program) = effective_cmd.first() else {
        bail!("terminal command cannot be empty");
    };

    let pty_system = native_pty_system();
    let pty_pair = pty_system.openpty(PtySize {
        rows: 24,
        cols: 120,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut builder = CommandBuilder::new(program);
    for arg in effective_cmd.iter().skip(1) {
        builder.arg(arg);
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env_remove("TERM_PROGRAM");
    builder.env_remove("TERM_PROGRAM_VERSION");
    if ssh_target.is_none() {
        builder.cwd(cwd);
    }

    let child = pty_pair.slave.spawn_command(builder)?;
    let mut reader = pty_pair.master.try_clone_reader()?;
    let writer = pty_pair.master.take_writer()?;

    let (tx, rx) = mpsc::channel(512);
    let tx_reader = tx.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx_reader.blocking_send(TerminalOutput::Exited(None));
                    break;
                }
                Ok(n) => {
                    if tx_reader
                        .blocking_send(TerminalOutput::Bytes(buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(_) => {
                    let _ = tx_reader.blocking_send(TerminalOutput::Exited(None));
                    break;
                }
            }
        }
    });

    Ok((
        TerminalSession {
            writer: Arc::new(Mutex::new(writer)),
            master: Arc::new(Mutex::new(pty_pair.master)),
            child: Arc::new(Mutex::new(child)),
        },
        rx,
    ))
}

/// Returns true if the command looks like a default local shell invocation.
fn is_default_shell_cmd(cmd: &[String]) -> bool {
    if cmd.is_empty() {
        return true;
    }
    let prog = &cmd[0];
    let is_shell = prog.ends_with("/bash")
        || prog.ends_with("/zsh")
        || prog.ends_with("/sh")
        || prog.ends_with("/fish")
        || prog == "bash"
        || prog == "zsh"
        || prog == "sh"
        || prog == "fish";
    is_shell && cmd.iter().skip(1).all(|a| a.starts_with('-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_is_default_shell() {
        assert!(is_default_shell_cmd(&[]));
    }

    #[test]
    fn bash_is_default_shell() {
        assert!(is_default_shell_cmd(&s(&["bash"])));
    }

    #[test]
    fn bash_with_flag_is_default() {
        assert!(is_default_shell_cmd(&s(&["bash", "-l"])));
    }

    #[test]
    fn zsh_is_default_shell() {
        assert!(is_default_shell_cmd(&s(&["zsh"])));
    }

    #[test]
    fn full_path_shell_is_default() {
        assert!(is_default_shell_cmd(&s(&["/bin/bash"])));
        assert!(is_default_shell_cmd(&s(&["/usr/bin/zsh"])));
        assert!(is_default_shell_cmd(&s(&["/bin/sh", "-l"])));
    }

    #[test]
    fn fish_is_default_shell() {
        assert!(is_default_shell_cmd(&s(&["fish"])));
    }

    #[test]
    fn vim_is_not_default_shell() {
        assert!(!is_default_shell_cmd(&s(&["vim"])));
    }

    #[test]
    fn bash_with_script_is_not_default() {
        assert!(!is_default_shell_cmd(&s(&["bash", "script.sh"])));
    }

    #[test]
    fn bash_with_command_flag_is_not_default() {
        assert!(!is_default_shell_cmd(&s(&["bash", "-c", "echo hello"])));
    }
}
