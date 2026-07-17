use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use crate::config::AppContext;
use crate::model::{GitContext, GitStatus, HeadState};

pub fn resolve_git_context(path: &Path) -> Option<GitContext> {
    let repo = gix::discover(path).ok()?;
    let head = repo.head().ok();
    let head_ref = head
        .as_ref()
        .and_then(|head| head.referent_name())
        .map(|name| name.to_string());
    let branch_name = head_ref
        .as_deref()
        .and_then(|name| name.strip_prefix("refs/heads/"))
        .map(str::to_string);
    let head_state = match head.as_ref() {
        Some(head) if head.is_detached() => HeadState::Detached,
        Some(head) if head.referent_name().is_some() => HeadState::Branch,
        _ => HeadState::Unknown,
    };
    let worktree_root = repo.workdir().map(Path::to_path_buf);
    let repo_root = worktree_root
        .clone()
        .or_else(|| Some(repo.git_dir().to_path_buf()));

    Some(GitContext {
        repo_root,
        git_dir: Some(repo.git_dir().to_path_buf()),
        common_dir: Some(repo.common_dir().to_path_buf()),
        worktree_root,
        branch_name,
        head_ref,
        head_state,
        is_worktree: repo.kind() == gix::repository::Kind::LinkedWorkTree,
    })
}

pub async fn classify_path_git_status(
    path: &Path,
    git_context: Option<&GitContext>,
    ctx: &AppContext,
) -> GitStatus {
    let Some(context) = git_context else {
        return GitStatus::Unknown;
    };
    let Some(worktree_root) = context
        .worktree_root
        .as_ref()
        .or(context.repo_root.as_ref())
    else {
        return GitStatus::Unknown;
    };
    let Ok(relative) = path.strip_prefix(worktree_root) else {
        return GitStatus::Unknown;
    };

    if relative.as_os_str().is_empty() {
        return GitStatus::Tracked;
    }

    let relative_arg = relative.to_string_lossy().to_string();
    match git_command_outcome(
        worktree_root,
        ["check-ignore", "-q", "--", relative_arg.as_str()],
        ctx,
    )
    .await
    {
        GitCommandOutcome::Matched => return GitStatus::Ignored,
        GitCommandOutcome::NotMatched => {}
        GitCommandOutcome::Failed => return GitStatus::Unknown,
    }

    match git_command_outcome(
        worktree_root,
        ["ls-files", "--error-unmatch", "--", relative_arg.as_str()],
        ctx,
    )
    .await
    {
        GitCommandOutcome::Matched => GitStatus::Tracked,
        GitCommandOutcome::NotMatched if path.exists() => GitStatus::Untracked,
        GitCommandOutcome::NotMatched | GitCommandOutcome::Failed => GitStatus::Unknown,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitCommandOutcome {
    Matched,
    NotMatched,
    Failed,
}

async fn git_command_outcome<const N: usize>(
    cwd: &Path,
    args: [&str; N],
    ctx: &AppContext,
) -> GitCommandOutcome {
    let sem = ctx.git_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");

    let mut cmd = tokio::process::Command::new("git");
    cmd.current_dir(cwd)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    match tokio::time::timeout(Duration::from_secs(2), cmd.status()).await {
        Ok(Ok(status)) if status.success() => GitCommandOutcome::Matched,
        Ok(Ok(status)) if status.code() == Some(1) => GitCommandOutcome::NotMatched,
        _ => GitCommandOutcome::Failed,
    }
}
