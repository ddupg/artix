use std::fs;
use std::process::Command;

use artix::classify::git::resolve_git_context;
use artix::config::AppContext;
use artix::git_storage::{GitStorageTarget, analyze_git_storage, execute_git_gc};
use tempfile::tempdir;

#[tokio::test]
async fn analysis_uses_git_count_objects_and_reports_lfs_separately() {
    let repo = tempdir().expect("repo");
    run_git(repo.path(), ["init", "-q"]);
    let common_dir = repo.path().join(".git");
    fs::create_dir_all(common_dir.join("lfs/objects")).expect("lfs directory");
    fs::write(common_dir.join("lfs/objects/payload"), vec![0; 2048]).expect("lfs payload");
    fs::write(
        common_dir.join("objects/pack/tmp_pack_artix_test"),
        vec![0; 4096],
    )
    .expect("temporary pack");

    let context = resolve_git_context(repo.path()).expect("git context");
    let target = GitStorageTarget::for_repo_root(repo.path(), &context).expect("repo root");
    let analysis = analyze_git_storage(target, &AppContext::default())
        .await
        .expect("analysis");

    assert_eq!(analysis.target.common_dir, common_dir);
    assert!(analysis.total_size_bytes >= 6144);
    assert!(analysis.garbage_count >= 1);
    assert!(analysis.garbage_size_bytes >= 4096);
    assert_eq!(analysis.lfs_size_bytes, 2048);
}

#[tokio::test]
async fn conservative_gc_runs_without_extra_flags() {
    let repo = tempdir().expect("repo");
    run_git(repo.path(), ["init", "-q"]);
    let context = resolve_git_context(repo.path()).expect("git context");
    let target = GitStorageTarget::for_repo_root(repo.path(), &context).expect("repo root");

    let result = execute_git_gc(target, &AppContext::default()).await;

    assert!(result.success, "{}", result.message());
}

fn run_git<const N: usize>(cwd: &std::path::Path, args: [&str; N]) {
    let status = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .status()
        .expect("git available");
    assert!(status.success());
}
