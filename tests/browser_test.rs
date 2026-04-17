use std::fs;

use artix::model::EntryKind;
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
    let entries = browse_directory(&root, &root).await.expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind.clone()))
        .collect::<Vec<_>>();

    // No ".." entry at root
    assert!(!names.iter().any(|(name, _)| *name == ".."));
    assert_eq!(names[0], ("target", EntryKind::CleanupCandidate));
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
        .map(|entry| (entry.name.as_str(), entry.entry_kind.clone()))
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

    let entries = browse_directory(&root, &root).await.expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind.clone()))
        .collect::<Vec<_>>();

    // No ".." at root, sorted by size descending
    assert_eq!(names[0], ("target", EntryKind::CleanupCandidate));
    assert_eq!(names[1], ("node_modules", EntryKind::CleanupCandidate));
    assert_eq!(names[2], ("src", EntryKind::Directory));
}
