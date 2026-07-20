//! Adopting an existing folder as a Workspace: a checkout that already has work
//! in flight, attached instead of having a fresh worktree created for it.
//!
//! These drive the real core task through `spawn_core` because the adoption
//! logic lives in the command loop, not in a standalone function.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use conduit_core::spawn_core;
use protocol::{Command as CoreCommand, Event as CoreEvent, RepositoryId, WorkspaceSummary};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

/// Serialises the env-var mutation these tests need — `XDG_CONFIG_HOME` is
/// process-wide, and cargo runs tests in one process by default. Held across
/// awaits on purpose: the env var must stay ours for the whole test body, so
/// `clippy::await_holding_lock` is suppressed at each use.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        match &self.previous {
            Some(previous) => std::env::set_var(self.key, previous),
            None => std::env::remove_var(self.key),
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repo with one commit on `main` and an uncommitted edit — the "already in
/// flight" state adoption exists to pick up.
fn dirty_repo(dir: &Path) {
    git(dir, &["init", "-b", "main"]);
    git(dir, &["config", "user.email", "t@t.com"]);
    git(dir, &["config", "user.name", "T"]);
    std::fs::write(dir.join("readme.md"), "hi").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-m", "init"]);
    std::fs::write(dir.join("readme.md"), "edited by someone else").unwrap();
}

/// Waits for the next `WorkspaceList` whose items satisfy `pred`, returning them.
async fn await_workspaces(
    rx: &mut Receiver<CoreEvent>,
    pred: impl Fn(&[WorkspaceSummary]) -> bool,
) -> Vec<WorkspaceSummary> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out waiting for WorkspaceList")
            .expect("event stream closed");
        if let CoreEvent::WorkspaceList { items } = evt {
            if pred(&items) {
                return items;
            }
        }
    }
}

async fn await_repository(rx: &mut Receiver<CoreEvent>, name: &str) -> RepositoryId {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let evt = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .expect("timed out waiting for RepositoryList")
            .expect("event stream closed");
        if let CoreEvent::RepositoryList { items } = evt {
            if let Some(repo) = items.iter().find(|r| r.name == name) {
                return repo.id;
            }
        }
    }
}

/// Adopting the registered repo's *own* directory — the case where you started
/// editing on `main` outside Conduit and want to keep going inside it.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn adopts_base_repo_directory_and_leaves_it_on_disk() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let config = TempDir::new().unwrap();
    let _cfg = EnvVarGuard::set("XDG_CONFIG_HOME", config.path().display().to_string());

    let repo = TempDir::new().unwrap();
    dirty_repo(repo.path());

    let core = spawn_core();
    let mut evt_rx = core.evt_tx.subscribe();

    core.cmd_tx
        .send(CoreCommand::RegisterRepository {
            name: "demo".into(),
            path: repo.path().display().to_string(),
            ssh: None,
            default_agent: None,
            worktree_root: None,
        })
        .await
        .unwrap();
    let repo_id = await_repository(&mut evt_rx, "demo").await;

    // Registering must NOT auto-create a workspace for the base checkout; only
    // an explicit adopt does that.
    core.cmd_tx
        .send(CoreCommand::AddWorkspace {
            name: String::new(),
            path: repo.path().display().to_string(),
            ssh: None,
            repository_id: Some(repo_id),
            base_branch: None,
            agent: Some("claude".into()),
            adopted: true,
        })
        .await
        .unwrap();

    let items = await_workspaces(&mut evt_rx, |items| !items.is_empty()).await;
    assert_eq!(items.len(), 1, "exactly one workspace: {items:?}");
    let ws = &items[0];
    assert!(ws.adopted, "explicit adoption must set the adopted flag");
    assert_eq!(ws.repository_id, Some(repo_id));
    assert_eq!(ws.branch.as_deref(), Some("main"));
    // Empty name falls back to the branch already checked out.
    assert_eq!(ws.name, "main");
    assert_eq!(ws.agent.as_deref(), Some("claude"));
    assert_eq!(ws.dirty_files, 1, "the in-flight edit should be visible");

    // Removal unregisters only — the folder and its uncommitted work survive.
    core.cmd_tx
        .send(CoreCommand::RemoveWorkspace { id: ws.id })
        .await
        .unwrap();
    await_workspaces(&mut evt_rx, |items| items.is_empty()).await;

    assert!(repo.path().join("readme.md").exists(), "folder was deleted");
    assert_eq!(
        std::fs::read_to_string(repo.path().join("readme.md")).unwrap(),
        "edited by someone else",
        "uncommitted work was lost"
    );
}

/// A sibling clone with an unrelated origin is still adopted (warn, don't
/// block), and the same folder can't be attached twice.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn adopts_sibling_checkout_once_and_rejects_non_repos() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let config = TempDir::new().unwrap();
    let _cfg = EnvVarGuard::set("XDG_CONFIG_HOME", config.path().display().to_string());

    let repo = TempDir::new().unwrap();
    dirty_repo(repo.path());
    let sibling = TempDir::new().unwrap();
    dirty_repo(sibling.path());
    git(sibling.path(), &["checkout", "-b", "feature-x"]);

    let core = spawn_core();
    let mut evt_rx = core.evt_tx.subscribe();

    core.cmd_tx
        .send(CoreCommand::RegisterRepository {
            name: "demo".into(),
            path: repo.path().display().to_string(),
            ssh: None,
            default_agent: None,
            worktree_root: None,
        })
        .await
        .unwrap();
    let repo_id = await_repository(&mut evt_rx, "demo").await;

    let adopt = |name: &str| CoreCommand::AddWorkspace {
        name: name.to_string(),
        path: sibling.path().display().to_string(),
        ssh: None,
        repository_id: Some(repo_id),
        base_branch: Some("main".into()),
        agent: None,
        adopted: true,
    };

    core.cmd_tx.send(adopt("in-flight")).await.unwrap();
    let items = await_workspaces(&mut evt_rx, |items| !items.is_empty()).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "in-flight", "explicit name wins");
    assert_eq!(items[0].branch.as_deref(), Some("feature-x"));
    assert_eq!(items[0].base_branch.as_deref(), Some("main"));

    // Re-adopting the same folder is refused: two agent terminals in one
    // working tree would fight over the same files.
    core.cmd_tx.send(adopt("duplicate")).await.unwrap();
    // A folder that isn't a git checkout is refused too.
    let plain = TempDir::new().unwrap();
    core.cmd_tx
        .send(CoreCommand::AddWorkspace {
            name: "not-a-repo".into(),
            path: plain.path().display().to_string(),
            ssh: None,
            repository_id: Some(repo_id),
            base_branch: None,
            agent: None,
            adopted: true,
        })
        .await
        .unwrap();

    let mut errors = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while errors.len() < 2 {
        let evt = tokio::time::timeout_at(deadline, evt_rx.recv())
            .await
            .expect("timed out waiting for rejections")
            .expect("event stream closed");
        if let CoreEvent::Error { message } = evt {
            errors.push(message);
        }
    }
    assert!(
        errors
            .iter()
            .any(|m| m.contains("already open as a workspace")),
        "expected a duplicate rejection: {errors:?}"
    );
    assert!(
        errors.iter().any(|m| m.contains("is not a git repository")),
        "expected a non-repo rejection: {errors:?}"
    );
}
