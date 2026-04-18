use std::env;
use std::ffi::OsString;
use std::fs;

use artix::config::{DeleteConfig, TrashBackend};
use artix::delete::{DeleteMode, delete_directories_with_config};
use tempfile::tempdir;

// Regression: ISSUE-001 — trash delete failed in headless macOS sessions
// Found by /qa on 2026-04-16
// Report: .gstack/qa-reports/qa-report-artix-cli-2026-04-16.md
#[test]
fn trash_delete_uses_builtin_fallback_when_requested() {
    let temp = tempdir().unwrap();
    let fake_home = temp.path().join("home");
    let doomed = temp.path().join("target");
    let original_home = env::var_os("HOME");

    fs::create_dir_all(fake_home.join(".Trash")).unwrap();
    fs::create_dir_all(&doomed).unwrap();
    fs::write(doomed.join("artifact.bin"), "artifact").unwrap();

    unsafe {
        env::set_var("HOME", &fake_home);
    }

    let result = delete_directories_with_config(
        std::slice::from_ref(&doomed),
        DeleteMode::Trash,
        &DeleteConfig {
            trash_backend: TrashBackend::Builtin,
        },
    );

    restore_env("HOME", original_home);

    result.unwrap();
    assert!(!doomed.exists());
    assert!(fake_home.join(".Trash/target").exists());
}

fn restore_env(key: &str, value: Option<OsString>) {
    match value {
        Some(value) => unsafe {
            env::set_var(key, value);
        },
        None => unsafe {
            env::remove_var(key);
        },
    }
}
