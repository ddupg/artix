use std::path::PathBuf;
use std::time::{Duration, Instant};

use artix::clean::{
    CleanCommand, CleanCommandResult, CleanPlan, CleanRunSummary, ProjectKind, ProjectProfile,
};
use artix::delete::DeleteMode;
use artix::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel};
use artix::ui::{AppState, CleanState, DeleteIntent, DeleteState, DeleteTargetKind, FilterMode};

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

#[test]
fn selection_jump_to_first_selects_first_visible_entry() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(
        cwd.clone(),
        vec![
            BrowserEntry::parent(cwd.parent().expect("parent").to_path_buf()),
            entry("a", GitStatus::Unknown, 1),
            entry("b", GitStatus::Unknown, 1),
        ],
    );

    app.jump_to_last();
    assert_eq!(app.selected_entry().expect("selected").name, "b");

    app.jump_to_first();
    assert_eq!(app.selected_entry().expect("selected").name, "..");
}

#[test]
fn selection_jump_to_last_uses_visible_entries_under_filter() {
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
    app.jump_to_last();
    assert_eq!(app.selected_entry().expect("selected").name, "scratch");
}

#[test]
fn replace_entries_can_preserve_selected_path() {
    let cwd = PathBuf::from("/workspace/repo");
    let parent = BrowserEntry::parent(cwd.parent().expect("parent").to_path_buf());
    let mut app = AppState::new(
        cwd.clone(),
        vec![
            parent.clone(),
            entry("a", GitStatus::Unknown, 1),
            entry("b", GitStatus::Unknown, 1),
        ],
    );
    app.move_selection_down();
    app.move_selection_down();
    let selected_path = app.selected_entry().expect("selected").path;

    app.replace_entries_preserving_selection(
        cwd,
        vec![
            parent,
            entry("b", GitStatus::Ignored, 20),
            entry("a", GitStatus::Unknown, 1),
        ],
        Some(&selected_path),
    );

    assert_eq!(app.selected_entry().expect("selected").name, "b");
}

#[test]
fn selection_jumps_do_not_panic_on_empty_list() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(cwd, vec![]);
    app.jump_to_first();
    app.jump_to_last();
    assert!(app.selected_entry().is_none());
}

#[test]
fn clean_shortcut_is_disabled_without_project_profile() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(cwd, vec![]);

    app.request_clean_current_dir();

    assert_eq!(app.clean_state(), &CleanState::Idle);
}

#[test]
fn clean_shortcut_opens_confirmation_when_plan_exists() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(cwd.clone(), vec![]);
    app.set_current_project_profile(Some(profile_with_command(cwd)));

    app.request_clean_current_dir();

    assert!(matches!(
        app.clean_state(),
        CleanState::Confirming { plan } if plan.commands[0].display() == "cargo clean"
    ));
}

#[test]
fn clean_shortcut_is_disabled_when_profile_has_no_commands() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut profile = profile_with_command(cwd.clone());
    profile.clean_plan.commands.clear();
    let mut app = AppState::new(cwd, vec![]);
    app.set_current_project_profile(Some(profile));

    app.request_clean_current_dir();

    assert_eq!(app.clean_state(), &CleanState::Idle);
}

#[test]
fn clean_result_auto_dismisses_after_three_seconds() {
    let cwd = PathBuf::from("/workspace/repo");
    let mut app = AppState::new(cwd.clone(), vec![]);
    let now = Instant::now();

    app.finish_clean(clean_summary(cwd), now);
    app.dismiss_expired_clean_result(now + Duration::from_secs(2));
    assert!(matches!(app.clean_state(), CleanState::Finished { .. }));

    app.dismiss_expired_clean_result(now + Duration::from_secs(3));
    assert_eq!(app.clean_state(), &CleanState::Idle);
}

#[test]
fn permanent_delete_for_tracked_entry_waits_for_extra_confirmation() {
    let cwd = PathBuf::from("/workspace/repo");
    let tracked = entry("src", GitStatus::Tracked, 5);
    let mut app = AppState::new(cwd, vec![tracked]);

    app.request_delete_for_selected();
    app.set_delete_mode(DeleteMode::Permanent { confirmed: true });
    app.request_extra_confirmation();

    assert!(matches!(
        app.delete_state(),
        DeleteState::AwaitingExtraConfirmation { mode, .. }
            if matches!(mode, DeleteMode::Permanent { confirmed: true })
    ));
}

#[test]
fn trash_delete_can_run_from_delete_confirmation() {
    let cwd = PathBuf::from("/workspace/repo");
    let ignored = entry("target", GitStatus::Ignored, 100);
    let mut app = AppState::new(cwd, vec![ignored]);

    app.request_delete_for_selected();
    app.set_delete_mode(DeleteMode::Trash);
    app.set_delete_running();

    assert!(matches!(
        app.delete_state(),
        DeleteState::Running { mode, .. } if matches!(mode, DeleteMode::Trash)
    ));
}

fn entry(name: &str, git_status: GitStatus, size_bytes: u64) -> BrowserEntry {
    BrowserEntry {
        path: PathBuf::from(format!("/workspace/repo/{name}")),
        name: name.to_string(),
        size_bytes,
        reclaimable_bytes: size_bytes,
        size_complete: true,
        entry_kind: EntryKind::Directory,
        git_status,
        git_context: GitContext::default(),
        risk_level: RiskLevel::Low,
        candidate_kind: None,
        is_visible_candidate: false,
    }
}

fn profile_with_command(root: PathBuf) -> ProjectProfile {
    ProjectProfile {
        root: root.clone(),
        kinds: vec![ProjectKind::Rust],
        languages: vec![artix::clean::LanguageSummary {
            name: "Rust".to_string(),
            bytes: 10,
            percent: 100,
        }],
        clean_plan: CleanPlan {
            project_root: root.clone(),
            commands: vec![CleanCommand {
                kind: ProjectKind::Rust,
                program: "cargo".to_string(),
                args: vec!["clean".to_string()],
                cwd: root,
            }],
        },
    }
}

fn clean_summary(root: PathBuf) -> CleanRunSummary {
    let command = CleanCommand {
        kind: ProjectKind::Rust,
        program: "cargo".to_string(),
        args: vec!["clean".to_string()],
        cwd: root.clone(),
    };
    CleanRunSummary {
        project_root: root,
        results: vec![CleanCommandResult {
            command,
            success: true,
            status_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        }],
    }
}
