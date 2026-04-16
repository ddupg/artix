use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use artix::classify::git::classify_path_git_status;
use artix::model::{GitContext, GitStatus};
use tempfile::tempdir;

// Regression: ISSUE-002 — git subprocess stderr polluted the TUI
// Found by /investigate on 2026-04-16
// Report: user-reported terminal corruption during TUI browsing
#[test]
fn git_subprocess_output_is_suppressed_for_untracked_paths() {
    if env::var_os("ARTIX_TEST_CHILD_SUPPRESS_GIT").is_some() {
        child_probe();
        return;
    }

    let temp = tempdir().unwrap();
    let root = temp.path().join("repo");
    let bin_dir = temp.path().join("bin");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(root.join(".gstack"), "probe").unwrap();

    let fake_git = bin_dir.join("git");
    fs::write(
        &fake_git,
        "#!/bin/sh\nprintf 'fake git stderr noise\\n' >&2\nprintf 'fake git stdout noise\\n'\nexit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&fake_git).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_git, perms).unwrap();

    let original_path = env::var_os("PATH").unwrap_or_default();
    let path = format!(
        "{}:{}",
        bin_dir.display(),
        PathBuf::from(original_path).display()
    );
    let output = Command::new(env::current_exe().unwrap())
        .env("ARTIX_TEST_CHILD_SUPPRESS_GIT", "1")
        .env("ARTIX_TEST_ROOT", &root)
        .env("PATH", path)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "",
        "git child stderr leaked into parent process"
    );
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("fake git stderr noise"),
        "git child noise leaked into parent stdout"
    );
}

fn child_probe() {
    let root = PathBuf::from(env::var("ARTIX_TEST_ROOT").unwrap());
    let context = GitContext {
        repo_root: Some(root.clone()),
        git_dir: None,
        common_dir: None,
        worktree_root: Some(root.clone()),
        branch_name: None,
        head_ref: None,
        head_state: artix::model::HeadState::Unknown,
        is_worktree: false,
    };
    let status = classify_path_git_status(&root.join(".gstack"), Some(&context));
    assert_eq!(status, GitStatus::Untracked);
}
