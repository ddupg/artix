use std::env;
use std::ffi::OsString;
use std::fs;

use artix::delete::{DeleteMode, delete_directories};
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
        env::set_var("ARTIX_FORCE_BUILTIN_TRASH", "1");
    }

    let result = delete_directories(std::slice::from_ref(&doomed), DeleteMode::Trash);

    restore_env("HOME", original_home);
    unsafe {
        env::remove_var("ARTIX_FORCE_BUILTIN_TRASH");
    }

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
