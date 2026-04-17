use std::path::PathBuf;

use artix::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel};
use artix::ui::{AppState, DeleteIntent, DeleteTargetKind, FilterMode};

#[test]
fn cleanup_focus_hides_tracked_entries_but_keeps_ignored_and_unknown() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(
        cwd.clone(),
        vec![
            BrowserEntry::parent(cwd.parent().expect("parent").to_path_buf()),
            entry("src", GitStatus::Tracked, 5),
            entry("target", GitStatus::Ignored, 100),
            entry("scratch", GitStatus::Unknown, 15),
        ],
    );

    app.set_filter_mode(FilterMode::CleanupFocus);

    let visible_entries = app.visible_entries();
    let visible_names = visible_entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(visible_names, vec!["..", "target", "scratch"]);
}

#[test]
fn tracked_targets_require_stronger_delete_confirmation() {
    let cwd = PathBuf::from("/workspace/repo");
    let tracked = entry("src", GitStatus::Tracked, 5);
    let unknown = entry("scratch", GitStatus::Unknown, 15);
    let ignored = entry("target", GitStatus::Ignored, 100);

    let app = AppState::new(cwd, vec![tracked.clone(), unknown.clone(), ignored.clone()]);

    assert_eq!(
        app.delete_intent_for(&tracked),
        DeleteIntent::Confirm {
            target_kind: DeleteTargetKind::TrackedOrUnknown,
            requires_extra_confirmation: true,
        }
    );
    assert_eq!(
        app.delete_intent_for(&unknown),
        DeleteIntent::Confirm {
            target_kind: DeleteTargetKind::TrackedOrUnknown,
            requires_extra_confirmation: true,
        }
    );
    assert_eq!(
        app.delete_intent_for(&ignored),
        DeleteIntent::Confirm {
            target_kind: DeleteTargetKind::CleanupCandidate,
            requires_extra_confirmation: false,
        }
    );
}

#[test]
fn selection_wraps_up_from_first_to_last_in_visible_list() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(
        cwd.clone(),
        vec![
            BrowserEntry::parent(cwd.parent().expect("parent").to_path_buf()),
            entry("a", GitStatus::Unknown, 1),
            entry("b", GitStatus::Unknown, 1),
        ],
    );

    // Initially selects the first visible entry ("..")
    assert_eq!(app.selected_entry().expect("selected").name, "..");
    app.move_selection_up();
    assert_eq!(app.selected_entry().expect("selected").name, "b");
}

#[test]
fn selection_wraps_down_from_last_to_first_in_visible_list() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(
        cwd.clone(),
        vec![
            BrowserEntry::parent(cwd.parent().expect("parent").to_path_buf()),
            entry("a", GitStatus::Unknown, 1),
            entry("b", GitStatus::Unknown, 1),
        ],
    );

    app.move_selection_up(); // .. -> b
    assert_eq!(app.selected_entry().expect("selected").name, "b");
    app.move_selection_down(); // b -> ..
    assert_eq!(app.selected_entry().expect("selected").name, "..");
}

#[test]
fn selection_wraps_with_filter_mode_using_visible_entries() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(
        cwd.clone(),
        vec![
            BrowserEntry::parent(cwd.parent().expect("parent").to_path_buf()),
            entry("src", GitStatus::Tracked, 5),
            entry("target", GitStatus::Ignored, 100),
            entry("scratch", GitStatus::Unknown, 15),
        ],
    );

    app.set_filter_mode(FilterMode::CleanupFocus);
    assert_eq!(app.visible_entries().len(), 3);
    assert_eq!(app.selected_entry().expect("selected").name, "..");
    app.move_selection_up();
    assert_eq!(app.selected_entry().expect("selected").name, "scratch");
    app.move_selection_down();
    assert_eq!(app.selected_entry().expect("selected").name, "..");
}

#[test]
fn selection_moves_do_not_panic_on_empty_list() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(cwd, vec![]);
    app.move_selection_up();
    app.move_selection_down();
    assert!(app.selected_entry().is_none());
}

fn entry(name: &str, git_status: GitStatus, size_bytes: u64) -> BrowserEntry {
    BrowserEntry {
        path: PathBuf::from(format!("/workspace/repo/{name}")),
        name: name.to_string(),
        size_bytes,
        reclaimable_bytes: size_bytes,
        entry_kind: EntryKind::Directory,
        git_status,
        git_context: GitContext::default(),
        risk_level: RiskLevel::Low,
        candidate_kind: None,
        is_visible_candidate: false,
    }
}
