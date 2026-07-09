use std::{path::Path, process::Output};

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use protocol::{
    PullRequestChecksSummary, PullRequestComment, PullRequestCommentKind, PullRequestCommentTarget,
    PullRequestDetails, PullRequestDiffLine, PullRequestDiffLineKind, PullRequestDiffSide,
    PullRequestFile, PullRequestProvider, PullRequestRef, PullRequestSummary, SshTarget,
};
use serde_json::Value;

use super::ssh;

const PR_VIEW_FIELDS: &str = "additions,author,baseRefName,body,changedFiles,deletions,headRefName,headRefOid,isDraft,labels,mergeStateStatus,mergeable,number,reviewDecision,reviewRequests,state,statusCheckRollup,title,url";
const PR_LIST_FIELDS: &str = "number,title,baseRefName,headRefName,url";

const REAL_GH: RealGhRunner = RealGhRunner;

#[derive(Debug)]
pub enum PullRequestLoadResult {
    /// A single pull request was resolved and fully loaded.
    Loaded(PullRequestDetails),
    /// Multiple repository-level PRs are available and need user selection.
    Candidates(Vec<PullRequestSummary>),
}

#[derive(Debug)]
pub enum PullRequestError {
    SetupRequired(String),
    NotFound(String),
    Other(anyhow::Error),
}

impl PullRequestError {
    pub fn message(&self) -> String {
        match self {
            Self::SetupRequired(message) | Self::NotFound(message) => message.clone(),
            Self::Other(err) => err.to_string(),
        }
    }
}

impl From<anyhow::Error> for PullRequestError {
    fn from(value: anyhow::Error) -> Self {
        Self::Other(value)
    }
}

trait GhRunner {
    fn output<'a>(
        &'a self,
        repo: &'a Path,
        ssh: Option<&'a SshTarget>,
        args: Vec<String>,
    ) -> BoxFuture<'a, std::io::Result<Output>>;
}

struct RealGhRunner;

impl GhRunner for RealGhRunner {
    fn output<'a>(
        &'a self,
        repo: &'a Path,
        ssh: Option<&'a SshTarget>,
        args: Vec<String>,
    ) -> BoxFuture<'a, std::io::Result<Output>> {
        Box::pin(async move {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            ssh::build_command(ssh, repo, "gh", &refs).output().await
        })
    }
}

pub async fn load_pull_request(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: Option<&PullRequestRef>,
    query: Option<&str>,
) -> Result<PullRequestLoadResult, PullRequestError> {
    load_pull_request_with_runner(repo, ssh, pr, query, &REAL_GH).await
}

async fn load_pull_request_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: Option<&PullRequestRef>,
    query: Option<&str>,
    runner: &R,
) -> Result<PullRequestLoadResult, PullRequestError> {
    ensure_gh_with_runner(repo, ssh, runner).await?;

    let query = query.map(str::trim).filter(|q| !q.is_empty());
    let explicit_lookup = pr.is_some() || query.is_some();
    match load_pull_request_details_with_runner(repo, ssh, pr, query, runner).await {
        Ok(details) => Ok(PullRequestLoadResult::Loaded(details)),
        Err(PullRequestError::NotFound(_)) if !explicit_lookup => {
            let candidates = list_open_pull_requests_with_runner(repo, ssh, runner).await?;
            match candidates.as_slice() {
                [] => Err(PullRequestError::NotFound(default_not_found_message())),
                [candidate] => {
                    let details = load_pull_request_details_with_runner(
                        repo,
                        ssh,
                        Some(&candidate.pr),
                        None,
                        runner,
                    )
                    .await?;
                    Ok(PullRequestLoadResult::Loaded(details))
                }
                _ => Ok(PullRequestLoadResult::Candidates(candidates)),
            }
        }
        Err(err) => Err(err),
    }
}

async fn load_pull_request_details_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: Option<&PullRequestRef>,
    query: Option<&str>,
    runner: &R,
) -> Result<PullRequestDetails, PullRequestError> {
    let view = gh_pr_view_with_runner(repo, ssh, pr, query, runner).await?;
    let pr_ref = pr
        .cloned()
        .or_else(|| pr_ref_from_view(&view))
        .ok_or_else(|| {
            PullRequestError::Other(anyhow!(
                "gh pr view did not return a parseable pull request URL"
            ))
        })?;
    let current_user = current_user_login_with_runner(repo, ssh, &pr_ref.host, runner)
        .await
        .ok();

    let files = load_pr_files_with_runner(repo, ssh, &pr_ref, runner).await?;
    let mut comments =
        load_issue_comments_with_runner(repo, ssh, &pr_ref, current_user.as_deref(), runner)
            .await?;
    comments.extend(
        load_review_comments_with_runner(repo, ssh, &pr_ref, current_user.as_deref(), runner)
            .await?,
    );
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let diff = load_pr_diff_lines_with_runner(repo, ssh, &pr_ref, runner).await?;

    Ok(details_from_view(view, pr_ref, files, comments, diff))
}

pub async fn update_pull_request(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<String, PullRequestError> {
    ensure_gh(repo, ssh).await?;
    if title.map(str::trim).filter(|s| !s.is_empty()).is_none() && body.is_none() {
        return Ok("No pull request changes to save".to_string());
    }

    let mut args = gh_api_prefix(
        pr,
        "PATCH",
        &format!("repos/{}/{}/pulls/{}", pr.owner, pr.repo, pr.number),
    );
    if let Some(title) = title.map(str::trim).filter(|s| !s.is_empty()) {
        args.push("-f".to_string());
        args.push(format!("title={title}"));
    }
    if let Some(body) = body {
        args.push("-f".to_string());
        args.push(format!("body={body}"));
    }
    gh_success(repo, ssh, args).await?;
    Ok("Pull request updated".to_string())
}

pub async fn create_issue_comment(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    body: &str,
) -> Result<String, PullRequestError> {
    ensure_gh(repo, ssh).await?;
    let mut args = gh_api_prefix(
        pr,
        "POST",
        &format!(
            "repos/{}/{}/issues/{}/comments",
            pr.owner, pr.repo, pr.number
        ),
    );
    args.push("-f".to_string());
    args.push(format!("body={}", body.trim()));
    gh_success(repo, ssh, args).await?;
    Ok("Comment posted".to_string())
}

pub async fn create_inline_comment(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    path: &str,
    body: &str,
    line: u32,
    side: PullRequestDiffSide,
    start_line: Option<u32>,
    start_side: Option<PullRequestDiffSide>,
) -> Result<String, PullRequestError> {
    ensure_gh(repo, ssh).await?;
    let head_sha = pr_head_sha(repo, ssh, pr).await?;
    let diff = load_pr_diff_lines(repo, ssh, pr).await?;
    if !diff_contains_comment_target(&diff, path, line, side, start_line, start_side) {
        return Err(PullRequestError::Other(anyhow!(
            "Cannot comment on {path}:{line}; that line is not present in the current GitHub PR diff"
        )));
    }

    let mut args = gh_api_prefix(
        pr,
        "POST",
        &format!(
            "repos/{}/{}/pulls/{}/comments",
            pr.owner, pr.repo, pr.number
        ),
    );
    args.extend([
        "-f".to_string(),
        format!("body={}", body.trim()),
        "-f".to_string(),
        format!("commit_id={head_sha}"),
        "-f".to_string(),
        format!("path={path}"),
        "-F".to_string(),
        format!("line={line}"),
        "-f".to_string(),
        format!("side={}", github_side(side)),
    ]);
    if let Some(start_line) = start_line {
        args.push("-F".to_string());
        args.push(format!("start_line={start_line}"));
        args.push("-f".to_string());
        args.push(format!(
            "start_side={}",
            github_side(start_side.unwrap_or(side))
        ));
    }
    gh_success(repo, ssh, args).await?;
    Ok("Inline comment posted".to_string())
}

pub async fn reply_to_review_comment(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    comment_id: &str,
    body: &str,
) -> Result<String, PullRequestError> {
    ensure_gh(repo, ssh).await?;
    let mut args = gh_api_prefix(
        pr,
        "POST",
        &format!(
            "repos/{}/{}/pulls/{}/comments/{}/replies",
            pr.owner, pr.repo, pr.number, comment_id
        ),
    );
    args.push("-f".to_string());
    args.push(format!("body={}", body.trim()));
    gh_success(repo, ssh, args).await?;
    Ok("Reply posted".to_string())
}

pub async fn edit_comment(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    target: &PullRequestCommentTarget,
    body: &str,
) -> Result<String, PullRequestError> {
    ensure_gh(repo, ssh).await?;
    ensure_comment_owned_by_current_user(repo, ssh, pr, target).await?;
    let mut args = gh_api_prefix(pr, "PATCH", &comment_endpoint(pr, target));
    args.push("-f".to_string());
    args.push(format!("body={}", body.trim()));
    gh_success(repo, ssh, args).await?;
    Ok("Comment updated".to_string())
}

pub async fn delete_comment(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    target: &PullRequestCommentTarget,
) -> Result<String, PullRequestError> {
    ensure_gh(repo, ssh).await?;
    ensure_comment_owned_by_current_user(repo, ssh, pr, target).await?;
    let args = gh_api_prefix(pr, "DELETE", &comment_endpoint(pr, target));
    gh_success(repo, ssh, args).await?;
    Ok("Comment deleted".to_string())
}

async fn ensure_comment_owned_by_current_user(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    target: &PullRequestCommentTarget,
) -> Result<(), PullRequestError> {
    let current = current_user_login(repo, ssh, &pr.host)
        .await
        .map_err(|_| PullRequestError::Other(anyhow!("Could not determine current GitHub user")))?;
    let value = gh_json(
        repo,
        ssh,
        gh_api_get_prefix(pr, &comment_endpoint(pr, target)),
    )
    .await?;
    let author = user_login(&value).unwrap_or_default();
    if author == current {
        Ok(())
    } else {
        Err(PullRequestError::Other(anyhow!(
            "Only your own comments can be edited or deleted"
        )))
    }
}

fn comment_endpoint(pr: &PullRequestRef, target: &PullRequestCommentTarget) -> String {
    match target.kind {
        PullRequestCommentKind::Issue => {
            format!(
                "repos/{}/{}/issues/comments/{}",
                pr.owner, pr.repo, target.id
            )
        }
        PullRequestCommentKind::Review => {
            format!(
                "repos/{}/{}/pulls/comments/{}",
                pr.owner, pr.repo, target.id
            )
        }
    }
}

async fn ensure_gh(repo: &Path, ssh: Option<&SshTarget>) -> Result<(), PullRequestError> {
    ensure_gh_with_runner(repo, ssh, &REAL_GH).await
}

async fn ensure_gh_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    runner: &R,
) -> Result<(), PullRequestError> {
    let out = runner
        .output(repo, ssh, vec!["--version".to_string()])
        .await;
    match out {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(PullRequestError::SetupRequired(gh_setup_message(
            ssh,
            &String::from_utf8_lossy(&out.stderr),
        ))),
        Err(err) => Err(PullRequestError::SetupRequired(gh_setup_message(
            ssh,
            &err.to_string(),
        ))),
    }
}

async fn gh_pr_view_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: Option<&PullRequestRef>,
    query: Option<&str>,
    runner: &R,
) -> Result<Value, PullRequestError> {
    let mut args = vec!["pr".to_string(), "view".to_string()];
    if let Some(pr) = pr {
        args.push(pr.number.to_string());
        args.push("--repo".to_string());
        args.push(repo_selector(pr));
    } else if let Some(query) = query.map(str::trim).filter(|q| !q.is_empty()) {
        args.push(query.to_string());
    }
    args.push("--json".to_string());
    args.push(PR_VIEW_FIELDS.to_string());
    gh_json_with_runner(repo, ssh, args, runner).await
}

async fn list_open_pull_requests_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    runner: &R,
) -> Result<Vec<PullRequestSummary>, PullRequestError> {
    let value = gh_json_with_runner(
        repo,
        ssh,
        vec![
            "pr".to_string(),
            "list".to_string(),
            "--state".to_string(),
            "open".to_string(),
            "--json".to_string(),
            PR_LIST_FIELDS.to_string(),
            "--limit".to_string(),
            "50".to_string(),
        ],
        runner,
    )
    .await?;
    let items = value.as_array().ok_or_else(|| {
        PullRequestError::Other(anyhow!("gh pr list did not return a JSON array"))
    })?;
    Ok(items.iter().filter_map(pr_summary_from_list_item).collect())
}

async fn pr_head_sha(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
) -> Result<String, PullRequestError> {
    let mut args = vec![
        "pr".to_string(),
        "view".to_string(),
        pr.number.to_string(),
        "--repo".to_string(),
        repo_selector(pr),
        "--json".to_string(),
        "headRefOid".to_string(),
    ];
    let value = gh_json(repo, ssh, std::mem::take(&mut args)).await?;
    json_string(&value, "headRefOid").ok_or_else(|| {
        PullRequestError::Other(anyhow!(
            "gh pr view did not return a head commit for PR #{}",
            pr.number
        ))
    })
}

async fn load_pr_files_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    runner: &R,
) -> Result<Vec<PullRequestFile>, PullRequestError> {
    let value = gh_json_with_runner(
        repo,
        ssh,
        gh_api_paginated_prefix(
            pr,
            &format!(
                "repos/{}/{}/pulls/{}/files?per_page=100",
                pr.owner, pr.repo, pr.number
            ),
        ),
        runner,
    )
    .await?;
    Ok(flatten_paginated_array(&value)
        .into_iter()
        .map(|item| PullRequestFile {
            path: json_string(&item, "filename").unwrap_or_default(),
            status: json_string(&item, "status").unwrap_or_default(),
            additions: json_u32(&item, "additions"),
            deletions: json_u32(&item, "deletions"),
            changes: json_u32(&item, "changes"),
            patch: json_string(&item, "patch"),
        })
        .filter(|file| !file.path.is_empty())
        .collect())
}

async fn load_issue_comments_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    current_user: Option<&str>,
    runner: &R,
) -> Result<Vec<PullRequestComment>, PullRequestError> {
    let value = gh_json_with_runner(
        repo,
        ssh,
        gh_api_paginated_prefix(
            pr,
            &format!(
                "repos/{}/{}/issues/{}/comments?per_page=100",
                pr.owner, pr.repo, pr.number
            ),
        ),
        runner,
    )
    .await?;
    Ok(flatten_paginated_array(&value)
        .into_iter()
        .filter_map(|item| comment_from_issue_json(&item, current_user))
        .collect())
}

async fn load_review_comments_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    current_user: Option<&str>,
    runner: &R,
) -> Result<Vec<PullRequestComment>, PullRequestError> {
    let value = gh_json_with_runner(
        repo,
        ssh,
        gh_api_paginated_prefix(
            pr,
            &format!(
                "repos/{}/{}/pulls/{}/comments?per_page=100",
                pr.owner, pr.repo, pr.number
            ),
        ),
        runner,
    )
    .await?;
    Ok(flatten_paginated_array(&value)
        .into_iter()
        .filter_map(|item| comment_from_review_json(&item, current_user))
        .collect())
}

async fn load_pr_diff_lines(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
) -> Result<Vec<PullRequestDiffLine>, PullRequestError> {
    load_pr_diff_lines_with_runner(repo, ssh, pr, &REAL_GH).await
}

async fn load_pr_diff_lines_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    pr: &PullRequestRef,
    runner: &R,
) -> Result<Vec<PullRequestDiffLine>, PullRequestError> {
    let mut args = gh_api_get_prefix(
        pr,
        &format!("repos/{}/{}/pulls/{}", pr.owner, pr.repo, pr.number),
    );
    args.push("-H".to_string());
    args.push("Accept: application/vnd.github.diff".to_string());
    let out = gh_success_with_runner(repo, ssh, args, runner).await?;
    Ok(parse_pr_diff(&String::from_utf8_lossy(&out.stdout)))
}

async fn current_user_login(
    repo: &Path,
    ssh: Option<&SshTarget>,
    host: &str,
) -> Result<String, PullRequestError> {
    current_user_login_with_runner(repo, ssh, host, &REAL_GH).await
}

async fn current_user_login_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    host: &str,
    runner: &R,
) -> Result<String, PullRequestError> {
    let host = host.trim();
    let pr = PullRequestRef {
        provider: PullRequestProvider::GitHub,
        host: if host.is_empty() {
            "github.com".to_string()
        } else {
            host.to_string()
        },
        owner: String::new(),
        repo: String::new(),
        number: 0,
    };
    let value = gh_json_with_runner(repo, ssh, gh_api_get_prefix(&pr, "user"), runner).await?;
    json_string(&value, "login")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| PullRequestError::Other(anyhow!("gh api user did not return login")))
}

async fn gh_json(
    repo: &Path,
    ssh: Option<&SshTarget>,
    args: Vec<String>,
) -> Result<Value, PullRequestError> {
    gh_json_with_runner(repo, ssh, args, &REAL_GH).await
}

async fn gh_json_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    args: Vec<String>,
    runner: &R,
) -> Result<Value, PullRequestError> {
    let out = gh_success_with_runner(repo, ssh, args, runner).await?;
    serde_json::from_slice(&out.stdout)
        .map_err(|err| PullRequestError::Other(anyhow!("failed to parse gh JSON: {err}")))
}

async fn gh_success(
    repo: &Path,
    ssh: Option<&SshTarget>,
    args: Vec<String>,
) -> Result<std::process::Output, PullRequestError> {
    gh_success_with_runner(repo, ssh, args, &REAL_GH).await
}

async fn gh_success_with_runner<R: GhRunner + Sync>(
    repo: &Path,
    ssh: Option<&SshTarget>,
    args: Vec<String>,
    runner: &R,
) -> Result<Output, PullRequestError> {
    let out = runner
        .output(repo, ssh, args)
        .await
        .map_err(|err| PullRequestError::SetupRequired(gh_setup_message(ssh, &err.to_string())))?;
    if out.status.success() {
        return Ok(out);
    }
    Err(map_gh_failure(ssh, &out))
}

fn gh_api_get_prefix(pr: &PullRequestRef, endpoint: &str) -> Vec<String> {
    let mut args = vec!["api".to_string(), "--hostname".to_string(), pr.host.clone()];
    args.push(endpoint.to_string());
    args
}

fn gh_api_paginated_prefix(pr: &PullRequestRef, endpoint: &str) -> Vec<String> {
    let mut args = gh_api_get_prefix(pr, endpoint);
    args.push("--paginate".to_string());
    args.push("--slurp".to_string());
    args
}

fn gh_api_prefix(pr: &PullRequestRef, method: &str, endpoint: &str) -> Vec<String> {
    let mut args = gh_api_get_prefix(pr, endpoint);
    args.push("-X".to_string());
    args.push(method.to_string());
    args
}

fn gh_setup_message(ssh: Option<&SshTarget>, detail: &str) -> String {
    let detail = detail.trim();
    let suffix = if detail.is_empty() {
        String::new()
    } else {
        format!(" Details: {detail}")
    };
    match ssh {
        Some(target) => format!(
            "GitHub CLI is not ready on SSH host {}. Install `gh` and run `gh auth login` on that host, then retry.{suffix}",
            ssh::ssh_destination(target)
        ),
        None => format!(
            "GitHub CLI is not ready on this machine. Install `gh` and run `gh auth login`, then retry.{suffix}"
        ),
    }
}

fn map_gh_failure(ssh: Option<&SshTarget>, out: &std::process::Output) -> PullRequestError {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stderr}\n{stdout}");
    let lower = combined.to_lowercase();
    if lower.contains("command not found")
        || lower.contains("executable file not found")
        || lower.contains("no such file or directory")
        || lower.contains("gh: not found")
    {
        return PullRequestError::SetupRequired(gh_setup_message(ssh, &combined));
    }
    if lower.contains("auth")
        || lower.contains("not logged")
        || lower.contains("login")
        || lower.contains("oauth")
        || lower.contains("http 401")
        || lower.contains("requires authentication")
    {
        return PullRequestError::SetupRequired(gh_setup_message(ssh, &combined));
    }
    if lower.contains("no pull request")
        || lower.contains("no pull requests")
        || lower.contains("could not find")
        || lower.contains("not found")
    {
        return PullRequestError::NotFound(default_not_found_message());
    }
    PullRequestError::Other(anyhow!(
        "gh failed: {}",
        combined.trim().if_empty("unknown error")
    ))
}

fn default_not_found_message() -> String {
    "No GitHub pull request was found for this Workspace. Open a PR or enter a PR number/URL."
        .to_string()
}

fn pr_summary_from_list_item(value: &Value) -> Option<PullRequestSummary> {
    let url = json_string(value, "url")?;
    Some(PullRequestSummary {
        pr: parse_github_pr_url(&url)?,
        title: json_string(value, "title").unwrap_or_default(),
        url,
        state: json_string(value, "state").unwrap_or_else(|| "OPEN".to_string()),
        base_ref: json_string(value, "baseRefName").unwrap_or_default(),
        head_ref: json_string(value, "headRefName").unwrap_or_default(),
    })
}

fn details_from_view(
    view: Value,
    pr: PullRequestRef,
    files: Vec<PullRequestFile>,
    comments: Vec<PullRequestComment>,
    diff: Vec<PullRequestDiffLine>,
) -> PullRequestDetails {
    PullRequestDetails {
        title: json_string(&view, "title").unwrap_or_default(),
        body: json_string(&view, "body").unwrap_or_default(),
        author: view.get("author").and_then(user_login).unwrap_or_default(),
        state: json_string(&view, "state").unwrap_or_default(),
        url: json_string(&view, "url").unwrap_or_default(),
        base_ref: json_string(&view, "baseRefName").unwrap_or_default(),
        head_ref: json_string(&view, "headRefName").unwrap_or_default(),
        head_sha: json_string(&view, "headRefOid").unwrap_or_default(),
        is_draft: json_bool(&view, "isDraft"),
        labels: json_nodes(&view, "labels")
            .into_iter()
            .filter_map(|label| json_string(&label, "name"))
            .collect(),
        reviewers: review_requests(&view),
        review_decision: json_string(&view, "reviewDecision"),
        mergeable: json_string(&view, "mergeable"),
        merge_state_status: json_string(&view, "mergeStateStatus"),
        additions: json_u32(&view, "additions"),
        deletions: json_u32(&view, "deletions"),
        changed_files: json_u32(&view, "changedFiles"),
        checks: checks_summary(view.get("statusCheckRollup")),
        files,
        comments,
        diff,
        pr,
    }
}

fn pr_ref_from_view(view: &Value) -> Option<PullRequestRef> {
    parse_github_pr_url(&json_string(view, "url")?)
}

pub fn parse_github_pr_url(url: &str) -> Option<PullRequestRef> {
    let rest = url
        .trim()
        .strip_prefix("https://")
        .or_else(|| url.trim().strip_prefix("http://"))?;
    let mut parts = rest.split('/');
    let host = parts.next()?.to_string();
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches(".git").to_string();
    if parts.next()? != "pull" {
        return None;
    }
    let number = parts.next()?.parse::<u32>().ok()?;
    Some(PullRequestRef {
        provider: PullRequestProvider::GitHub,
        host,
        owner,
        repo,
        number,
    })
}

fn repo_selector(pr: &PullRequestRef) -> String {
    if pr.host == "github.com" {
        format!("{}/{}", pr.owner, pr.repo)
    } else {
        format!("{}/{}/{}", pr.host, pr.owner, pr.repo)
    }
}

fn comment_from_issue_json(
    value: &Value,
    current_user: Option<&str>,
) -> Option<PullRequestComment> {
    let id = json_id(value, "id")?;
    let author = value.get("user").and_then(user_login).unwrap_or_default();
    let own = current_user.map(|u| u == author).unwrap_or(false);
    Some(PullRequestComment {
        target: PullRequestCommentTarget {
            kind: PullRequestCommentKind::Issue,
            id,
        },
        author,
        body: json_string(value, "body").unwrap_or_default(),
        created_at: json_string(value, "created_at").unwrap_or_default(),
        updated_at: json_string(value, "updated_at").unwrap_or_default(),
        html_url: json_string(value, "html_url").unwrap_or_default(),
        path: None,
        diff_hunk: None,
        line: None,
        side: None,
        start_line: None,
        start_side: None,
        in_reply_to_id: None,
        can_edit: own,
        can_delete: own,
    })
}

fn comment_from_review_json(
    value: &Value,
    current_user: Option<&str>,
) -> Option<PullRequestComment> {
    let id = json_id(value, "id")?;
    let author = value.get("user").and_then(user_login).unwrap_or_default();
    let own = current_user.map(|u| u == author).unwrap_or(false);
    Some(PullRequestComment {
        target: PullRequestCommentTarget {
            kind: PullRequestCommentKind::Review,
            id,
        },
        author,
        body: json_string(value, "body").unwrap_or_default(),
        created_at: json_string(value, "created_at").unwrap_or_default(),
        updated_at: json_string(value, "updated_at").unwrap_or_default(),
        html_url: json_string(value, "html_url").unwrap_or_default(),
        path: json_string(value, "path"),
        diff_hunk: json_string(value, "diff_hunk"),
        line: json_u32_opt(value, "line").or_else(|| json_u32_opt(value, "original_line")),
        side: json_string(value, "side").and_then(|side| parse_side(&side)),
        start_line: json_u32_opt(value, "start_line")
            .or_else(|| json_u32_opt(value, "original_start_line")),
        start_side: json_string(value, "start_side").and_then(|side| parse_side(&side)),
        in_reply_to_id: json_id(value, "in_reply_to_id"),
        can_edit: own,
        can_delete: own,
    })
}

fn review_requests(value: &Value) -> Vec<String> {
    json_nodes(value, "reviewRequests")
        .into_iter()
        .filter_map(|request| {
            request
                .get("requestedReviewer")
                .and_then(user_login)
                .or_else(|| user_login(&request))
        })
        .collect()
}

fn checks_summary(value: Option<&Value>) -> PullRequestChecksSummary {
    fn visit(value: &Value, summary: &mut PullRequestChecksSummary) {
        match value {
            Value::Array(items) => {
                for item in items {
                    visit(item, summary);
                }
            }
            Value::Object(map) => {
                if let Some(nodes) = map.get("nodes") {
                    visit(nodes, summary);
                    return;
                }
                let status = map
                    .get("conclusion")
                    .or_else(|| map.get("status"))
                    .and_then(Value::as_str);
                if let Some(status) = status {
                    summary.total += 1;
                    match status {
                        "SUCCESS" | "NEUTRAL" | "COMPLETED" => summary.passed += 1,
                        "SKIPPED" => summary.skipped += 1,
                        "FAILURE" | "ERROR" | "TIMED_OUT" | "ACTION_REQUIRED" | "CANCELLED" => {
                            summary.failed += 1
                        }
                        _ => summary.pending += 1,
                    }
                } else {
                    for child in map.values() {
                        visit(child, summary);
                    }
                }
            }
            _ => {}
        }
    }

    let mut summary = PullRequestChecksSummary::default();
    if let Some(value) = value {
        visit(value, &mut summary);
    }
    summary
}

pub fn parse_pr_diff(diff: &str) -> Vec<PullRequestDiffLine> {
    let mut out = Vec::new();
    let mut path = String::new();
    let mut old_path = String::new();
    let mut old_line = 0u32;
    let mut new_line = 0u32;
    let mut in_hunk = false;

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            in_hunk = false;
            path.clear();
            old_path.clear();
            continue;
        }
        if let Some(prev) = line.strip_prefix("--- ") {
            old_path = strip_diff_path(prev).to_string();
            continue;
        }
        if let Some(next) = line.strip_prefix("+++ ") {
            path = if next == "/dev/null" {
                old_path.clone()
            } else {
                strip_diff_path(next).to_string()
            };
            continue;
        }
        if let Some((old_start, new_start)) = parse_hunk_header(line) {
            old_line = old_start;
            new_line = new_start;
            in_hunk = true;
            out.push(PullRequestDiffLine {
                path: path.clone(),
                kind: PullRequestDiffLineKind::Hunk,
                text: line.to_string(),
                old_line: None,
                new_line: None,
                side: None,
            });
            continue;
        }
        if !in_hunk || path.is_empty() {
            continue;
        }
        if let Some(text) = line.strip_prefix('+') {
            out.push(PullRequestDiffLine {
                path: path.clone(),
                kind: PullRequestDiffLineKind::Add,
                text: text.to_string(),
                old_line: None,
                new_line: Some(new_line),
                side: Some(PullRequestDiffSide::Right),
            });
            new_line += 1;
        } else if let Some(text) = line.strip_prefix('-') {
            out.push(PullRequestDiffLine {
                path: path.clone(),
                kind: PullRequestDiffLineKind::Delete,
                text: text.to_string(),
                old_line: Some(old_line),
                new_line: None,
                side: Some(PullRequestDiffSide::Left),
            });
            old_line += 1;
        } else if line.starts_with('\\') {
            out.push(PullRequestDiffLine {
                path: path.clone(),
                kind: PullRequestDiffLineKind::Meta,
                text: line.to_string(),
                old_line: None,
                new_line: None,
                side: None,
            });
        } else {
            let text = line.strip_prefix(' ').unwrap_or(line);
            out.push(PullRequestDiffLine {
                path: path.clone(),
                kind: PullRequestDiffLineKind::Context,
                text: text.to_string(),
                old_line: Some(old_line),
                new_line: Some(new_line),
                side: Some(PullRequestDiffSide::Right),
            });
            old_line += 1;
            new_line += 1;
        }
    }
    out
}

fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    let body = line.strip_prefix("@@ -")?;
    let (old, rest) = body.split_once(' ')?;
    let new = rest.strip_prefix('+')?.split_once(' ')?.0;
    let old_start = old.split(',').next()?.parse().ok()?;
    let new_start = new.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

fn strip_diff_path(path: &str) -> &str {
    if path == "/dev/null" {
        path
    } else {
        path.strip_prefix("b/")
            .or_else(|| path.strip_prefix("a/"))
            .unwrap_or(path)
    }
}

pub fn diff_contains_comment_target(
    diff: &[PullRequestDiffLine],
    path: &str,
    line: u32,
    side: PullRequestDiffSide,
    start_line: Option<u32>,
    start_side: Option<PullRequestDiffSide>,
) -> bool {
    let line_ok = diff
        .iter()
        .any(|item| diff_line_matches(item, path, line, side));
    if !line_ok {
        return false;
    }
    if let Some(start_line) = start_line {
        let start_side = start_side.unwrap_or(side);
        if !diff
            .iter()
            .any(|item| diff_line_matches(item, path, start_line, start_side))
        {
            return false;
        }
    }
    true
}

fn diff_line_matches(
    item: &PullRequestDiffLine,
    path: &str,
    line: u32,
    side: PullRequestDiffSide,
) -> bool {
    if item.path != path {
        return false;
    }
    match side {
        PullRequestDiffSide::Left => item.old_line == Some(line),
        PullRequestDiffSide::Right => item.new_line == Some(line),
    }
}

fn flatten_paginated_array(value: &Value) -> Vec<Value> {
    match value {
        Value::Array(items) => {
            if items.iter().all(Value::is_array) {
                items
                    .iter()
                    .flat_map(|page| page.as_array().into_iter().flatten().cloned())
                    .collect()
            } else {
                items.clone()
            }
        }
        _ => Vec::new(),
    }
}

fn json_nodes(value: &Value, key: &str) -> Vec<Value> {
    let Some(value) = value.get(key) else {
        return Vec::new();
    };
    if let Some(nodes) = value.get("nodes").and_then(Value::as_array) {
        nodes.clone()
    } else if let Some(items) = value.as_array() {
        items.clone()
    } else {
        Vec::new()
    }
}

fn user_login(value: &Value) -> Option<String> {
    value
        .get("login")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

fn json_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn json_bool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn json_u32(value: &Value, key: &str) -> u32 {
    json_u32_opt(value, key).unwrap_or(0)
}

fn json_u32_opt(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

fn json_id(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|id| match id {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    })
}

fn parse_side(side: &str) -> Option<PullRequestDiffSide> {
    match side {
        "LEFT" => Some(PullRequestDiffSide::Left),
        "RIGHT" => Some(PullRequestDiffSide::Right),
        _ => None,
    }
}

fn github_side(side: PullRequestDiffSide) -> &'static str {
    match side {
        PullRequestDiffSide::Left => "LEFT",
        PullRequestDiffSide::Right => "RIGHT",
    }
}

trait IfEmpty {
    fn if_empty(&self, fallback: &'static str) -> &str;
}

impl IfEmpty for str {
    fn if_empty(&self, fallback: &'static str) -> &str {
        if self.is_empty() {
            fallback
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::VecDeque;
    use std::process::{ExitStatus, Output};
    use std::sync::Arc;

    #[cfg(unix)]
    fn exit_status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }

    fn successful_output(stdout: impl AsRef<[u8]>) -> Output {
        Output {
            status: exit_status(0),
            stdout: stdout.as_ref().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn failed_output(stderr: impl AsRef<[u8]>) -> Output {
        Output {
            status: exit_status(1),
            stdout: Vec::new(),
            stderr: stderr.as_ref().to_vec(),
        }
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    struct ExpectedGhCall {
        args: Vec<String>,
        output: std::io::Result<Output>,
    }

    #[derive(Clone, Default)]
    struct FakeGhRunner {
        calls: Arc<Mutex<Vec<Vec<String>>>>,
        expected: Arc<Mutex<VecDeque<ExpectedGhCall>>>,
    }

    impl FakeGhRunner {
        fn new(expected: Vec<ExpectedGhCall>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                expected: Arc::new(Mutex::new(expected.into())),
            }
        }

        fn assert_complete(&self) {
            assert!(
                self.expected.lock().is_empty(),
                "not all expected gh calls were made"
            );
        }
    }

    impl GhRunner for FakeGhRunner {
        fn output<'a>(
            &'a self,
            _repo: &'a Path,
            _ssh: Option<&'a SshTarget>,
            args: Vec<String>,
        ) -> BoxFuture<'a, std::io::Result<Output>> {
            Box::pin(async move {
                self.calls.lock().push(args.clone());
                let expected = self
                    .expected
                    .lock()
                    .pop_front()
                    .expect("unexpected gh call");
                assert_eq!(args, expected.args);
                expected.output
            })
        }
    }

    fn expect(args: Vec<String>, output: Output) -> ExpectedGhCall {
        ExpectedGhCall {
            args,
            output: Ok(output),
        }
    }

    fn default_view_args() -> Vec<String> {
        args(&["pr", "view", "--json", PR_VIEW_FIELDS])
    }

    fn query_view_args(query: &str) -> Vec<String> {
        args(&["pr", "view", query, "--json", PR_VIEW_FIELDS])
    }

    fn pr_view_args(number: u32) -> Vec<String> {
        args(&[
            "pr",
            "view",
            &number.to_string(),
            "--repo",
            "owner/repo",
            "--json",
            PR_VIEW_FIELDS,
        ])
    }

    fn list_open_args() -> Vec<String> {
        args(&[
            "pr",
            "list",
            "--state",
            "open",
            "--json",
            PR_LIST_FIELDS,
            "--limit",
            "50",
        ])
    }

    fn pr_view_json(number: u32, title: &str, head_ref: &str) -> String {
        serde_json::json!({
            "additions": 10,
            "author": { "login": "octocat" },
            "baseRefName": "main",
            "body": "Body",
            "changedFiles": 1,
            "deletions": 2,
            "headRefName": head_ref,
            "headRefOid": "abc123",
            "isDraft": false,
            "labels": { "nodes": [] },
            "mergeStateStatus": "CLEAN",
            "mergeable": "MERGEABLE",
            "number": number,
            "reviewDecision": null,
            "reviewRequests": { "nodes": [] },
            "state": "OPEN",
            "statusCheckRollup": { "nodes": [] },
            "title": title,
            "url": format!("https://github.com/owner/repo/pull/{number}")
        })
        .to_string()
    }

    fn pr_list_json(items: &[(u32, &str, &str)]) -> String {
        let items: Vec<Value> = items
            .iter()
            .map(|(number, title, head_ref)| {
                serde_json::json!({
                    "number": number,
                    "title": title,
                    "baseRefName": "main",
                    "headRefName": head_ref,
                    "url": format!("https://github.com/owner/repo/pull/{number}")
                })
            })
            .collect();
        serde_json::to_string(&items).unwrap()
    }

    fn detail_calls(number: u32) -> Vec<ExpectedGhCall> {
        vec![
            expect(
                args(&["api", "--hostname", "github.com", "user"]),
                successful_output(r#"{"login":"me"}"#),
            ),
            expect(
                args(&[
                    "api",
                    "--hostname",
                    "github.com",
                    &format!("repos/owner/repo/pulls/{number}/files?per_page=100"),
                    "--paginate",
                    "--slurp",
                ]),
                successful_output("[]"),
            ),
            expect(
                args(&[
                    "api",
                    "--hostname",
                    "github.com",
                    &format!("repos/owner/repo/issues/{number}/comments?per_page=100"),
                    "--paginate",
                    "--slurp",
                ]),
                successful_output("[]"),
            ),
            expect(
                args(&[
                    "api",
                    "--hostname",
                    "github.com",
                    &format!("repos/owner/repo/pulls/{number}/comments?per_page=100"),
                    "--paginate",
                    "--slurp",
                ]),
                successful_output("[]"),
            ),
            expect(
                args(&[
                    "api",
                    "--hostname",
                    "github.com",
                    &format!("repos/owner/repo/pulls/{number}"),
                    "-H",
                    "Accept: application/vnd.github.diff",
                ]),
                successful_output(""),
            ),
        ]
    }

    fn version_call() -> ExpectedGhCall {
        expect(args(&["--version"]), successful_output("gh version 2.0.0"))
    }

    #[tokio::test]
    async fn branch_pr_view_success_loads_that_pull_request() {
        let mut expected = vec![
            version_call(),
            expect(
                default_view_args(),
                successful_output(pr_view_json(15, "Branch PR", "feature")),
            ),
        ];
        expected.extend(detail_calls(15));
        let runner = FakeGhRunner::new(expected);

        let result = load_pull_request_with_runner(Path::new("."), None, None, None, &runner)
            .await
            .unwrap();

        let PullRequestLoadResult::Loaded(details) = result else {
            panic!("expected loaded pull request");
        };
        assert_eq!(details.pr.number, 15);
        assert_eq!(details.title, "Branch PR");
        runner.assert_complete();
    }

    #[tokio::test]
    async fn branch_not_found_with_one_open_repo_pr_loads_that_pull_request() {
        let mut expected = vec![
            version_call(),
            expect(
                default_view_args(),
                failed_output("no pull requests found for branch \"pr-viewer\""),
            ),
            expect(
                list_open_args(),
                successful_output(pr_list_json(&[(15, "Repo PR", "circular-corners-bug")])),
            ),
            expect(
                pr_view_args(15),
                successful_output(pr_view_json(15, "Repo PR", "circular-corners-bug")),
            ),
        ];
        expected.extend(detail_calls(15));
        let runner = FakeGhRunner::new(expected);

        let result = load_pull_request_with_runner(Path::new("."), None, None, None, &runner)
            .await
            .unwrap();

        let PullRequestLoadResult::Loaded(details) = result else {
            panic!("expected loaded pull request");
        };
        assert_eq!(details.pr.number, 15);
        assert_eq!(details.head_ref, "circular-corners-bug");
        runner.assert_complete();
    }

    #[tokio::test]
    async fn branch_not_found_with_multiple_open_repo_prs_returns_candidates() {
        let runner = FakeGhRunner::new(vec![
            version_call(),
            expect(
                default_view_args(),
                failed_output("no pull request for branch"),
            ),
            expect(
                list_open_args(),
                successful_output(pr_list_json(&[
                    (15, "First PR", "one"),
                    (16, "Second PR", "two"),
                ])),
            ),
        ]);

        let result = load_pull_request_with_runner(Path::new("."), None, None, None, &runner)
            .await
            .unwrap();

        let PullRequestLoadResult::Candidates(candidates) = result else {
            panic!("expected pull request candidates");
        };
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].pr.number, 15);
        assert_eq!(candidates[1].head_ref, "two");
        runner.assert_complete();
    }

    #[tokio::test]
    async fn branch_not_found_with_zero_open_repo_prs_returns_not_found() {
        let runner = FakeGhRunner::new(vec![
            version_call(),
            expect(default_view_args(), failed_output("no pull requests found")),
            expect(list_open_args(), successful_output("[]")),
        ]);

        let err = load_pull_request_with_runner(Path::new("."), None, None, None, &runner)
            .await
            .unwrap_err();

        assert!(matches!(err, PullRequestError::NotFound(_)));
        assert_eq!(err.message(), default_not_found_message());
        runner.assert_complete();
    }

    #[tokio::test]
    async fn explicit_query_does_not_run_repo_level_fallback() {
        let mut expected = vec![
            version_call(),
            expect(
                query_view_args("15"),
                successful_output(pr_view_json(15, "Explicit PR", "feature")),
            ),
        ];
        expected.extend(detail_calls(15));
        let runner = FakeGhRunner::new(expected);

        let result = load_pull_request_with_runner(Path::new("."), None, None, Some("15"), &runner)
            .await
            .unwrap();

        let PullRequestLoadResult::Loaded(details) = result else {
            panic!("expected loaded pull request");
        };
        assert_eq!(details.pr.number, 15);
        assert_eq!(details.title, "Explicit PR");
        runner.assert_complete();
    }

    #[test]
    fn parses_github_pr_url() {
        let parsed = parse_github_pr_url("https://github.com/owner/repo/pull/42").unwrap();
        assert_eq!(parsed.provider, PullRequestProvider::GitHub);
        assert_eq!(parsed.host, "github.com");
        assert_eq!(parsed.owner, "owner");
        assert_eq!(parsed.repo, "repo");
        assert_eq!(parsed.number, 42);
    }

    #[test]
    fn parses_enterprise_pr_url() {
        let parsed = parse_github_pr_url("https://git.example.test/acme/widgets/pull/7").unwrap();
        assert_eq!(parsed.host, "git.example.test");
        assert_eq!(parsed.owner, "acme");
        assert_eq!(parsed.repo, "widgets");
        assert_eq!(parsed.number, 7);
    }

    #[test]
    fn flattens_slurped_pages() {
        let value = serde_json::json!([[{"id": 1}], [{"id": 2}, {"id": 3}]]);
        let ids: Vec<String> = flatten_paginated_array(&value)
            .into_iter()
            .filter_map(|v| json_id(&v, "id"))
            .collect();
        assert_eq!(ids, ["1", "2", "3"]);
    }

    #[test]
    fn marks_only_current_user_comments_editable() {
        let value = serde_json::json!({
            "id": 12345678901234567890u128,
            "body": "hello",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "html_url": "https://github.com/o/r/pull/1#issuecomment-1",
            "user": { "login": "me" }
        });
        let comment = comment_from_issue_json(&value, Some("me")).unwrap();
        assert!(comment.can_edit);
        assert!(comment.can_delete);
        assert_eq!(comment.target.id, "12345678901234567890");

        let other = comment_from_issue_json(&value, Some("someone-else")).unwrap();
        assert!(!other.can_edit);
        assert!(!other.can_delete);
    }

    #[test]
    fn parses_diff_lines_and_comment_targets() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,3 +10,4 @@ fn demo() {
 context
-old
+new
+another
 }";
        let lines = parse_pr_diff(diff);
        assert!(lines.iter().any(|line| {
            line.kind == PullRequestDiffLineKind::Add
                && line.path == "src/lib.rs"
                && line.new_line == Some(11)
        }));
        assert!(diff_contains_comment_target(
            &lines,
            "src/lib.rs",
            12,
            PullRequestDiffSide::Right,
            Some(11),
            Some(PullRequestDiffSide::Right)
        ));
        assert!(diff_contains_comment_target(
            &lines,
            "src/lib.rs",
            11,
            PullRequestDiffSide::Left,
            None,
            None
        ));
        assert!(!diff_contains_comment_target(
            &lines,
            "src/lib.rs",
            99,
            PullRequestDiffSide::Right,
            None,
            None
        ));
    }

    #[test]
    fn deleted_file_diff_keeps_old_path_for_left_comments() {
        let diff = "\
diff --git a/old.rs b/old.rs
--- a/old.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-old
-gone";
        let lines = parse_pr_diff(diff);
        assert!(diff_contains_comment_target(
            &lines,
            "old.rs",
            2,
            PullRequestDiffSide::Left,
            None,
            None
        ));
    }

    #[test]
    fn setup_messages_are_local_or_remote() {
        let local = gh_setup_message(None, "missing");
        assert!(local.contains("this machine"));
        let remote = gh_setup_message(
            Some(&SshTarget {
                host: "devbox".into(),
                user: Some("ian".into()),
                port: None,
            }),
            "missing",
        );
        assert!(remote.contains("ian@devbox"));
        assert!(remote.contains("SSH host"));
    }

    #[test]
    fn summarizes_status_check_rollup() {
        let value = serde_json::json!({
            "nodes": [
                { "status": "COMPLETED", "conclusion": "SUCCESS" },
                { "status": "COMPLETED", "conclusion": "FAILURE" },
                { "status": "IN_PROGRESS" },
                { "status": "COMPLETED", "conclusion": "SKIPPED" }
            ]
        });
        let summary = checks_summary(Some(&value));
        assert_eq!(summary.total, 4);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.pending, 1);
        assert_eq!(summary.skipped, 1);
    }
}
