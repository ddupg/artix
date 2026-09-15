use std::fs;
#[cfg(unix)]
use std::process::Command;

use artix::delete::{DeleteMode, delete_directories};
use tempfile::tempdir;

#[test]
fn delete_directories_requires_explicit_confirmation_for_permanent_delete() {
    let temp = tempdir().unwrap();
    let doomed = temp.path().join("target");
    fs::create_dir_all(&doomed).unwrap();

    let result = delete_directories(&[doomed], DeleteMode::Permanent { confirmed: false });

    assert_eq!(
        result.unwrap_err(),
        "permanent delete requires explicit confirmation"
    );
}

#[test]
fn delete_directories_permanently_deletes_file() {
    let temp = tempdir().unwrap();
    let doomed = temp.path().join("large.log");
    fs::write(&doomed, "artifact").unwrap();

    delete_directories(
        std::slice::from_ref(&doomed),
        DeleteMode::Permanent { confirmed: true },
    )
    .unwrap();

    assert!(!doomed.exists());
}

#[test]
fn delete_directories_permanently_deletes_directory() {
    let temp = tempdir().unwrap();
    let doomed = temp.path().join("target");
    fs::create_dir_all(doomed.join("debug")).unwrap();
    fs::write(doomed.join("debug/app"), "artifact").unwrap();

    delete_directories(
        std::slice::from_ref(&doomed),
        DeleteMode::Permanent { confirmed: true },
    )
    .unwrap();

    assert!(!doomed.exists());
}

#[cfg(unix)]
#[test]
fn permanent_delete_handles_deep_tree_under_low_fd_limit() {
    use std::os::unix::fs::symlink;

    const CHILD_ENV: &str = "ARTIX_LOW_FD_DELETE_TEST_CHILD";

    if std::env::var_os(CHILD_ENV).is_none() {
        let current_exe = std::env::current_exe().expect("current test executable");
        let status = Command::new("sh")
            .args(["-c", "ulimit -n 32 && exec \"$@\"", "sh"])
            .arg(current_exe)
            .args([
                "--exact",
                "permanent_delete_handles_deep_tree_under_low_fd_limit",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .status()
            .expect("run low-fd child test");
        assert!(status.success(), "low-fd child test failed: {status}");
        return;
    }

    let temp = tempdir().expect("tempdir");
    let doomed = temp.path().join("target");
    let mut path = doomed.clone();
    fs::create_dir(&doomed).expect("create target");
    for _ in 0..64 {
        path.push("d");
        fs::create_dir(&path).expect("create nested directory");
    }
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).expect("create outside directory");
    let sentinel = outside.join("sentinel");
    fs::write(&sentinel, "keep").expect("write outside sentinel");
    symlink(&outside, path.join("outside-link")).expect("create outside symlink");
    fs::write(path.join("artifact"), "data").expect("write artifact");

    delete_directories(
        std::slice::from_ref(&doomed),
        DeleteMode::Permanent { confirmed: true },
    )
    .expect("delete deep tree");

    assert!(!doomed.exists());
    assert!(sentinel.exists(), "permanent delete followed a symlink");
}

#[test]
fn delete_directories_reports_missing_path_failure() {
    let result = delete_directories(
        &[std::path::PathBuf::from("/tmp/does-not-exist")],
        DeleteMode::Permanent { confirmed: true },
    );

    let err = result.unwrap_err();
    assert!(!err.is_empty());
}
