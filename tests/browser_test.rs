use std::fs;

use artix::model::{CandidateKind, EntryKind};
use artix::scan::browse_directory;
use tempfile::tempdir;

#[tokio::test]
async fn browse_directory_root_has_no_parent_entry() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("create src");
    fs::create_dir_all(root.join("target/debug")).expect("create target");
    fs::write(root.join("src/lib.rs"), "fn main() {}\n").expect("write src");
    fs::write(
        root.join("target/debug/app"),
        "123456789012345678901234567890",
    )
    .expect("write target");

    // When browsing the root directory (same as start_dir), no ".." entry should be present
    let entries = browse_directory(&root, &root)
        .await
        .expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind))
        .collect::<Vec<_>>();

    // No ".." entry at root
    assert!(!names.iter().any(|(name, _)| *name == ".."));
    assert_eq!(
        names[0],
        (
            "target",
            EntryKind::CleanupCandidate(CandidateKind::RustTarget),
        )
    );
    assert_eq!(names[1], ("src", EntryKind::Directory));

    let src = entries
        .iter()
        .find(|entry| entry.name == "src")
        .expect("src entry");
    assert!(src.size_bytes > 0, "expected src directory size to be > 0");
}

#[tokio::test]
async fn browse_directory_subdirectory_has_parent_entry() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("create src");
    fs::create_dir_all(root.join("target/debug")).expect("create target");
    fs::write(root.join("src/lib.rs"), "fn main() {}\n").expect("write src");
    fs::write(
        root.join("target/debug/app"),
        "123456789012345678901234567890",
    )
    .expect("write target");

    // When browsing a subdirectory, ".." entry should be present
    let entries = browse_directory(&root.join("src"), &root)
        .await
        .expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind))
        .collect::<Vec<_>>();

    // ".." entry should be first in subdirectory
    assert_eq!(names[0], ("..", EntryKind::Parent));
}

#[tokio::test]
async fn browse_directory_sorts_cleanup_candidates_by_size() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(root.join("src")).expect("create src");
    fs::create_dir_all(root.join("target/debug")).expect("create target");
    fs::create_dir_all(root.join("node_modules/react")).expect("create node_modules");
    fs::write(root.join("src/lib.rs"), "fn main() {}\n").expect("write src");
    fs::write(
        root.join("target/debug/app"),
        "123456789012345678901234567890",
    )
    .expect("write target");
    fs::write(
        root.join("node_modules/react/index.js"),
        "12345678901234567890",
    )
    .expect("write node_modules");

    let entries = browse_directory(&root, &root)
        .await
        .expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind))
        .collect::<Vec<_>>();

    // No ".." at root, sorted by size descending
    assert_eq!(
        names[0],
        (
            "target",
            EntryKind::CleanupCandidate(CandidateKind::RustTarget),
        )
    );
    assert_eq!(
        names[1],
        (
            "node_modules",
            EntryKind::CleanupCandidate(CandidateKind::NodeModules),
        )
    );
    assert_eq!(names[2], ("src", EntryKind::Directory));
}

#[tokio::test]
async fn browse_directory_includes_files_and_sorts_by_size() {
    let temp = tempdir().expect("tempdir");
    let root = temp.path().join("repo");
    fs::create_dir_all(root.join("target/debug")).expect("create target");
    fs::create_dir_all(root.join("small-dir")).expect("create small dir");
    fs::write(root.join("large.log"), "x".repeat(50)).expect("write large file");
    fs::write(root.join(".gitignore"), "target\n").expect("write gitignore");
    fs::write(root.join("target/debug/app"), "x".repeat(20)).expect("write target");
    fs::write(root.join("small-dir/file.txt"), "x").expect("write small dir file");

    let entries = browse_directory(&root, &root)
        .await
        .expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind, entry.size_bytes))
        .collect::<Vec<_>>();

    assert_eq!(names[0].0, "large.log");
    assert_eq!(names[0].1, EntryKind::File);
    assert_eq!(names[0].2, 50);
    assert_eq!(names[1].0, "target");
    assert_eq!(
        names[1].1,
        EntryKind::CleanupCandidate(CandidateKind::RustTarget)
    );
    assert!(
        names
            .iter()
            .any(|(name, kind, _)| { *name == ".gitignore" && matches!(kind, EntryKind::File) })
    );
}
