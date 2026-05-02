use std::path::PathBuf;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone)]
pub struct ForegroundInfo {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
}

pub fn lookup(pid: i32) -> Option<ForegroundInfo> {
    if pid <= 0 {
        return None;
    }
    let pid = Pid::from_u32(pid as u32);
    let mut system = System::new();
    let refresh = ProcessRefreshKind::nothing()
        .with_cmd(UpdateKind::Always)
        .with_cwd(UpdateKind::Always);
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), false, refresh);
    let proc = system.process(pid)?;
    let argv: Vec<String> = proc
        .cmd()
        .iter()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    if argv.is_empty() {
        return None;
    }
    let cwd = proc.cwd().map(|p| p.to_path_buf()).unwrap_or_default();
    Some(ForegroundInfo { argv, cwd })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn lookup_self_returns_argv_and_cwd() {
        let pid = std::process::id() as i32;
        let info = lookup(pid).expect("self lookup");
        assert!(!info.argv.is_empty());
        let expected_cwd = std::env::current_dir().expect("cwd");
        assert_eq!(info.cwd, expected_cwd);
    }

    #[test]
    fn lookup_invalid_pid_returns_none() {
        assert!(lookup(0).is_none());
        assert!(lookup(-1).is_none());
    }
}
