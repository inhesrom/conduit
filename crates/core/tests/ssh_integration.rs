use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use tempfile::TempDir;

use anvl_core::{spawn_core, workspace::git::refresh_git};
use protocol::{Command as CoreCommand, Event as CoreEvent, SshTarget};

static SSH_ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<str>) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value.as_ref());
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn git_init(dir: &Path) {
    Command::new("git")
        .args(["init"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir)
        .output()
        .unwrap();
}

fn write_file(dir: &Path, name: &str, content: &str) {
    if let Some(parent) = dir.join(name).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(dir.join(name), content).unwrap();
}

fn git_add_all(dir: &Path) {
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .output()
        .unwrap();
}

fn git_add_file(dir: &Path, file: &str) {
    Command::new("git")
        .args(["add", "--", file])
        .current_dir(dir)
        .output()
        .unwrap();
}

fn git_commit(dir: &Path, message: &str) {
    Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output()
        .unwrap();
}

fn write_fake_ssh_script(dir: &Path) -> PathBuf {
    let path = dir.join("ssh");
    std::fs::write(
        &path,
        r#"#!/bin/sh
mode="${ANVL_FAKE_SSH_MODE:-ok}"
last=""
for arg in "$@"; do
  last="$arg"
done

case "$mode" in
  fail)
    echo "simulated ssh failure" >&2
    exit 255
    ;;
  truncate)
    printf 'main'
    exit 0
    ;;
  *)
    exec /bin/sh -c "$last"
    ;;
esac
"#,
    )
    .unwrap();

    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();

    path
}

fn fake_target() -> SshTarget {
    SshTarget {
        host: "fake.example".into(),
        user: Some("tester".into()),
        port: Some(2222),
    }
}

#[tokio::test]
async fn refresh_git_over_fake_ssh_clean_repo() {
    let _lock = SSH_ENV_LOCK.lock().unwrap();
    let repo = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let fake_ssh = write_fake_ssh_script(fake_bin_dir.path());

    git_init(repo.path());
    write_file(repo.path(), "hello.txt", "hello");
    git_add_all(repo.path());
    git_commit(repo.path(), "initial commit");

    let _ssh_bin = EnvVarGuard::set("ANVL_SSH_BIN", fake_ssh.display().to_string());
    let _fake_mode = EnvVarGuard::set("ANVL_FAKE_SSH_MODE", "ok");
    let _shell = EnvVarGuard::set("SHELL", "/bin/bash");

    let state = refresh_git(repo.path(), Some(&fake_target()))
        .await
        .unwrap();
    let branch = state.branch.as_deref().unwrap();
    assert!(branch == "main" || branch == "master");
    assert!(state.changed.is_empty());
    assert!(!state.local_branches.is_empty());
}

#[tokio::test]
async fn refresh_git_over_fake_ssh_preserves_changed_paths_and_branch_names() {
    let _lock = SSH_ENV_LOCK.lock().unwrap();
    let repo = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let fake_ssh = write_fake_ssh_script(fake_bin_dir.path());

    git_init(repo.path());
    write_file(
        repo.path(),
        "models/spectrogram/checkpoint_epoch_005.pt",
        "initial",
    );
    git_add_all(repo.path());
    git_commit(repo.path(), "initial commit");
    Command::new("git")
        .args(["branch", "aaa-feature"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    write_file(
        repo.path(),
        "models/spectrogram/checkpoint_epoch_005.pt",
        "modified",
    );

    let _ssh_bin = EnvVarGuard::set("ANVL_SSH_BIN", fake_ssh.display().to_string());
    let _fake_mode = EnvVarGuard::set("ANVL_FAKE_SSH_MODE", "ok");
    let _shell = EnvVarGuard::set("SHELL", "/bin/bash");

    let state = refresh_git(repo.path(), Some(&fake_target()))
        .await
        .unwrap();
    assert_eq!(state.changed.len(), 1);
    assert_eq!(
        state.changed[0].path,
        "models/spectrogram/checkpoint_epoch_005.pt"
    );
    assert!(state
        .local_branches
        .iter()
        .any(|branch| branch.name == "aaa-feature"));
}

#[tokio::test]
async fn refresh_git_over_fake_ssh_matches_local_changed_statuses() {
    let _lock = SSH_ENV_LOCK.lock().unwrap();
    let repo = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let fake_ssh = write_fake_ssh_script(fake_bin_dir.path());

    git_init(repo.path());
    write_file(
        repo.path(),
        "models/spectrogram/checkpoint_epoch_005.pt",
        "base-005",
    );
    write_file(
        repo.path(),
        "models/spectrogram/checkpoint_epoch_010.pt",
        "base-010",
    );
    git_add_all(repo.path());
    git_commit(repo.path(), "initial commit");

    write_file(
        repo.path(),
        "models/spectrogram/checkpoint_epoch_005.pt",
        "staged-change",
    );
    git_add_file(repo.path(), "models/spectrogram/checkpoint_epoch_005.pt");
    write_file(
        repo.path(),
        "models/spectrogram/checkpoint_epoch_010.pt",
        "unstaged-change",
    );

    let _ssh_bin = EnvVarGuard::set("ANVL_SSH_BIN", fake_ssh.display().to_string());
    let _fake_mode = EnvVarGuard::set("ANVL_FAKE_SSH_MODE", "ok");
    let _shell = EnvVarGuard::set("SHELL", "/bin/bash");

    let local = refresh_git(repo.path(), None).await.unwrap();
    let ssh = refresh_git(repo.path(), Some(&fake_target()))
        .await
        .unwrap();

    let local_changed: Vec<_> = local
        .changed
        .iter()
        .map(|f| (f.path.clone(), f.index_status, f.worktree_status))
        .collect();
    let ssh_changed: Vec<_> = ssh
        .changed
        .iter()
        .map(|f| (f.path.clone(), f.index_status, f.worktree_status))
        .collect();

    assert_eq!(ssh_changed, local_changed);
}

#[tokio::test]
async fn refresh_git_over_fake_ssh_reports_transport_failure() {
    let _lock = SSH_ENV_LOCK.lock().unwrap();
    let repo = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let fake_ssh = write_fake_ssh_script(fake_bin_dir.path());

    let _ssh_bin = EnvVarGuard::set("ANVL_SSH_BIN", fake_ssh.display().to_string());
    let _fake_mode = EnvVarGuard::set("ANVL_FAKE_SSH_MODE", "fail");
    let _shell = EnvVarGuard::set("SHELL", "/bin/bash");

    let err = refresh_git(repo.path(), Some(&fake_target()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("simulated ssh failure"));
}

#[tokio::test]
async fn refresh_git_over_fake_ssh_reports_incomplete_output() {
    let _lock = SSH_ENV_LOCK.lock().unwrap();
    let repo = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let fake_ssh = write_fake_ssh_script(fake_bin_dir.path());

    let _ssh_bin = EnvVarGuard::set("ANVL_SSH_BIN", fake_ssh.display().to_string());
    let _fake_mode = EnvVarGuard::set("ANVL_FAKE_SSH_MODE", "truncate");
    let _shell = EnvVarGuard::set("SHELL", "/bin/bash");

    let err = refresh_git(repo.path(), Some(&fake_target()))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("expected 8"));
}

#[tokio::test]
async fn add_workspace_over_ssh_is_not_blocked_by_noninteractive_auth_failure() {
    let _lock = SSH_ENV_LOCK.lock().unwrap();
    let repo = TempDir::new().unwrap();
    let fake_bin_dir = TempDir::new().unwrap();
    let fake_ssh = write_fake_ssh_script(fake_bin_dir.path());
    let fake_home = TempDir::new().unwrap();

    let _ssh_bin = EnvVarGuard::set("ANVL_SSH_BIN", fake_ssh.display().to_string());
    let _fake_mode = EnvVarGuard::set("ANVL_FAKE_SSH_MODE", "fail");
    let _shell = EnvVarGuard::set("SHELL", "/bin/bash");
    let _home = EnvVarGuard::set("HOME", fake_home.path().display().to_string());

    let core = spawn_core();
    let mut evt_rx = core.evt_tx.subscribe();

    core.cmd_tx
        .send(CoreCommand::AddWorkspace {
            name: "remote-repo".into(),
            path: repo.path().display().to_string(),
            ssh: Some(fake_target()),
        })
        .await
        .unwrap();

    let mut saw_workspace = false;
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        let evt = tokio::time::timeout_at(deadline, evt_rx.recv()).await;
        let Ok(Ok(evt)) = evt else { break };
        if let CoreEvent::WorkspaceList { items } = evt {
            if items.iter().any(|item| item.name == "remote-repo") {
                saw_workspace = true;
                break;
            }
        }
    }

    assert!(
        saw_workspace,
        "expected SSH workspace to be added despite auth failure"
    );
}
