use std::fs;

use artix::model::EntryKind;
use artix::scan::browse_directory;
use tempfile::tempdir;

#[test]
fn browse_directory_includes_parent_and_sorts_cleanup_candidates_by_size() {
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

    let entries = browse_directory(&root).expect("browse directory");
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.entry_kind.clone()))
        .collect::<Vec<_>>();

    assert_eq!(names[0], ("..", EntryKind::Parent));
    assert_eq!(names[1], ("target", EntryKind::CleanupCandidate));
    assert_eq!(names[2], ("node_modules", EntryKind::CleanupCandidate));
    assert_eq!(names[3], ("src", EntryKind::Directory));
}
