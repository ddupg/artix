use std::fs;

use artix::delete::{delete_directories, DeleteMode};
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
fn delete_directories_reports_missing_path_failure() {
    let result = delete_directories(
        &[std::path::PathBuf::from("/tmp/does-not-exist")],
        DeleteMode::Permanent { confirmed: true },
    );

    let err = result.unwrap_err();
    assert!(!err.is_empty());
}
