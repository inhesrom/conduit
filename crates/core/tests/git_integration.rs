use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

use conduit_core::workspace::git::{
    commit, create_worktree, detect_default_branch, diff_branch_files, diff_file, list_worktrees,
    refresh_git, remove_worktree, repo_root, stage_file, unstage_file,
};

// ---------------------------------------------------------------------------
// Helper: initialise a throwaway git repo inside a TempDir
// ---------------------------------------------------------------------------

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

fn git_commit(dir: &Path, message: &str) {
    Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(dir)
        .output()
        .unwrap();
}

fn git_branch(dir: &Path, branch: &str) {
    Command::new("git")
        .args(["branch", branch])
        .current_dir(dir)
        .output()
        .unwrap();
}

// ===========================================================================
// C1 — refresh_git with a real temp repo
// ===========================================================================

#[tokio::test]
async fn refresh_git_clean_repo() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "hello.txt", "hello");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    let state = refresh_git(dir, None).await.unwrap();

    // Branch should be "main" or "master" depending on git config.
    let branch = state.branch.as_deref().unwrap();
    assert!(
        branch == "main" || branch == "master",
        "expected main or master, got {branch}"
    );

    // No uncommitted changes.
    assert!(state.changed.is_empty(), "expected no changed files");
}

#[tokio::test]
async fn refresh_git_dirty_worktree() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "hello.txt", "hello");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    // Modify a tracked file without staging.
    write_file(dir, "hello.txt", "hello world");

    let state = refresh_git(dir, None).await.unwrap();
    assert_eq!(state.changed.len(), 1, "expected 1 changed file");
    assert_eq!(state.changed[0].path, "hello.txt");
}

#[tokio::test]
async fn refresh_git_reports_untracked_directory_leaves() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "hello.txt", "hello");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    write_file(dir, "agent-skills/SKILL.md", "skill");
    write_file(dir, "agent-skills/docs/reference.md", "reference");

    let state = refresh_git(dir, None).await.unwrap();
    let changed: Vec<_> = state.changed.iter().map(|f| f.path.as_str()).collect();

    assert!(changed.contains(&"agent-skills/SKILL.md"));
    assert!(changed.contains(&"agent-skills/docs/reference.md"));
    assert!(!changed.contains(&"agent-skills/"));
}

#[tokio::test]
async fn refresh_git_shows_new_branch() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "hello.txt", "hello");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    git_branch(dir, "feature-x");

    let state = refresh_git(dir, None).await.unwrap();
    let branch_names: Vec<&str> = state
        .local_branches
        .iter()
        .map(|b| b.name.as_str())
        .collect();
    assert!(
        branch_names.contains(&"feature-x"),
        "expected feature-x in {branch_names:?}"
    );
}

// ===========================================================================
// C2 — stage_file / unstage_file / commit
// ===========================================================================

#[tokio::test]
async fn stage_and_unstage_file() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "a.txt", "aaa");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    // Modify the file.
    write_file(dir, "a.txt", "aaa modified");

    // Stage it.
    stage_file(dir, "a.txt", None).await.unwrap();
    let state = refresh_git(dir, None).await.unwrap();
    assert_eq!(state.changed.len(), 1);
    // After staging, the index_status should reflect the change (not ' ' and not '?').
    let f = &state.changed[0];
    assert_eq!(f.path, "a.txt");
    assert!(
        f.index_status != ' ' && f.index_status != '?',
        "expected staged index_status, got '{}'",
        f.index_status
    );

    // Unstage it.
    unstage_file(dir, "a.txt", None).await.unwrap();
    let state = refresh_git(dir, None).await.unwrap();
    assert_eq!(state.changed.len(), 1);
    let f = &state.changed[0];
    assert_eq!(f.path, "a.txt");
    // After unstaging the index_status should be clean (' ') and the worktree dirty.
    assert_eq!(
        f.index_status, ' ',
        "expected ' ' index_status after unstage"
    );
    assert_eq!(
        f.worktree_status, 'M',
        "expected 'M' worktree_status after unstage"
    );
}

#[tokio::test]
async fn stage_and_commit_clears_changes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "b.txt", "bbb");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    // Modify, stage, and commit via the library functions.
    write_file(dir, "b.txt", "bbb modified");
    stage_file(dir, "b.txt", None).await.unwrap();
    commit(dir, "second commit", None).await.unwrap();

    let state = refresh_git(dir, None).await.unwrap();
    assert!(
        state.changed.is_empty(),
        "expected no changed files after commit"
    );
    assert!(
        state
            .recent_commits
            .iter()
            .any(|c| c.message == "second commit"),
        "expected 'second commit' in recent_commits: {:?}",
        state.recent_commits
    );
}

// ===========================================================================
// C3 — diff_file for tracked, untracked, and clean files
// ===========================================================================

#[tokio::test]
async fn diff_file_modified_tracked() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "c.txt", "original");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    write_file(dir, "c.txt", "modified");

    let diff = diff_file(dir, "c.txt", None).await.unwrap();
    assert!(
        !diff.trim().is_empty(),
        "expected non-empty diff for modified tracked file"
    );
    assert!(
        diff.contains('+') || diff.contains('-'),
        "expected diff hunks"
    );
}

#[tokio::test]
async fn diff_file_untracked_new() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "tracked.txt", "tracked");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    // Create an untracked file.
    write_file(dir, "new.txt", "brand new content");

    let diff = diff_file(dir, "new.txt", None).await.unwrap();
    assert!(
        !diff.trim().is_empty(),
        "expected non-empty synthetic diff for untracked file"
    );
    assert!(
        diff.contains("+brand new content"),
        "expected '+' prefixed lines in synthetic diff, got:\n{diff}"
    );
}

#[tokio::test]
async fn diff_file_clean_committed() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "d.txt", "committed");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    // File is committed and unmodified — diff should be empty.
    let diff = diff_file(dir, "d.txt", None).await.unwrap();
    assert!(
        diff.trim().is_empty(),
        "expected empty diff for clean committed file, got:\n{diff}"
    );
}

// ===========================================================================
// Worktrees — create / list / branch-diff / remove
// ===========================================================================

#[tokio::test]
async fn worktree_create_list_diff_remove() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "hello.txt", "hello");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    let base = refresh_git(dir, None).await.unwrap().branch.unwrap();

    // Create a worktree on a new branch from current HEAD.
    let wt_path = dir.join(".conduit-worktrees").join("feature-x");
    create_worktree(dir, &wt_path, "feature-x", "HEAD", None)
        .await
        .unwrap();
    assert!(
        wt_path.join("hello.txt").exists(),
        "worktree should contain the committed file"
    );

    let worktrees = list_worktrees(dir, None).await.unwrap();
    assert!(
        worktrees
            .iter()
            .any(|(p, b)| p.ends_with("feature-x") && b == "feature-x"),
        "list_worktrees should include the new worktree: {worktrees:?}"
    );

    // Commit a change on the branch and verify whole-branch diff vs base.
    write_file(&wt_path, "new.txt", "added");
    git_add_all(&wt_path);
    git_commit(&wt_path, "add new file");
    let changed = diff_branch_files(&wt_path, &base, None).await.unwrap();
    assert!(
        changed.iter().any(|c| c.path == "new.txt"),
        "branch diff should list new.txt: {changed:?}"
    );

    // Remove the worktree.
    remove_worktree(dir, &wt_path, None).await.unwrap();
    let after = list_worktrees(dir, None).await.unwrap();
    assert!(
        !after.iter().any(|(p, _)| p.ends_with("feature-x")),
        "worktree should be gone after remove: {after:?}"
    );
}

#[tokio::test]
async fn repo_root_and_default_branch() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    git_init(dir);
    write_file(dir, "f.txt", "x");
    git_add_all(dir);
    git_commit(dir, "initial commit");

    // repo_root resolves from a subdirectory back to the toplevel.
    let sub = dir.join("subdir");
    std::fs::create_dir_all(&sub).unwrap();
    let root = repo_root(&sub, None).await.unwrap();
    assert_eq!(
        root.canonicalize().unwrap(),
        dir.canonicalize().unwrap(),
        "repo_root should resolve to the repo toplevel"
    );

    // No remote configured -> falls back to "main".
    let default = detect_default_branch(dir, None).await.unwrap();
    assert_eq!(default, "main");
}
