use std::fs;

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

#[test]
fn delete_directories_reports_missing_path_failure() {
    let result = delete_directories(
        &[std::path::PathBuf::from("/tmp/does-not-exist")],
        DeleteMode::Permanent { confirmed: true },
    );

    let err = result.unwrap_err();
    assert!(!err.is_empty());
}
