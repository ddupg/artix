use std::path::PathBuf;

use artix::classify::ownership::{infer_project_roots, resolve_owner_project};

#[test]
fn nested_workspace_member_wins_ownership_over_repo_root() {
    let markers = vec![
        PathBuf::from("/repo/Cargo.toml"),
        PathBuf::from("/repo/packages/app/Cargo.toml"),
    ];
    let roots = infer_project_roots(&markers);

    let owner = resolve_owner_project(
        PathBuf::from("/repo/packages/app/src/main.rs").as_path(),
        &roots,
    );

    assert_eq!(owner, Some(PathBuf::from("/repo/packages/app")));
}

#[test]
fn falls_back_to_repo_root_when_no_nested_marker_exists() {
    let markers = vec![PathBuf::from("/repo/Cargo.toml")];
    let roots = infer_project_roots(&markers);

    let owner = resolve_owner_project(PathBuf::from("/repo/src/lib.rs").as_path(), &roots);

    assert_eq!(owner, Some(PathBuf::from("/repo")));
}

#[test]
fn deeper_nested_workspace_member_still_wins_over_parent_workspace() {
    let markers = vec![
        PathBuf::from("/repo/Cargo.toml"),
        PathBuf::from("/repo/packages/app/Cargo.toml"),
        PathBuf::from("/repo/packages/app/tools/cli/Cargo.toml"),
    ];
    let roots = infer_project_roots(&markers);

    let owner = resolve_owner_project(
        PathBuf::from("/repo/packages/app/tools/cli/src/main.rs").as_path(),
        &roots,
    );

    assert_eq!(owner, Some(PathBuf::from("/repo/packages/app/tools/cli")));
}
