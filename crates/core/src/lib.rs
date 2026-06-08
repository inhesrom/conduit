pub mod commands;
pub mod events;
mod foreground_commands;
pub mod state;
pub mod workspace;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, time::Duration};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{broadcast, mpsc};

use protocol::{
    AttentionLevel, Command, Event, GitState, RepositoryId, RepositorySummary, SavedCommand,
    SshTarget, WorkspaceId, WorkspaceSummary,
};
use state::{AppState, Repository, Workspace};
use uuid::Uuid;

use foreground_commands::{ForegroundCommandKey, ForegroundCommandStore};

/// Result of a background git refresh for one workspace.
struct GitRefreshResult {
    id: Uuid,
    result: Result<GitState, anyhow::Error>,
}

async fn forward_workspace_command_output<R>(
    mut reader: R,
    evt_tx: broadcast::Sender<Event>,
    id: Uuid,
    cwd: String,
    stream: &'static str,
) where
    R: AsyncRead + Unpin,
{
    let mut buf = vec![0u8; 8192];
    loop {
        let Ok(n) = reader.read(&mut buf).await else {
            break;
        };
        if n == 0 {
            break;
        }
        let _ = evt_tx.send(Event::WorkspaceCommandOutput {
            id,
            cwd: cwd.clone(),
            stream: stream.to_string(),
            data: String::from_utf8_lossy(&buf[..n]).to_string(),
        });
    }
}

async fn run_workspace_shell_command(
    id: Uuid,
    path: PathBuf,
    ssh: Option<SshTarget>,
    command: String,
    evt_tx: broadcast::Sender<Event>,
    git_tx: mpsc::Sender<GitRefreshResult>,
) {
    let cwd = path.display().to_string();
    let mut shell_cmd = ssh::build_shell_command(ssh.as_ref(), &path, &command);
    shell_cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let exit_code = match shell_cmd.spawn() {
        Ok(mut child) => {
            let stdout_task = child.stdout.take().map(|stdout| {
                tokio::spawn(forward_workspace_command_output(
                    stdout,
                    evt_tx.clone(),
                    id,
                    cwd.clone(),
                    "stdout",
                ))
            });
            let stderr_task = child.stderr.take().map(|stderr| {
                tokio::spawn(forward_workspace_command_output(
                    stderr,
                    evt_tx.clone(),
                    id,
                    cwd.clone(),
                    "stderr",
                ))
            });
            let status = child.wait().await.ok();
            if let Some(task) = stdout_task {
                let _ = task.await;
            }
            if let Some(task) = stderr_task {
                let _ = task.await;
            }
            status.and_then(|status| status.code())
        }
        Err(err) => {
            let _ = evt_tx.send(Event::WorkspaceCommandOutput {
                id,
                cwd: cwd.clone(),
                stream: "stderr".to_string(),
                data: err.to_string(),
            });
            None
        }
    };

    let _ = evt_tx.send(Event::WorkspaceCommandResult {
        id,
        cwd,
        command,
        exit_code,
    });
    let _ = git_tx
        .send(GitRefreshResult {
            id,
            result: refresh_git(&path, ssh.as_ref()).await,
        })
        .await;
}

/// Grace window after a local keystroke during which the agent terminal's
/// echoed output is ignored for the spinner / review / prompt heuristics, so
/// typing doesn't flash `agent_active` or trip false `NeedsInput`. Distinct from
/// the 500 ms `SETTLE_MS` window, which still governs real agent output.
const USER_TYPING_GRACE_MS: u64 = 3000;

/// Signal from an agent's output task to the event loop about review state.
/// The loop owns `AppState` (and thus git state), so it makes the final
/// ready-for-review decision; the task only reports agent activity.
enum ReviewSignal {
    /// Agent terminal went quiet past the settle window without a prompt.
    AgentSettled(Uuid),
    /// Agent terminal produced new output (it's actively working).
    AgentActive(Uuid),
    /// Agent terminal process exited. Clears any typing-grace window before
    /// settling so the final review state is always recomputed.
    AgentExited(Uuid),
}

/// Result of a background worktree-create task, sent back to the event loop so
/// the new Workspace is registered on the state-owning thread.
struct WorkspaceCreateOutcome {
    id: Uuid,
    repo_id: RepositoryId,
    name: String,
    path: PathBuf,
    branch: String,
    base_branch: String,
    agent: Option<String>,
    ssh: Option<SshTarget>,
    result: Result<(), anyhow::Error>,
}

use workspace::attention::AttentionDetector;
use workspace::git::{
    checkout_branch, checkout_remote_branch, commit, create_branch, create_worktree,
    delete_local_branch, delete_remote_branch, detect_default_branch, diff_branch_file,
    diff_branch_files, diff_commit, diff_commit_file, diff_file, discard_all, discard_file,
    gh_create_pr, git_fetch, git_pull, git_push, git_stash, git_stash_all, git_stash_pull_pop,
    list_commit_files, list_worktrees, refresh_git, remote_origin_url, remove_worktree, repo_root,
    stage_all,
    stage_file, unstage_all, unstage_file,
};
use workspace::process_info;
use workspace::ssh;
use workspace::terminal::{start_terminal, TerminalOutput};

#[derive(Clone)]
pub struct CoreHandle {
    pub cmd_tx: mpsc::Sender<Command>,
    pub evt_tx: broadcast::Sender<Event>,
}

pub fn spawn_core() -> CoreHandle {
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(1024);
    let cmd_tx_internal = cmd_tx.clone();
    let (evt_tx, _) = broadcast::channel::<Event>(16384);
    let evt_tx_task = evt_tx.clone();

    tokio::spawn(async move {
        let mut state = AppState::default();
        restore_repositories(&mut state).await;
        restore_workspaces(&mut state, &evt_tx_task).await;
        // Surface any on-disk worktrees that aren't tracked yet as Workspaces
        // under their owning Repository (created outside the app, or in a
        // session whose persisted state was lost). Done before the initial list
        // emits so the sidebar shows them on first paint.
        {
            let repo_ids = state.ordered_repo_ids.clone();
            let mut picked_up = Vec::new();
            for repo_id in repo_ids {
                picked_up.extend(pickup_worktrees_for_repo(&mut state, repo_id).await);
            }
            if !picked_up.is_empty() {
                save_workspaces(&state);
                for id in picked_up {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let _ = evt_tx_task.send(Event::WorkspaceGitUpdated {
                            id,
                            git: ws.git.clone(),
                        });
                    }
                }
            }
        }
        let _ = evt_tx_task.send(Event::RepositoryList {
            items: repository_summaries(&state),
        });
        let _ = evt_tx_task.send(Event::WorkspaceList {
            items: workspace_summaries(&state),
        });
        let mut git_tick = tokio::time::interval(Duration::from_secs(2));
        let mut git_refresh_in_flight = false;
        let (git_result_tx, mut git_result_rx) = mpsc::channel::<GitRefreshResult>(64);
        let mut fg_tick = tokio::time::interval(Duration::from_secs(1));
        let mut last_fg: HashMap<(Uuid, String), Option<SavedCommand>> = HashMap::new();
        let foreground_commands_path = foreground_commands_persist_file();
        let mut foreground_commands = foreground_commands_path
            .as_deref()
            .map(ForegroundCommandStore::load)
            .unwrap_or_default();
        let mut pending_resurrections: HashSet<ForegroundCommandKey> =
            foreground_commands.keys().collect();
        let (created_ws_tx, mut created_ws_rx) = mpsc::channel::<WorkspaceCreateOutcome>(16);
        let (review_tx, mut review_rx) = mpsc::channel::<ReviewSignal>(64);

        loop {
            tokio::select! {
                maybe_cmd = cmd_rx.recv() => {
                    let Some(cmd) = maybe_cmd else { break; };
                    match cmd {
                Command::SetRoute(route) => state.route = route,
                // --- Repository registry + worktree lifecycle ---
                Command::RegisterRepository {
                    name,
                    path,
                    ssh,
                    default_agent,
                    worktree_root,
                } => {
                    let candidate = PathBuf::from(&path);
                    match repo_root(&candidate, ssh.as_ref()).await {
                        Ok(root) => {
                            let default_branch =
                                detect_default_branch(&root, ssh.as_ref()).await.ok();
                            let id = Uuid::new_v4();
                            let repo = Repository {
                                id,
                                name,
                                path: root,
                                default_branch,
                                worktree_root: worktree_root.map(PathBuf::from),
                                default_agent,
                                ssh,
                            };
                            state.ordered_repo_ids.push(id);
                            state.repositories.insert(id, repo);
                            save_repositories(&state);
                            let _ = evt_tx_task.send(Event::RepositoryList {
                                items: repository_summaries(&state),
                            });
                            // Auto-pickup any worktrees that already exist on
                            // disk for this freshly registered repo.
                            let added = pickup_worktrees_for_repo(&mut state, id).await;
                            if !added.is_empty() {
                                save_workspaces(&state);
                                let _ = evt_tx_task.send(Event::RepositoryList {
                                    items: repository_summaries(&state),
                                });
                                let _ = evt_tx_task.send(Event::WorkspaceList {
                                    items: workspace_summaries(&state),
                                });
                                for ws_id in added {
                                    if let Some(ws) = state.workspaces.get(&ws_id) {
                                        let _ = evt_tx_task.send(Event::WorkspaceGitUpdated {
                                            id: ws_id,
                                            git: ws.git.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            let _ = evt_tx_task.send(Event::Error {
                                message: format!("Cannot register repository: {e}"),
                            });
                        }
                    }
                }
                Command::RemoveRepository { repo_id } => {
                    state.repositories.remove(&repo_id);
                    state.ordered_repo_ids.retain(|rid| *rid != repo_id);
                    save_repositories(&state);
                    let _ = evt_tx_task.send(Event::RepositoryList {
                        items: repository_summaries(&state),
                    });
                }
                Command::CreateWorkspace {
                    repo_id,
                    name,
                    base_branch,
                    agent,
                } => {
                    if let Some(repo) = state.repositories.get(&repo_id) {
                        let id = Uuid::new_v4();
                        let slug = {
                            let s = protocol::branch_slug(&name);
                            if s.is_empty() {
                                format!("ws-{}", &id.simple().to_string()[..8])
                            } else {
                                s
                            }
                        };
                        let base = base_branch
                            .or_else(|| repo.default_branch.clone())
                            .unwrap_or_else(|| "main".to_string());
                        let wt_path = worktree_path_for(repo, &slug);
                        let repo_path = repo.path.clone();
                        let ssh = repo.ssh.clone();
                        let display_name = if name.trim().is_empty() {
                            slug.clone()
                        } else {
                            name
                        };

                        let _ = evt_tx_task.send(Event::WorktreeCreateProgress {
                            repo_id,
                            stage: "fetch".to_string(),
                        });

                        let tx = created_ws_tx.clone();
                        let evt_progress = evt_tx_task.clone();
                        tokio::spawn(async move {
                            // Best-effort fetch so the worktree starts from latest upstream.
                            let _ = git_fetch(&repo_path, ssh.as_ref()).await;
                            let _ = evt_progress.send(Event::WorktreeCreateProgress {
                                repo_id,
                                stage: "worktree-add".to_string(),
                            });
                            let remote_ref = format!("origin/{base}");
                            let mut result =
                                create_worktree(&repo_path, &wt_path, &slug, &remote_ref, ssh.as_ref())
                                    .await;
                            if result.is_err() {
                                // Repo may have no `origin` remote — fall back to a local base ref.
                                result =
                                    create_worktree(&repo_path, &wt_path, &slug, &base, ssh.as_ref())
                                        .await;
                            }
                            let _ = tx
                                .send(WorkspaceCreateOutcome {
                                    id,
                                    repo_id,
                                    name: display_name,
                                    path: wt_path,
                                    branch: slug,
                                    base_branch: base,
                                    agent,
                                    ssh,
                                    result,
                                })
                                .await;
                        });
                    } else {
                        let _ = evt_tx_task.send(Event::Error {
                            message: "Cannot create workspace: unknown repository".to_string(),
                        });
                    }
                }
                Command::SetReadyForReview { id, ready } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        ws.review_manual = true;
                        ws.ready_for_review = ready;
                        let _ = evt_tx_task.send(Event::WorkspaceReviewChanged { id, ready });
                    }
                }
                Command::LoadBranchDiff { id } => {
                    if let Some((path, ssh, base)) = workspace_base_for_diff(&state, id) {
                        let evt = evt_tx_task.clone();
                        tokio::spawn(async move {
                            match diff_branch_files(&path, &base, ssh.as_ref()).await {
                                Ok(files) => {
                                    let _ = evt.send(Event::BranchDiffFilesLoaded { id, base, files });
                                }
                                Err(e) => {
                                    let _ = evt.send(Event::Error {
                                        message: format!("branch diff failed: {e}"),
                                    });
                                }
                            }
                        });
                    }
                }
                Command::LoadBranchFileDiff { id, file } => {
                    if let Some((path, ssh, base)) = workspace_base_for_diff(&state, id) {
                        let evt = evt_tx_task.clone();
                        tokio::spawn(async move {
                            match diff_branch_file(&path, &base, &file, ssh.as_ref()).await {
                                Ok(diff) => {
                                    let _ = evt.send(Event::WorkspaceDiffUpdated { id, file, diff });
                                }
                                Err(e) => {
                                    let _ = evt.send(Event::Error {
                                        message: format!("branch file diff failed: {e}"),
                                    });
                                }
                            }
                        });
                    }
                }
                Command::OpenPullRequest { id } => {
                    let info = state.workspaces.get(&id).map(|ws| {
                        (
                            ws.path.clone(),
                            ws.ssh.clone(),
                            ws.branch.clone().unwrap_or_default(),
                        )
                    });
                    if let Some((path, ssh, branch)) = info {
                        let base = workspace_base_for_diff(&state, id)
                            .map(|(_, _, b)| b)
                            .unwrap_or_else(|| "main".to_string());
                        let evt = evt_tx_task.clone();
                        tokio::spawn(async move {
                            // 1. Push the branch (reuses existing push-to-origin).
                            if let Err(e) = git_push(&path, ssh.as_ref()).await {
                                let _ = evt.send(Event::GitActionResult {
                                    id,
                                    action: "open_pr".to_string(),
                                    success: false,
                                    message: format!("push failed: {e}"),
                                });
                                return;
                            }
                            // 2. Try to open a PR via gh; fall back to a compare URL.
                            match gh_create_pr(&path, &branch, ssh.as_ref()).await {
                                Ok(url) => {
                                    let _ = evt.send(Event::GitActionResult {
                                        id,
                                        action: "open_pr".to_string(),
                                        success: true,
                                        message: url,
                                    });
                                }
                                Err(_) => {
                                    let message = match remote_origin_url(&path, ssh.as_ref()).await {
                                        Ok(origin) => github_compare_url(&origin, &base, &branch)
                                            .map(|u| format!("pushed — open PR: {u}"))
                                            .unwrap_or_else(|| {
                                                "pushed — create PR manually (gh unavailable)"
                                                    .to_string()
                                            }),
                                        Err(_) => "pushed — create PR manually (gh unavailable)"
                                            .to_string(),
                                    };
                                    let _ = evt.send(Event::GitActionResult {
                                        id,
                                        action: "open_pr".to_string(),
                                        success: true,
                                        message,
                                    });
                                }
                            }
                        });
                    }
                }
                Command::AddWorkspace { name, path, ssh } => {
                    let id = Uuid::new_v4();
                    let repo_path = std::path::PathBuf::from(&path);

                    let ws = Workspace {
                        id,
                        name,
                        path: repo_path.clone(),
                        ssh: ssh.clone(),
                        git: GitState::default(),
                        attention: AttentionLevel::None,
                        terminals: Default::default(),
                        last_activity: Instant::now(),
                        repository_id: None,
                        branch: None,
                        base_branch: None,
                        agent: None,
                        ready_for_review: false,
                        review_manual: false,
                        agent_active: false,
                        agent_idle: false,
                        agent_input_suppress_until: None,
                    };
                    state.ordered_ids.push(id);
                    state.workspaces.insert(id, ws);
                    let _ = evt_tx_task.send(Event::WorkspaceGitUpdated {
                        id,
                        git: GitState::default(),
                    });
                    let git_tx = git_result_tx.clone();
                    tokio::spawn(async move {
                        let _ = git_tx.send(GitRefreshResult {
                            id,
                            result: refresh_git(&repo_path, ssh.as_ref()).await,
                        }).await;
                    });
                }
                Command::RemoveWorkspace { id } => {
                    let removed = state.workspaces.remove(&id);
                    if let Some(ws) = &removed {
                        let removed_path = ws.path.to_string_lossy().into_owned();
                        let changed = foreground_commands.remove_workspace(&ws.path);
                        pending_resurrections.retain(|key| key.workspace_path != removed_path);
                        if changed {
                            save_foreground_commands(
                                &foreground_commands_path,
                                &foreground_commands,
                            );
                        }
                    }
                    state.ordered_ids.retain(|wid| *wid != id);
                    if let Some(ws) = removed {
                        // Tear down the git worktree for worktree-backed Workspaces.
                        if let Some(repo_id) = ws.repository_id {
                            if let Some(repo) = state.repositories.get(&repo_id) {
                                let repo_path = repo.path.clone();
                                let wt_path = ws.path.clone();
                                let ssh = ws.ssh.clone();
                                tokio::spawn(async move {
                                    let _ =
                                        remove_worktree(&repo_path, &wt_path, ssh.as_ref()).await;
                                });
                            }
                            let _ = evt_tx_task.send(Event::RepositoryList {
                                items: repository_summaries(&state),
                            });
                        }
                    }
                }
                Command::RenameWorkspace { id, name } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        ws.name = name;
                        ws.last_activity = Instant::now();
                    }
                }
                Command::MoveWorkspace { id, delta } => {
                    if let Some(pos) = state.ordered_ids.iter().position(|wid| *wid == id) {
                        let new_pos = (pos as i32 + delta)
                            .max(0)
                            .min(state.ordered_ids.len() as i32 - 1) as usize;
                        if pos != new_pos {
                            let removed = state.ordered_ids.remove(pos);
                            state.ordered_ids.insert(new_pos, removed);
                        }
                    }
                }
                Command::SetAttention { id, level } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        // Drop prompt-detection `NeedsInput` triggered by local
                        // typing echo; `Error` is always honoured.
                        if should_suppress_attention(ws, level, Instant::now()) {
                            continue;
                        }
                        ws.attention = level;
                        let review_changed = recompute_review(ws);
                        let ready = ws.ready_for_review;
                        let _ = evt_tx_task.send(Event::WorkspaceAttentionChanged { id, level });
                        if review_changed {
                            let _ = evt_tx_task
                                .send(Event::WorkspaceReviewChanged { id, ready });
                        }
                    }
                }
                Command::ClearAttention { id } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        ws.attention = AttentionLevel::None;
                        let _ = evt_tx_task.send(Event::WorkspaceAttentionChanged {
                            id,
                            level: AttentionLevel::None,
                        });
                    }
                }
                Command::RefreshGit { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let result = refresh_git(&path, ssh.as_ref()).await;
                            if let Err(ref err) = result {
                                let _ = evt_tx.send(Event::Error {
                                    message: format!(
                                        "RefreshGit failed for {}: {err}",
                                        path.display()
                                    ),
                                });
                            }
                            let _ = git_tx.send(GitRefreshResult { id, result }).await;
                        });
                    }
                }
                Command::RunWorkspaceCommand { id, command } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            run_workspace_shell_command(id, path, ssh, command, evt_tx, git_tx)
                                .await;
                        });
                    }
                }
                Command::LoadDiff { id, file } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        tokio::spawn(async move {
                            match diff_file(&path, &file, ssh.as_ref()).await {
                                Ok(diff) => {
                                    let _ = evt_tx.send(Event::WorkspaceDiffUpdated {
                                        id,
                                        file,
                                        diff,
                                    });
                                }
                                Err(err) => {
                                    let _ = evt_tx.send(Event::Error {
                                        message: format!(
                                            "LoadDiff failed for {}: {err}",
                                            path.display()
                                        ),
                                    });
                                }
                            }
                        });
                    }
                }
                Command::LoadCommitDiff { id, hash } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        tokio::spawn(async move {
                            match diff_commit(&path, &hash, ssh.as_ref()).await {
                                Ok(diff) => {
                                    let _ = evt_tx.send(Event::WorkspaceDiffUpdated {
                                        id,
                                        file: hash,
                                        diff,
                                    });
                                }
                                Err(err) => {
                                    let _ = evt_tx.send(Event::Error {
                                        message: format!(
                                            "LoadCommitDiff failed for {}: {err}",
                                            path.display()
                                        ),
                                    });
                                }
                            }
                        });
                    }
                }
                Command::LoadCommitFiles { id, hash } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        tokio::spawn(async move {
                            match list_commit_files(&path, &hash, ssh.as_ref()).await {
                                Ok(files) => {
                                    let _ = evt_tx.send(Event::CommitFilesLoaded {
                                        id,
                                        hash,
                                        files,
                                    });
                                }
                                Err(err) => {
                                    let _ = evt_tx.send(Event::Error {
                                        message: format!(
                                            "LoadCommitFiles failed for {}: {err}",
                                            path.display()
                                        ),
                                    });
                                }
                            }
                        });
                    }
                }
                Command::LoadCommitFileDiff { id, hash, file } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        tokio::spawn(async move {
                            match diff_commit_file(&path, &hash, &file, ssh.as_ref()).await {
                                Ok(diff) => {
                                    let _ = evt_tx.send(Event::WorkspaceDiffUpdated {
                                        id,
                                        file: format!("{hash}:{file}"),
                                        diff,
                                    });
                                }
                                Err(err) => {
                                    let _ = evt_tx.send(Event::Error {
                                        message: format!(
                                            "LoadCommitFileDiff failed for {}: {err}",
                                            path.display()
                                        ),
                                    });
                                }
                            }
                        });
                    }
                }
                Command::GitStageFile { id, file } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, message) = match stage_file(&path, &file, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Staged {file}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "stage".to_string(), success, message,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitUnstageFile { id, file } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, message) = match unstage_file(&path, &file, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Unstaged {file}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "unstage".to_string(), success, message,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitStageAll { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, message) = match stage_all(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Staged all".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "stage_all".to_string(), success, message,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitUnstageAll { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, message) = match unstage_all(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Unstaged all".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "unstage_all".to_string(), success, message,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitCommit { id, message } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match commit(&path, &message, ssh.as_ref()).await {
                                Ok(()) => (true, "Committed".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "commit".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitCheckoutBranch { id, branch } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match checkout_branch(&path, &branch, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Checked out {branch}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "checkout".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitCheckoutRemoteBranch { id, remote_branch, local_name } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match checkout_remote_branch(&path, &remote_branch, &local_name, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Created and checked out {local_name} from {remote_branch}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "checkout".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitCreateBranch { id, branch } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match create_branch(&path, &branch, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Created and checked out {branch}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "create_branch".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitDeleteLocalBranch { id, branch } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match delete_local_branch(&path, &branch, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Deleted local branch {branch}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "delete_branch".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitDeleteRemoteBranch { id, remote, branch } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match delete_remote_branch(&path, &remote, &branch, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Deleted remote branch {remote}/{branch}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "delete_remote_branch".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitPush { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match git_push(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Pushed".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "push".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitPull { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, action, msg) = match git_pull(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "pull".to_string(), "Pulled".to_string()),
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    if err_msg.starts_with("DIRTY_TREE:") {
                                        (false, "pull_dirty_tree".to_string(), err_msg)
                                    } else {
                                        (false, "pull".to_string(), err_msg)
                                    }
                                }
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action, success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitFetch { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match git_fetch(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Fetched".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "fetch".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitDiscardFile { id, file } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let (idx_status, wt_status) = ws.git.changed.iter()
                            .find(|c| c.path == file)
                            .map(|c| (c.index_status, c.worktree_status))
                            .unwrap_or((' ', ' '));
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match discard_file(&path, &file, idx_status, wt_status, ssh.as_ref()).await {
                                Ok(()) => (true, format!("Discarded {file}")),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "discard".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitStash { id, message } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match git_stash(&path, message.as_deref(), ssh.as_ref()).await {
                                Ok(()) => (true, "Stashed".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "stash".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitStashPullPop { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match git_stash_pull_pop(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Pulled (stash-pull-pop)".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "pull".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitDiscardAll { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match discard_all(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Discarded all uncommitted changes".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "discard_all".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::GitStashAll { id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        let path = ws.path.clone();
                        let ssh = ws.ssh.clone();
                        let evt_tx = evt_tx_task.clone();
                        let git_tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let (success, msg) = match git_stash_all(&path, ssh.as_ref()).await {
                                Ok(()) => (true, "Stashed all (incl. untracked)".to_string()),
                                Err(e) => (false, e.to_string()),
                            };
                            let _ = evt_tx.send(Event::GitActionResult {
                                id, action: "stash".to_string(), success, message: msg,
                            });
                            let _ = git_tx.send(GitRefreshResult {
                                id, result: refresh_git(&path, ssh.as_ref()).await,
                            }).await;
                        });
                    }
                }
                Command::StartTerminal {
                    id,
                    kind,
                    tab_id,
                    cmd,
                    cols,
                    rows,
                } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        let cwd = ws.path.clone();
                        let ssh_target = ws.ssh.clone();
                        let command = if cmd.is_empty() {
                            default_terminal_cmd(kind)
                        } else {
                            cmd
                        };

                        let tid = normalize_tab_id(kind, tab_id);
                        let resurrection_key = if matches!(kind, protocol::TerminalKind::Shell)
                            && ssh_target.is_none()
                        {
                            Some(ForegroundCommandKey::new(&cwd, tid.clone()))
                        } else {
                            None
                        };
                        let already_running = match kind {
                            protocol::TerminalKind::Agent => ws
                                .terminals
                                .agent
                                .as_ref()
                                .map(|s| s.is_alive())
                                .unwrap_or(false),
                            protocol::TerminalKind::Shell => ws
                                .terminals
                                .shells
                                .get(&tid)
                                .map(|s| s.is_alive())
                                .unwrap_or(false),
                        };
                        if already_running {
                            ws.last_activity = Instant::now();
                            if let Some(key) = &resurrection_key {
                                emit_pending_shell_resurrection(
                                    &evt_tx_task,
                                    id,
                                    &tid,
                                    key,
                                    &foreground_commands,
                                    &pending_resurrections,
                                );
                            }
                        } else {
                            match kind {
                                protocol::TerminalKind::Agent => {
                                    if let Some(existing) = ws.terminals.agent.take() {
                                        let _ = existing.stop().await;
                                    }
                                    ws.agent_active = false;
                                    ws.agent_idle = false;
                                    ws.agent_input_suppress_until = None;
                                }
                                protocol::TerminalKind::Shell => {
                                    if let Some(existing) = ws.terminals.shells.remove(&tid) {
                                        let _ = existing.stop().await;
                                    }
                                }
                            }

                            match start_terminal(cwd, command, ssh_target.as_ref(), cols, rows)
                                .await
                            {
                                Ok((session, mut out_rx)) => {
                                    match kind {
                                        protocol::TerminalKind::Agent => {
                                            ws.terminals.agent = Some(session)
                                        }
                                        protocol::TerminalKind::Shell => {
                                            ws.terminals.shells.insert(tid.clone(), session);
                                        }
                                    }
                                    ws.last_activity = Instant::now();
                                    let _ = evt_tx_task.send(Event::TerminalStarted {
                                        id,
                                        kind,
                                        tab_id: Some(tid.clone()),
                                    });
                                    if let Some(key) = &resurrection_key {
                                        emit_pending_shell_resurrection(
                                            &evt_tx_task,
                                            id,
                                            &tid,
                                            key,
                                            &foreground_commands,
                                            &pending_resurrections,
                                        );
                                    }

                                    let evt_tx_outputs = evt_tx_task.clone();
                                    let cmd_tx_outputs = cmd_tx_internal.clone();
                                    let review_outputs = review_tx.clone();
                                    let out_tab_id = tid.clone();
                                    tokio::spawn(async move {
                                    let mut detector = AttentionDetector::new();
                                    let mut attention_active = false;
                                    const SETTLE_MS: u64 = 500;

                                    let is_agent = matches!(kind, protocol::TerminalKind::Agent);
                                    let mut settle_deadline: Option<tokio::time::Instant> = None;

                                    loop {
                                        let out = if is_agent {
                                            if let Some(deadline) = settle_deadline {
                                                tokio::select! {
                                                    maybe_out = out_rx.recv() => { maybe_out }
                                                    _ = tokio::time::sleep_until(deadline) => {
                                                        settle_deadline = None;
                                                        if detector.check_for_prompt() {
                                                            if !attention_active {
                                                                attention_active = true;
                                                                let _ = cmd_tx_outputs
                                                                    .send(Command::SetAttention {
                                                                        id,
                                                                        level: AttentionLevel::NeedsInput,
                                                                    })
                                                                    .await;
                                                            }
                                                        } else {
                                                            if attention_active {
                                                                attention_active = false;
                                                                let _ = cmd_tx_outputs
                                                                    .send(Command::ClearAttention { id })
                                                                    .await;
                                                            }
                                                            // Agent went quiet without a prompt —
                                                            // a candidate for ready-for-review.
                                                            let _ = review_outputs
                                                                .send(ReviewSignal::AgentSettled(id))
                                                                .await;
                                                        }
                                                        continue;
                                                    }
                                                }
                                            } else {
                                                out_rx.recv().await
                                            }
                                        } else {
                                            out_rx.recv().await
                                        };

                                        let Some(out) = out else { break; };

                                        match out {
                                            TerminalOutput::Bytes(bytes) => {
                                                if is_agent {
                                                    let has_content = detector.append(&bytes);
                                                    if has_content {
                                                        settle_deadline = Some(
                                                            tokio::time::Instant::now()
                                                                + Duration::from_millis(SETTLE_MS),
                                                        );
                                                        if attention_active {
                                                            attention_active = false;
                                                            let _ = cmd_tx_outputs
                                                                .send(Command::ClearAttention { id })
                                                                .await;
                                                        }
                                                        let _ = review_outputs
                                                            .send(ReviewSignal::AgentActive(id))
                                                            .await;
                                                    }
                                                    // ANSI-only: has_content=false → settle_deadline unchanged
                                                }
                                                let data_b64 =
                                                    base64::engine::general_purpose::STANDARD
                                                        .encode(bytes);
                                                let _ =
                                                    evt_tx_outputs.send(Event::TerminalOutput {
                                                        id,
                                                        kind,
                                                        tab_id: Some(out_tab_id.clone()),
                                                        data_b64,
                                                    });
                                            }
                                            TerminalOutput::Exited(code) => {
                                                if is_agent {
                                                    let _ = review_outputs
                                                        .send(ReviewSignal::AgentExited(id))
                                                        .await;
                                                }
                                                let _ = evt_tx_outputs.send(Event::TerminalExited {
                                                    id,
                                                    kind,
                                                    tab_id: Some(out_tab_id.clone()),
                                                    code,
                                                });
                                                break;
                                            }
                                        }
                                    }
                                    });
                                }
                                Err(err) => {
                                    let _ = evt_tx_task.send(Event::Error {
                                        message: format!(
                                            "StartTerminal failed for workspace {id}: {err}"
                                        ),
                                    });
                                }
                            }
                        }
                    }
                }
                Command::StopTerminal { id, kind, tab_id } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        let tid = normalize_tab_id(kind, tab_id);
                        let resurrection_key = if matches!(kind, protocol::TerminalKind::Shell)
                            && ws.ssh.is_none()
                        {
                            Some(ForegroundCommandKey::new(&ws.path, tid.clone()))
                        } else {
                            None
                        };
                        let stopped = match kind {
                            protocol::TerminalKind::Agent => {
                                ws.agent_active = false;
                                ws.agent_input_suppress_until = None;
                                ws.terminals.agent.take()
                            }
                            protocol::TerminalKind::Shell => ws.terminals.shells.remove(&tid),
                        };
                        if let Some(session) = stopped {
                            let _ = session.stop().await;
                            let _ = evt_tx_task.send(Event::TerminalExited {
                                id,
                                kind,
                                tab_id: Some(tid.clone()),
                                code: None,
                            });
                        }
                        if let Some(key) = resurrection_key {
                            if clear_shell_resurrection_state(
                                &mut foreground_commands,
                                &mut pending_resurrections,
                                &key,
                            ) {
                                save_foreground_commands(
                                    &foreground_commands_path,
                                    &foreground_commands,
                                );
                            }
                            let _ = evt_tx_task.send(Event::ShellResurrectionChanged {
                                id,
                                tab_id: tid,
                                command: None,
                            });
                        }
                    }
                }
                Command::SendTerminalInput {
                    id,
                    kind,
                    tab_id,
                    data_b64,
                } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        let tid = normalize_tab_id(kind, tab_id);
                        let session = match kind {
                            protocol::TerminalKind::Agent => ws.terminals.agent.as_mut(),
                            protocol::TerminalKind::Shell => ws.terminals.shells.get_mut(&tid),
                        };
                        if let Some(session) = session {
                            if let Ok(bytes) =
                                base64::engine::general_purpose::STANDARD.decode(data_b64)
                            {
                                let _ = session.send_input(&bytes).await;
                                if matches!(kind, protocol::TerminalKind::Agent) {
                                    // Local keystrokes echo back as agent output;
                                    // arm a grace window so that echo doesn't flash
                                    // the spinner or trip prompt/review heuristics.
                                    // The per-command WorkspaceList emit below
                                    // propagates any cleared `agent_active`.
                                    let _ = apply_agent_input(ws, &bytes, Instant::now());
                                    if ws.attention == AttentionLevel::NeedsInput {
                                        ws.attention = AttentionLevel::None;
                                        let _ =
                                            evt_tx_task.send(Event::WorkspaceAttentionChanged {
                                                id,
                                                level: AttentionLevel::None,
                                            });
                                    }
                                }
                            }
                        }
                    }
                }
                Command::ResizeTerminal {
                    id,
                    kind,
                    tab_id,
                    cols,
                    rows,
                } => {
                    if let Some(ws) = state.workspaces.get_mut(&id) {
                        let tid = normalize_tab_id(kind, tab_id);
                        let session = match kind {
                            protocol::TerminalKind::Agent => ws.terminals.agent.as_mut(),
                            protocol::TerminalKind::Shell => ws.terminals.shells.get_mut(&tid),
                        };
                        if let Some(session) = session {
                            let _ = session.resize(cols, rows).await;
                        }
                    }
                }
                Command::ClearShellResurrection { id, tab_id } => {
                    if let Some(ws) = state.workspaces.get(&id) {
                        if ws.ssh.is_none() {
                            let key = ForegroundCommandKey::new(&ws.path, tab_id.clone());
                            if clear_shell_resurrection_state(
                                &mut foreground_commands,
                                &mut pending_resurrections,
                                &key,
                            ) {
                                save_foreground_commands(
                                    &foreground_commands_path,
                                    &foreground_commands,
                                );
                            }
                            let _ = evt_tx_task.send(Event::ShellResurrectionChanged {
                                id,
                                tab_id,
                                command: None,
                            });
                        }
                    }
                }
            }

            save_workspaces(&state);
            let items = workspace_summaries(&state);
            let _ = evt_tx_task.send(Event::WorkspaceList { items });
                }
                _ = git_tick.tick(), if !git_refresh_in_flight => {
                    let pairs: Vec<_> = state.ordered_ids.iter()
                        .filter_map(|id| state.workspaces.get(id).map(|ws| (*id, ws.path.clone(), ws.ssh.clone())))
                        .collect();
                    if !pairs.is_empty() {
                        git_refresh_in_flight = true;
                        let tx = git_result_tx.clone();
                        tokio::spawn(async move {
                            let results = futures::future::join_all(
                                pairs.into_iter().map(|(id, path, ssh)| {
                                    async move {
                                        GitRefreshResult { id, result: refresh_git(&path, ssh.as_ref()).await }
                                    }
                                })
                            ).await;
                            for r in results {
                                let _ = tx.send(r).await;
                            }
                        });
                    }
                }
                Some(gr) = git_result_rx.recv() => {
                    if let Ok(git) = gr.result {
                        let mut review_changed = false;
                        if let Some(ws) = state.workspaces.get_mut(&gr.id) {
                            ws.git = git.clone();
                            ws.last_activity = Instant::now();
                            review_changed = recompute_review(ws);
                        }
                        let _ = evt_tx_task.send(Event::WorkspaceGitUpdated { id: gr.id, git });
                        if review_changed {
                            if let Some(ws) = state.workspaces.get(&gr.id) {
                                let _ = evt_tx_task.send(Event::WorkspaceReviewChanged {
                                    id: gr.id,
                                    ready: ws.ready_for_review,
                                });
                            }
                        }
                    }
                    // When channel is drained (no more pending), clear in-flight flag
                    if git_result_rx.is_empty() {
                        git_refresh_in_flight = false;
                        let _ = evt_tx_task.send(Event::WorkspaceList {
                            items: workspace_summaries(&state),
                        });
                    }
                }
                _ = fg_tick.tick() => {
                    let mut current_keys: HashSet<(Uuid, String)> = HashSet::new();
                    for (id, ws) in &state.workspaces {
                        if ws.ssh.is_some() {
                            // SSH resurrection is intentionally unsupported for v1:
                            // argv/cwd capture relies on the local PTY process table.
                            continue;
                        }
                        for (tab_id, sess) in &ws.terminals.shells {
                            let key = (*id, tab_id.clone());
                            current_keys.insert(key.clone());
                            let resurrection_key =
                                ForegroundCommandKey::new(&ws.path, tab_id.clone());
                            let new_cmd = if !sess.is_alive() {
                                None
                            } else {
                                let pgid = sess.foreground_pgid();
                                let shell_pid = sess.shell_pid().map(|p| p as i32);
                                match (pgid, shell_pid) {
                                    (Some(pg), Some(sp)) if pg > 0 && pg != sp => {
                                        process_info::lookup(pg).map(|fi| SavedCommand {
                                            argv: fi.argv,
                                            cwd: fi.cwd.to_string_lossy().into_owned(),
                                        })
                                    }
                                    _ => None,
                                }
                            };
                            let prev = last_fg.get(&key).cloned().unwrap_or(None);
                            let observation = apply_foreground_observation(
                                &mut foreground_commands,
                                &mut pending_resurrections,
                                &resurrection_key,
                                new_cmd.clone(),
                            );
                            if observation.persisted_changed {
                                save_foreground_commands(
                                    &foreground_commands_path,
                                    &foreground_commands,
                                );
                            }
                            if observation.cleared_pending {
                                let _ = evt_tx_task.send(Event::ShellResurrectionChanged {
                                    id: *id,
                                    tab_id: tab_id.clone(),
                                    command: None,
                                });
                            }
                            if prev != new_cmd {
                                last_fg.insert(key.clone(), new_cmd.clone());
                                let _ = evt_tx_task.send(Event::ShellForegroundChanged {
                                    id: *id,
                                    tab_id: tab_id.clone(),
                                    command: new_cmd,
                                });
                            }
                        }
                    }
                    last_fg.retain(|key, _| current_keys.contains(key));
                }
                Some(outcome) = created_ws_rx.recv() => {
                    match outcome.result {
                        Ok(()) => {
                            let id = outcome.id;
                            let ssh = outcome.ssh.clone();
                            let path = outcome.path.clone();
                            let ws = Workspace {
                                id,
                                name: outcome.name,
                                path: outcome.path,
                                ssh: outcome.ssh,
                                git: GitState::default(),
                                attention: AttentionLevel::None,
                                terminals: Default::default(),
                                last_activity: Instant::now(),
                                repository_id: Some(outcome.repo_id),
                                branch: Some(outcome.branch.clone()),
                                base_branch: Some(outcome.base_branch),
                                agent: outcome.agent,
                                ready_for_review: false,
                                review_manual: false,
                                agent_active: false,
                                agent_idle: false,
                                agent_input_suppress_until: None,
                            };
                            state.ordered_ids.push(id);
                            state.workspaces.insert(id, ws);
                            save_workspaces(&state);
                            let _ = evt_tx_task.send(Event::WorkspaceCreated {
                                id,
                                repo_id: outcome.repo_id,
                                slug: outcome.branch,
                            });
                            let _ = evt_tx_task.send(Event::RepositoryList {
                                items: repository_summaries(&state),
                            });
                            let _ = evt_tx_task.send(Event::WorkspaceList {
                                items: workspace_summaries(&state),
                            });
                            let git_tx = git_result_tx.clone();
                            tokio::spawn(async move {
                                let _ = git_tx.send(GitRefreshResult {
                                    id,
                                    result: refresh_git(&path, ssh.as_ref()).await,
                                }).await;
                            });
                        }
                        Err(e) => {
                            let _ = evt_tx_task.send(Event::Error {
                                message: format!("Failed to create workspace: {e}"),
                            });
                        }
                    }
                }
                Some(sig) = review_rx.recv() => {
                    let result = match sig {
                        ReviewSignal::AgentActive(id) => {
                            state
                                .workspaces
                                .get_mut(&id)
                                .map(|ws| (id, apply_agent_active(ws)))
                        }
                        ReviewSignal::AgentSettled(id) => {
                            state
                                .workspaces
                                .get_mut(&id)
                                .map(|ws| (id, apply_agent_settled(ws)))
                        }
                        ReviewSignal::AgentExited(id) => {
                            state.workspaces.get_mut(&id).map(|ws| {
                                // Exit ends any typing-grace window, so the final
                                // settle/review is always recomputed.
                                ws.agent_input_suppress_until = None;
                                (id, apply_agent_settled(ws))
                            })
                        }
                    };
                    if let Some((id, transition)) = result {
                        if transition.review_changed {
                            let _ = evt_tx_task.send(Event::WorkspaceReviewChanged {
                                id,
                                ready: transition.ready,
                            });
                        }
                        if transition.review_changed || transition.active_changed {
                            let _ = evt_tx_task.send(Event::WorkspaceList {
                                items: workspace_summaries(&state),
                            });
                        }
                    }
                }
            }
        }
    });

    CoreHandle { cmd_tx, evt_tx }
}

struct ForegroundObservation {
    persisted_changed: bool,
    cleared_pending: bool,
}

fn apply_foreground_observation(
    store: &mut ForegroundCommandStore,
    pending_resurrections: &mut HashSet<ForegroundCommandKey>,
    key: &ForegroundCommandKey,
    command: Option<SavedCommand>,
) -> ForegroundObservation {
    match command {
        Some(command) => {
            let cleared_pending = pending_resurrections.remove(key);
            let persisted_changed = store.set_key(key.clone(), command);
            ForegroundObservation {
                persisted_changed,
                cleared_pending,
            }
        }
        None => {
            if pending_resurrections.contains(key) {
                ForegroundObservation {
                    persisted_changed: false,
                    cleared_pending: false,
                }
            } else {
                ForegroundObservation {
                    persisted_changed: store.remove_key(key),
                    cleared_pending: false,
                }
            }
        }
    }
}

fn clear_shell_resurrection_state(
    store: &mut ForegroundCommandStore,
    pending_resurrections: &mut HashSet<ForegroundCommandKey>,
    key: &ForegroundCommandKey,
) -> bool {
    let removed_pending = pending_resurrections.remove(key);
    let removed_store = store.remove_key(key);
    removed_pending || removed_store
}

fn emit_pending_shell_resurrection(
    evt_tx: &broadcast::Sender<Event>,
    id: Uuid,
    tab_id: &str,
    key: &ForegroundCommandKey,
    store: &ForegroundCommandStore,
    pending_resurrections: &HashSet<ForegroundCommandKey>,
) {
    if !pending_resurrections.contains(key) {
        return;
    }
    if let Some(command) = store.get_key(key).cloned() {
        let _ = evt_tx.send(Event::ShellResurrectionChanged {
            id,
            tab_id: tab_id.to_string(),
            command: Some(command),
        });
    }
}

fn save_foreground_commands(path: &Option<PathBuf>, store: &ForegroundCommandStore) {
    if let Some(path) = path {
        let _ = store.save(path);
    }
}

/// Returns `true` if the input bytes submit a prompt (contain a carriage return
/// or newline) rather than being mid-line editing.
fn input_is_submit(bytes: &[u8]) -> bool {
    bytes.iter().any(|&b| b == b'\r' || b == b'\n')
}

/// Whether the workspace is inside a local-typing grace window at `now`.
fn agent_input_suppressed(ws: &Workspace, now: Instant) -> bool {
    ws.agent_input_suppress_until
        .map(|until| now < until)
        .unwrap_or(false)
}

/// Whether an inbound attention level should be dropped because it was triggered
/// by local typing echo. Only `NeedsInput` (prompt detection) is suppressed;
/// `Error` is always honoured.
fn should_suppress_attention(ws: &Workspace, level: AttentionLevel, now: Instant) -> bool {
    matches!(level, AttentionLevel::NeedsInput) && agent_input_suppressed(ws, now)
}

/// Apply local input to the agent terminal's typing-grace window. A submit
/// (Enter) ends the window immediately so the real response can show activity;
/// any other non-empty input (re)arms a [`USER_TYPING_GRACE_MS`] window and
/// clears the spinner. Returns `true` if `agent_active` had been visible and was
/// cleared, so the caller can refresh the sidebar.
fn apply_agent_input(ws: &mut Workspace, bytes: &[u8], now: Instant) -> bool {
    if bytes.is_empty() {
        return false;
    }
    if input_is_submit(bytes) {
        ws.agent_input_suppress_until = None;
        return false;
    }
    ws.agent_input_suppress_until = Some(now + Duration::from_millis(USER_TYPING_GRACE_MS));
    let was_active = ws.agent_active;
    ws.agent_active = false;
    was_active
}

struct ReviewTransition {
    review_changed: bool,
    ready: bool,
    active_changed: bool,
}

impl ReviewTransition {
    /// A transition that changes nothing — used when a signal is dropped because
    /// it was caused by local typing echo inside the grace window.
    fn noop(ws: &Workspace) -> Self {
        ReviewTransition {
            review_changed: false,
            ready: ws.ready_for_review,
            active_changed: false,
        }
    }
}

fn apply_agent_active(ws: &mut Workspace) -> ReviewTransition {
    if agent_input_suppressed(ws, Instant::now()) {
        return ReviewTransition::noop(ws);
    }
    ws.agent_idle = false;
    ws.review_manual = false;
    let active_changed = !ws.agent_active;
    ws.agent_active = true;
    let review_changed = ws.ready_for_review;
    ws.ready_for_review = false;
    ReviewTransition {
        review_changed,
        ready: false,
        active_changed,
    }
}

fn apply_agent_settled(ws: &mut Workspace) -> ReviewTransition {
    if agent_input_suppressed(ws, Instant::now()) {
        return ReviewTransition::noop(ws);
    }
    ws.agent_idle = true;
    let active_changed = ws.agent_active;
    ws.agent_active = false;
    let review_changed = recompute_review(ws);
    ReviewTransition {
        review_changed,
        ready: ws.ready_for_review,
        active_changed,
    }
}

/// Recomputes a workspace's heuristic `ready_for_review`. Returns true if the
/// value changed. A manual override (`review_manual`) suppresses the heuristic
/// until the agent becomes active again. Ready = agent went idle AND the
/// worktree has uncommitted-or-ahead changes AND it isn't awaiting input.
fn recompute_review(ws: &mut Workspace) -> bool {
    if ws.review_manual {
        return false;
    }
    let has_changes = !ws.git.changed.is_empty() || ws.git.ahead.unwrap_or(0) > 0;
    let should = ws.agent_idle && has_changes && ws.attention != AttentionLevel::NeedsInput;
    if should != ws.ready_for_review {
        ws.ready_for_review = should;
        true
    } else {
        false
    }
}

fn workspace_summaries(state: &AppState) -> Vec<WorkspaceSummary> {
    state
        .ordered_ids
        .iter()
        .filter_map(|id| state.workspaces.get(id))
        .map(|ws| {
            let ssh_host = ws.ssh.as_ref().map(|t| ssh::ssh_destination(t));
            WorkspaceSummary {
                id: ws.id,
                name: ws.name.clone(),
                path: ws.path.display().to_string(),
                branch: ws.git.branch.clone(),
                ahead: ws.git.ahead,
                behind: ws.git.behind,
                dirty_files: ws.git.changed.len(),
                attention: ws.attention,
                agent_running: ws.terminals.agent.is_some(),
                shell_running: !ws.terminals.shells.is_empty(),
                last_activity_unix_ms: unix_ms_now(),
                ssh_host,
                repository_id: ws.repository_id,
                base_branch: ws.base_branch.clone(),
                ready_for_review: ws.ready_for_review,
                agent_active: ws.agent_active,
                agent: ws.agent.clone(),
            }
        })
        .collect::<Vec<_>>()
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn default_terminal_cmd(kind: protocol::TerminalKind) -> Vec<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "zsh".to_string());
    match kind {
        protocol::TerminalKind::Agent => vec![shell.clone(), "-i".to_string()],
        protocol::TerminalKind::Shell => vec![shell, "-i".to_string()],
    }
}

fn normalize_tab_id(kind: protocol::TerminalKind, tab_id: Option<String>) -> String {
    match kind {
        protocol::TerminalKind::Agent => "agent".to_string(),
        protocol::TerminalKind::Shell => tab_id
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "shell".to_string()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedWorkspace {
    name: String,
    path: String,
    #[serde(default)]
    ssh: Option<SshTarget>,
    #[serde(default)]
    repository_id: Option<RepositoryId>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    base_branch: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

fn persist_file() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let base = PathBuf::from(home).join(".config/conduit");
    let file = if let Ok(session) = std::env::var("CONDUIT_SESSION_NAME") {
        let safe = sanitize_session_name(&session);
        format!("workspaces.{safe}.json")
    } else {
        "workspaces.json".to_string()
    };
    Some(base.join(file))
}

fn foreground_commands_persist_file() -> Option<PathBuf> {
    let base = config_dir()?.join("conduit");
    let file = if let Ok(session) = std::env::var("CONDUIT_SESSION_NAME") {
        let safe = sanitize_session_name(&session);
        format!("foreground_commands.{safe}.json")
    } else {
        "foreground_commands.json".to_string()
    };
    Some(base.join(file))
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        Some(PathBuf::from(xdg))
    } else {
        std::env::var("HOME")
            .ok()
            .map(|home| PathBuf::from(home).join(".config"))
    }
}

fn sanitize_session_name(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "default".to_string();
    }
    let mut out = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

fn save_workspaces(state: &AppState) {
    let Some(path) = persist_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let items = state
        .ordered_ids
        .iter()
        .filter_map(|id| state.workspaces.get(id))
        .map(|ws| PersistedWorkspace {
            name: ws.name.clone(),
            path: ws.path.display().to_string(),
            ssh: ws.ssh.clone(),
            repository_id: ws.repository_id,
            branch: ws.branch.clone(),
            base_branch: ws.base_branch.clone(),
            agent: ws.agent.clone(),
        })
        .collect::<Vec<_>>();
    if let Ok(json) = serde_json::to_string_pretty(&items) {
        let _ = fs::write(path, json);
    }
}

async fn restore_workspaces(state: &mut AppState, evt_tx: &broadcast::Sender<Event>) {
    let Some(path) = persist_file() else {
        return;
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return;
    };
    let Ok(items) = serde_json::from_str::<Vec<PersistedWorkspace>>(&raw) else {
        return;
    };
    for item in items {
        let repo_path = PathBuf::from(item.path);
        // Skip legacy migration duplicates: a bare workspace (no repository FK)
        // whose path is already a registered Repository. On first launch the
        // entry was imported as a Repository; without this guard it would also
        // load as an orphan Workspace and float to the bottom of the sidebar,
        // visually attaching to whichever repository is listed last.
        if item.repository_id.is_none()
            && state.repositories.values().any(|r| r.path == repo_path)
        {
            continue;
        }
        let id = Uuid::new_v4();
        let initial_git = refresh_git(&repo_path, item.ssh.as_ref())
            .await
            .unwrap_or_default();
        let ws = Workspace {
            id,
            name: item.name,
            path: repo_path,
            ssh: item.ssh,
            git: initial_git.clone(),
            attention: AttentionLevel::None,
            terminals: Default::default(),
            last_activity: Instant::now(),
            repository_id: item.repository_id,
            branch: item.branch.clone().or_else(|| initial_git.branch.clone()),
            base_branch: item.base_branch.clone(),
            agent: item.agent.clone(),
            ready_for_review: false,
            review_manual: false,
            agent_active: false,
            agent_idle: false,
            agent_input_suppress_until: None,
        };
        state.ordered_ids.push(id);
        state.workspaces.insert(id, ws);
        let _ = evt_tx.send(Event::WorkspaceGitUpdated {
            id,
            git: initial_git,
        });
    }
}

/// Best-effort path equality. Compares canonicalized forms when both resolve,
/// otherwise falls back to a raw comparison. Guards against double-adding a
/// worktree when git reports a symlink-resolved path that differs textually
/// from a stored Workspace path.
fn same_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    matches!((a.canonicalize(), b.canonicalize()), (Ok(ca), Ok(cb)) if ca == cb)
}

/// Discovers on-disk git worktrees for `repo_id` that are not yet tracked as
/// Workspaces and registers them under their owning Repository. The primary
/// worktree (the base repo itself) and worktrees already represented by a
/// Workspace are skipped. Returns the ids of any newly added Workspaces (empty
/// on error or when nothing new is found). Callers persist + emit events.
async fn pickup_worktrees_for_repo(state: &mut AppState, repo_id: RepositoryId) -> Vec<WorkspaceId> {
    let Some(repo) = state.repositories.get(&repo_id) else {
        return Vec::new();
    };
    let repo_path = repo.path.clone();
    let default_branch = repo.default_branch.clone();
    let ssh = repo.ssh.clone();

    let Ok(worktrees) = list_worktrees(&repo_path, ssh.as_ref()).await else {
        return Vec::new();
    };

    let mut added = Vec::new();
    for (wt_path, branch) in worktrees {
        // Skip the primary worktree (the base repo itself, never worked in).
        if same_path(&wt_path, &repo_path) {
            continue;
        }
        // Skip worktrees whose directory has gone away (local only; over SSH we
        // can't cheaply stat, so trust git's listing).
        if ssh.is_none() && !wt_path.exists() {
            continue;
        }
        // Skip worktrees already tracked as a Workspace.
        if state.workspaces.values().any(|w| same_path(&w.path, &wt_path)) {
            continue;
        }

        let id = Uuid::new_v4();
        let initial_git = refresh_git(&wt_path, ssh.as_ref()).await.unwrap_or_default();
        let branch = if branch.is_empty() {
            initial_git.branch.clone()
        } else {
            Some(branch)
        };
        let name = branch
            .clone()
            .or_else(|| {
                wt_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "worktree".to_string());

        let ws = Workspace {
            id,
            name,
            path: wt_path,
            ssh: ssh.clone(),
            git: initial_git,
            attention: AttentionLevel::None,
            terminals: Default::default(),
            last_activity: Instant::now(),
            repository_id: Some(repo_id),
            branch,
            base_branch: default_branch.clone(),
            agent: None,
            ready_for_review: false,
            review_manual: false,
            agent_active: false,
            agent_idle: false,
            agent_input_suppress_until: None,
        };
        state.ordered_ids.push(id);
        state.workspaces.insert(id, ws);
        added.push(id);
    }
    added
}

// ---------------------------------------------------------------------------
// Repository registry — global (above sessions), persisted to repositories.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedRepository {
    id: RepositoryId,
    name: String,
    path: String,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    worktree_root: Option<String>,
    #[serde(default)]
    default_agent: Option<String>,
    #[serde(default)]
    ssh: Option<SshTarget>,
}

/// XDG-aware config base (`$XDG_CONFIG_HOME` or `~/.config`) joined with
/// `conduit`. Used for machine-global files like the repository registry.
fn config_base() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.trim().is_empty() {
            return Some(PathBuf::from(xdg).join("conduit"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/conduit"))
}

/// Path to the machine-global repository registry. NOT session-scoped — base
/// repos are registered once and available in every session.
fn repositories_file() -> Option<PathBuf> {
    Some(config_base()?.join("repositories.json"))
}

fn repository_summaries(state: &AppState) -> Vec<RepositorySummary> {
    state
        .ordered_repo_ids
        .iter()
        .filter_map(|id| state.repositories.get(id))
        .map(|repo| {
            let workspaces: Vec<&Workspace> = state
                .workspaces
                .values()
                .filter(|w| w.repository_id == Some(repo.id))
                .collect();
            RepositorySummary {
                id: repo.id,
                name: repo.name.clone(),
                path: repo.path.display().to_string(),
                default_branch: repo.default_branch.clone(),
                worktree_root: repo.worktree_root.as_ref().map(|p| p.display().to_string()),
                default_agent: repo.default_agent.clone(),
                ssh_host: repo.ssh.as_ref().map(ssh::ssh_destination),
                workspace_count: workspaces.len(),
                ready_for_review_count: workspaces.iter().filter(|w| w.ready_for_review).count(),
            }
        })
        .collect()
}

/// Computes the on-disk worktree path for a new Workspace `slug` of `repo`.
/// Honors a per-repo `worktree_root` override; otherwise uses the default
/// `<repo_parent>/.conduit-worktrees/<repo_name>/<slug>` scheme (works locally
/// and over SSH since it is relative to wherever the repo lives).
fn worktree_path_for(repo: &Repository, slug: &str) -> PathBuf {
    match &repo.worktree_root {
        Some(root) => root.join(slug),
        None => {
            let parent = repo.path.parent().unwrap_or(repo.path.as_path());
            parent
                .join(".conduit-worktrees")
                .join(&repo.name)
                .join(slug)
        }
    }
}

/// Resolves `(worktree_path, ssh, base_branch)` for a workspace's branch diff /
/// PR. The base falls back: workspace `base_branch` → repo `default_branch` →
/// `"main"`.
fn workspace_base_for_diff(
    state: &AppState,
    id: Uuid,
) -> Option<(PathBuf, Option<SshTarget>, String)> {
    let ws = state.workspaces.get(&id)?;
    let base = ws
        .base_branch
        .clone()
        .or_else(|| {
            ws.repository_id
                .and_then(|rid| state.repositories.get(&rid))
                .and_then(|r| r.default_branch.clone())
        })
        .unwrap_or_else(|| "main".to_string());
    Some((ws.path.clone(), ws.ssh.clone(), base))
}

/// Builds a GitHub-style compare URL from an `origin` remote URL, supporting
/// `git@host:owner/repo(.git)`, `ssh://git@…`, and `http(s)://…` forms.
fn github_compare_url(origin: &str, base: &str, branch: &str) -> Option<String> {
    let s = origin.trim();
    let path = if let Some(rest) = s.strip_prefix("git@") {
        rest.replacen(':', "/", 1)
    } else if let Some(rest) = s.strip_prefix("ssh://git@") {
        rest.to_string()
    } else if let Some(rest) = s.strip_prefix("https://") {
        rest.to_string()
    } else if let Some(rest) = s.strip_prefix("http://") {
        rest.to_string()
    } else {
        return None;
    };
    let path = path.strip_suffix(".git").unwrap_or(&path);
    Some(format!("https://{path}/compare/{base}...{branch}?expand=1"))
}

fn save_repositories(state: &AppState) {
    let Some(path) = repositories_file() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let items = state
        .ordered_repo_ids
        .iter()
        .filter_map(|id| state.repositories.get(id))
        .map(|repo| PersistedRepository {
            id: repo.id,
            name: repo.name.clone(),
            path: repo.path.display().to_string(),
            default_branch: repo.default_branch.clone(),
            worktree_root: repo.worktree_root.as_ref().map(|p| p.display().to_string()),
            default_agent: repo.default_agent.clone(),
            ssh: repo.ssh.clone(),
        })
        .collect::<Vec<_>>();
    if let Ok(json) = serde_json::to_string_pretty(&items) {
        // Write-temp-then-rename: the registry is shared across session daemons,
        // so avoid torn files. Last-writer-wins is acceptable for one user.
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
    }
}

/// Loads the persisted repository registry. Returns `None` when the file does
/// not exist yet (first launch) so the caller can trigger migration.
fn load_repositories() -> Option<Vec<PersistedRepository>> {
    let path = repositories_file()?;
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Vec<PersistedRepository>>(&raw).ok()
}

/// Populates `state.repositories` from the persisted registry, or — on first
/// launch (no registry file) — migrates legacy `workspaces.json` entries into
/// Repositories.
async fn restore_repositories(state: &mut AppState) {
    if let Some(items) = load_repositories() {
        for item in items {
            let repo = Repository {
                id: item.id,
                name: item.name,
                path: PathBuf::from(item.path),
                default_branch: item.default_branch,
                worktree_root: item.worktree_root.map(PathBuf::from),
                default_agent: item.default_agent,
                ssh: item.ssh,
            };
            state.ordered_repo_ids.push(repo.id);
            state.repositories.insert(repo.id, repo);
        }
        return;
    }
    migrate_legacy_workspaces(state).await;
    save_repositories(state);
}

/// First-launch migration: each legacy `workspaces.json` entry that is a git
/// repository becomes a Repository. Non-git entries are skipped. No Workspaces
/// are created (source-only model — the user spawns worktree-Workspaces).
async fn migrate_legacy_workspaces(state: &mut AppState) {
    let Some(base) = config_base() else {
        return;
    };
    // Migrate from the default (non-session) legacy file.
    let legacy = base.join("workspaces.json");
    let Ok(raw) = fs::read_to_string(&legacy) else {
        return;
    };
    let Ok(items) = serde_json::from_str::<Vec<PersistedWorkspace>>(&raw) else {
        return;
    };
    for item in items {
        let path = PathBuf::from(&item.path);
        match repo_root(&path, item.ssh.as_ref()).await {
            Ok(root) => {
                let default_branch = detect_default_branch(&root, item.ssh.as_ref()).await.ok();
                let id = Uuid::new_v4();
                let repo = Repository {
                    id,
                    name: item.name,
                    path: root,
                    default_branch,
                    worktree_root: None,
                    default_agent: None,
                    ssh: item.ssh,
                };
                state.ordered_repo_ids.push(id);
                state.repositories.insert(id, repo);
            }
            Err(_) => {
                // Not a git repo — skip (the file stays on disk untouched).
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_command(argv: &[&str], cwd: &str) -> SavedCommand {
        SavedCommand {
            argv: argv.iter().map(|s| (*s).to_string()).collect(),
            cwd: cwd.to_string(),
        }
    }

    #[test]
    fn sanitize_alphanumeric_passthrough() {
        assert_eq!(sanitize_session_name("hello123"), "hello123");
    }

    #[test]
    fn sanitize_preserves_dashes_underscores() {
        assert_eq!(sanitize_session_name("my-session_1"), "my-session_1");
    }

    #[test]
    fn sanitize_spaces_become_underscores() {
        assert_eq!(sanitize_session_name("my session"), "my_session");
    }

    #[test]
    fn sanitize_special_chars() {
        assert_eq!(sanitize_session_name("a@b#c.d"), "a_b_c_d");
    }

    fn test_workspace() -> Workspace {
        Workspace {
            id: Uuid::new_v4(),
            name: "ws".into(),
            path: PathBuf::from("/tmp/ws"),
            ssh: None,
            git: GitState::default(),
            attention: AttentionLevel::None,
            terminals: Default::default(),
            last_activity: Instant::now(),
            repository_id: None,
            branch: None,
            base_branch: None,
            agent: None,
            ready_for_review: false,
            review_manual: false,
            agent_active: false,
            agent_idle: false,
            agent_input_suppress_until: None,
        }
    }

    #[test]
    fn compare_url_forms() {
        assert_eq!(
            github_compare_url("git@github.com:o/r.git", "main", "feat"),
            Some("https://github.com/o/r/compare/main...feat?expand=1".to_string())
        );
        assert_eq!(
            github_compare_url("https://github.com/o/r.git", "main", "feat"),
            Some("https://github.com/o/r/compare/main...feat?expand=1".to_string())
        );
        assert_eq!(
            github_compare_url("https://github.com/o/r", "dev", "x"),
            Some("https://github.com/o/r/compare/dev...x?expand=1".to_string())
        );
        assert_eq!(github_compare_url("file:///tmp/repo", "main", "x"), None);
    }

    #[test]
    fn review_heuristic_transitions() {
        let mut ws = test_workspace();

        // Idle but no changes -> not ready.
        ws.agent_idle = true;
        assert!(!recompute_review(&mut ws));
        assert!(!ws.ready_for_review);

        // Idle + dirty -> ready.
        ws.git.changed.push(protocol::ChangedFile {
            path: "a.rs".into(),
            index_status: 'M',
            worktree_status: ' ',
        });
        assert!(recompute_review(&mut ws));
        assert!(ws.ready_for_review);

        // Awaiting input -> not ready.
        ws.attention = AttentionLevel::NeedsInput;
        assert!(recompute_review(&mut ws));
        assert!(!ws.ready_for_review);

        // Ahead-of-upstream also counts as changes.
        ws.attention = AttentionLevel::None;
        ws.git.changed.clear();
        ws.git.ahead = Some(2);
        assert!(recompute_review(&mut ws));
        assert!(ws.ready_for_review);

        // Manual override suppresses the heuristic and sticks.
        ws.review_manual = true;
        ws.git.ahead = None;
        ws.git.changed.clear();
        assert!(!recompute_review(&mut ws));
        assert!(ws.ready_for_review);
    }

    #[test]
    fn agent_active_transition_sets_active_and_clears_review() {
        let mut ws = test_workspace();
        ws.agent_idle = true;
        ws.ready_for_review = true;
        ws.review_manual = true;

        let transition = apply_agent_active(&mut ws);

        assert!(ws.agent_active);
        assert!(!ws.agent_idle);
        assert!(!ws.ready_for_review);
        assert!(!ws.review_manual);
        assert!(transition.active_changed);
        assert!(transition.review_changed);
        assert!(!transition.ready);

        let transition = apply_agent_active(&mut ws);
        assert!(!transition.active_changed);
        assert!(!transition.review_changed);
    }

    #[test]
    fn agent_settled_clears_active_and_recomputes_review() {
        let mut ws = test_workspace();
        ws.agent_active = true;
        ws.git.changed.push(protocol::ChangedFile {
            path: "a.rs".into(),
            index_status: 'M',
            worktree_status: ' ',
        });

        let transition = apply_agent_settled(&mut ws);

        assert!(!ws.agent_active);
        assert!(ws.agent_idle);
        assert!(ws.ready_for_review);
        assert!(transition.active_changed);
        assert!(transition.review_changed);
        assert!(transition.ready);
    }

    #[test]
    fn input_submit_detection() {
        assert!(input_is_submit(b"\r"));
        assert!(input_is_submit(b"\n"));
        assert!(input_is_submit(b"hello\r"));
        assert!(!input_is_submit(b"hello"));
        assert!(!input_is_submit(b""));
        assert!(!input_is_submit(&[0x1b, b'[', b'A'])); // arrow-up escape sequence
    }

    #[test]
    fn typing_arms_grace_window_and_clears_spinner() {
        let mut ws = test_workspace();
        ws.agent_active = true;
        let now = Instant::now();

        let was_active = apply_agent_input(&mut ws, b"h", now);

        assert!(was_active, "spinner was visible, so it reports as cleared");
        assert!(!ws.agent_active, "typing clears the spinner");
        assert_eq!(
            ws.agent_input_suppress_until,
            Some(now + Duration::from_millis(USER_TYPING_GRACE_MS))
        );
        assert!(agent_input_suppressed(&ws, now));
    }

    #[test]
    fn submit_clears_grace_window() {
        let mut ws = test_workspace();
        ws.agent_input_suppress_until = Some(Instant::now() + Duration::from_secs(3));

        let was_active = apply_agent_input(&mut ws, b"\r", Instant::now());

        assert!(!was_active);
        assert!(
            ws.agent_input_suppress_until.is_none(),
            "Enter ends the typing-grace window immediately"
        );
    }

    #[test]
    fn empty_input_does_not_arm_window() {
        let mut ws = test_workspace();
        apply_agent_input(&mut ws, b"", Instant::now());
        assert!(ws.agent_input_suppress_until.is_none());
    }

    #[test]
    fn agent_active_ignored_during_grace_window() {
        let mut ws = test_workspace();
        ws.ready_for_review = true;
        ws.agent_input_suppress_until = Some(Instant::now() + Duration::from_secs(3));

        let transition = apply_agent_active(&mut ws);

        assert!(!ws.agent_active, "echoed activity must not light the spinner");
        assert!(ws.ready_for_review, "review state is left untouched");
        assert!(!transition.active_changed);
        assert!(!transition.review_changed);
    }

    #[test]
    fn agent_settled_ignored_during_grace_window() {
        let mut ws = test_workspace();
        ws.git.changed.push(protocol::ChangedFile {
            path: "a.rs".into(),
            index_status: 'M',
            worktree_status: ' ',
        });
        ws.agent_input_suppress_until = Some(Instant::now() + Duration::from_secs(3));

        let transition = apply_agent_settled(&mut ws);

        assert!(!ws.agent_idle, "echoed settle must not mark the agent idle");
        assert!(!ws.ready_for_review, "echoed settle must not flip review");
        assert!(!transition.review_changed);
        assert!(!transition.active_changed);
    }

    #[test]
    fn needs_input_suppressed_but_error_honoured_during_grace_window() {
        let mut ws = test_workspace();
        let now = Instant::now();
        ws.agent_input_suppress_until = Some(now + Duration::from_secs(3));

        assert!(should_suppress_attention(&ws, AttentionLevel::NeedsInput, now));
        assert!(!should_suppress_attention(&ws, AttentionLevel::Error, now));
        assert!(!should_suppress_attention(&ws, AttentionLevel::None, now));
    }

    #[test]
    fn attention_not_suppressed_outside_grace_window() {
        let ws = test_workspace(); // no window armed
        let now = Instant::now();
        assert!(!should_suppress_attention(&ws, AttentionLevel::NeedsInput, now));
    }

    #[test]
    fn sanitize_empty_returns_default() {
        assert_eq!(sanitize_session_name(""), "default");
    }

    #[test]
    fn sanitize_whitespace_only_returns_default() {
        assert_eq!(sanitize_session_name("   "), "default");
    }

    #[test]
    fn sanitize_trims_whitespace() {
        assert_eq!(sanitize_session_name("  hello  "), "hello");
    }

    #[test]
    fn startup_loaded_foreground_command_survives_fresh_idle_shell_until_clear() {
        let workspace = PathBuf::from("/tmp/conduit-test-workspace");
        let key = ForegroundCommandKey::new(&workspace, "shell");
        let command = saved_command(&["sleep", "300"], "/tmp/conduit-test-workspace");
        let mut store = ForegroundCommandStore::default();
        store.set_key(key.clone(), command.clone());
        let mut pending: HashSet<ForegroundCommandKey> = store.keys().collect();

        let observation = apply_foreground_observation(&mut store, &mut pending, &key, None);

        assert!(!observation.persisted_changed);
        assert!(!observation.cleared_pending);
        assert_eq!(store.get_key(&key), Some(&command));
        assert!(pending.contains(&key));

        assert!(clear_shell_resurrection_state(
            &mut store,
            &mut pending,
            &key
        ));
        assert!(store.get_key(&key).is_none());
        assert!(!pending.contains(&key));
    }

    #[test]
    fn foreground_observation_removes_non_pending_command_on_idle() {
        let workspace = PathBuf::from("/tmp/conduit-test-workspace");
        let key = ForegroundCommandKey::new(&workspace, "shell");
        let command = saved_command(&["cargo", "test"], "/tmp/conduit-test-workspace");
        let mut store = ForegroundCommandStore::default();
        let mut pending = HashSet::new();

        let observation =
            apply_foreground_observation(&mut store, &mut pending, &key, Some(command));
        assert!(observation.persisted_changed);
        assert!(!observation.cleared_pending);
        assert!(store.get_key(&key).is_some());

        let observation = apply_foreground_observation(&mut store, &mut pending, &key, None);
        assert!(observation.persisted_changed);
        assert!(!observation.cleared_pending);
        assert!(store.get_key(&key).is_none());
    }

    // ── normalize_tab_id tests ──────────────────────────────────────────

    #[test]
    fn normalize_tab_id_agent_none_returns_agent() {
        assert_eq!(
            normalize_tab_id(protocol::TerminalKind::Agent, None),
            "agent"
        );
    }

    #[test]
    fn normalize_tab_id_agent_some_custom_returns_agent() {
        assert_eq!(
            normalize_tab_id(protocol::TerminalKind::Agent, Some("custom".to_string())),
            "agent"
        );
    }

    #[test]
    fn normalize_tab_id_shell_some_returns_value() {
        assert_eq!(
            normalize_tab_id(protocol::TerminalKind::Shell, Some("my-tab".to_string())),
            "my-tab"
        );
    }

    #[test]
    fn normalize_tab_id_shell_none_returns_default() {
        assert_eq!(
            normalize_tab_id(protocol::TerminalKind::Shell, None),
            "shell"
        );
    }

    #[test]
    fn normalize_tab_id_shell_whitespace_only_returns_default() {
        assert_eq!(
            normalize_tab_id(protocol::TerminalKind::Shell, Some("  ".to_string())),
            "shell"
        );
    }

    #[test]
    fn normalize_tab_id_shell_empty_returns_default() {
        assert_eq!(
            normalize_tab_id(protocol::TerminalKind::Shell, Some("".to_string())),
            "shell"
        );
    }

    // ── default_terminal_cmd tests ──────────────────────────────────────

    #[test]
    fn default_terminal_cmd_agent_returns_two_elements() {
        let cmd = default_terminal_cmd(protocol::TerminalKind::Agent);
        assert_eq!(cmd.len(), 2);
    }

    #[test]
    fn default_terminal_cmd_shell_returns_two_elements() {
        let cmd = default_terminal_cmd(protocol::TerminalKind::Shell);
        assert_eq!(cmd.len(), 2);
    }

    #[test]
    fn default_terminal_cmd_agent_second_element_is_interactive_flag() {
        let cmd = default_terminal_cmd(protocol::TerminalKind::Agent);
        assert_eq!(cmd[1], "-i");
    }

    #[test]
    fn default_terminal_cmd_shell_second_element_is_interactive_flag() {
        let cmd = default_terminal_cmd(protocol::TerminalKind::Shell);
        assert_eq!(cmd[1], "-i");
    }

    #[test]
    fn default_terminal_cmd_agent_and_shell_return_same_command() {
        let agent_cmd = default_terminal_cmd(protocol::TerminalKind::Agent);
        let shell_cmd = default_terminal_cmd(protocol::TerminalKind::Shell);
        assert_eq!(agent_cmd, shell_cmd);
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
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

    /// Auto-pickup registers on-disk worktrees as Workspaces, skips the primary
    /// worktree (the base repo itself), and is idempotent (no duplicates).
    #[tokio::test]
    async fn pickup_worktrees_registers_and_dedupes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path();
        git(dir, &["init"]);
        git(dir, &["config", "user.email", "t@t.com"]);
        git(dir, &["config", "user.name", "T"]);
        std::fs::write(dir.join("readme.md"), "hi").unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-m", "init"]);

        // Two worktrees off HEAD, on new branches.
        let wt_a = dir.join(".conduit-worktrees").join("feat-a");
        let wt_b = dir.join(".conduit-worktrees").join("feat-b");
        create_worktree(dir, &wt_a, "feat-a", "HEAD", None)
            .await
            .unwrap();
        create_worktree(dir, &wt_b, "feat-b", "HEAD", None)
            .await
            .unwrap();

        let root = repo_root(dir, None).await.unwrap();
        let mut state = AppState::default();
        let repo_id = Uuid::new_v4();
        state.ordered_repo_ids.push(repo_id);
        state.repositories.insert(
            repo_id,
            Repository {
                id: repo_id,
                name: "demo".into(),
                path: root,
                default_branch: Some("main".into()),
                worktree_root: None,
                default_agent: None,
                ssh: None,
            },
        );

        let added = pickup_worktrees_for_repo(&mut state, repo_id).await;
        assert_eq!(added.len(), 2, "should pick up exactly the two worktrees");

        // Every new Workspace belongs to the repo; primary worktree is absent.
        for id in &added {
            let ws = state.workspaces.get(id).unwrap();
            assert_eq!(ws.repository_id, Some(repo_id));
            assert_eq!(ws.base_branch.as_deref(), Some("main"));
            assert!(!same_path(&ws.path, &state.repositories[&repo_id].path));
        }
        let mut branches: Vec<String> = added
            .iter()
            .filter_map(|id| state.workspaces[id].branch.clone())
            .collect();
        branches.sort();
        assert_eq!(branches, vec!["feat-a".to_string(), "feat-b".to_string()]);

        // Running again finds nothing new (idempotent — no duplicates).
        let again = pickup_worktrees_for_repo(&mut state, repo_id).await;
        assert!(again.is_empty(), "re-scan should add nothing: {again:?}");
        assert_eq!(state.workspaces.len(), 2);
    }
}
