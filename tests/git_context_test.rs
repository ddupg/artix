use std::fs;
use std::path::Path;
use std::process::Command;

use artix::classify::git::resolve_git_context;
use tempfile::tempdir;

#[test]
fn resolve_git_context_detects_branch_for_a_worktree() {
    let temp = tempdir().expect("tempdir");
    let repo_root = temp.path().join("repo");
    let worktree_root = temp.path().join("repo-feature");

    fs::create_dir_all(&repo_root).expect("create repo root");
    run_git(&repo_root, ["init", "--initial-branch=main"]);
    run_git(&repo_root, ["config", "user.name", "Artix Tests"]);
    run_git(&repo_root, ["config", "user.email", "artix@example.com"]);
    fs::write(repo_root.join("README.md"), "seed\n").expect("write readme");
    run_git(&repo_root, ["add", "README.md"]);
    run_git(&repo_root, ["commit", "-m", "seed"]);
    run_git(&repo_root, ["branch", "feature/worktree"]);
    run_git(
        &repo_root,
        [
            "worktree",
            "add",
            worktree_root.to_str().expect("worktree path utf8"),
            "feature/worktree",
        ],
    );

    let context = resolve_git_context(&worktree_root).expect("worktree git context");

    assert!(context.is_worktree);
    assert_eq!(context.worktree_root, Some(worktree_root.clone()));
    assert_eq!(context.branch_name.as_deref(), Some("feature/worktree"));
    assert_eq!(
        context.head_ref.as_deref(),
        Some("refs/heads/feature/worktree")
    );
}

fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("run git");
    assert!(
        status.success(),
        "git {:?} failed in {}",
        args,
        cwd.display()
    );
}
