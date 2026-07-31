use std::path::PathBuf;

use artix::delete::DeleteMode;
use artix::delete_flow::{DeleteFlow, DeleteState};
use artix::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel, SizeStatus};

#[test]
fn cleanup_candidate_permanent_delete_runs_after_one_confirmation() {
    let mut flow = DeleteFlow::default();
    flow.request(entry("target", GitStatus::Ignored));

    let request = flow
        .confirm_permanent()
        .expect("cleanup candidate should be authorized");
    let (entry, mode) = request.into_parts();

    assert_eq!(entry.name, "target");
    assert_eq!(mode, DeleteMode::Permanent { confirmed: true });
    assert!(matches!(flow.state(), DeleteState::Running { .. }));
    assert!(flow.finish(Ok("deleted".to_string())));
    assert_eq!(flow.state(), &DeleteState::Idle);
    assert!(!flow.finish(Ok("duplicate completion".to_string())));
}

#[test]
fn tracked_and_unknown_permanent_delete_require_two_confirmations() {
    for git_status in [GitStatus::Tracked, GitStatus::Unknown] {
        let mut flow = DeleteFlow::default();
        flow.request(entry("sensitive", git_status));

        assert!(flow.confirm_permanent().is_none());
        assert!(matches!(
            flow.state(),
            DeleteState::AwaitingExtraConfirmation { .. }
        ));

        let request = flow
            .confirm_permanent()
            .expect("second confirmation should authorize permanent delete");
        let (_, mode) = request.into_parts();
        assert_eq!(mode, DeleteMode::Permanent { confirmed: true });
        assert!(matches!(flow.state(), DeleteState::Running { .. }));
    }
}

#[test]
fn tracked_target_can_move_to_trash_without_extra_confirmation() {
    let mut flow = DeleteFlow::default();
    flow.request(entry("src", GitStatus::Tracked));

    let request = flow
        .choose_trash()
        .expect("trash should be authorized after the first confirmation");
    let (_, mode) = request.into_parts();

    assert_eq!(mode, DeleteMode::Trash);
    assert!(matches!(flow.state(), DeleteState::Running { .. }));
}

#[test]
fn execution_failure_stays_visible_until_dismissed() {
    let mut flow = DeleteFlow::default();
    flow.request(entry("target", GitStatus::Ignored));
    flow.choose_trash().expect("authorized trash request");

    assert!(!flow.finish(Err("trash failed".to_string())));
    assert_eq!(
        flow.state(),
        &DeleteState::Failed {
            message: "trash failed".to_string(),
        }
    );

    flow.cancel();
    assert_eq!(flow.state(), &DeleteState::Idle);
}

#[test]
fn cancellation_and_out_of_order_actions_fail_closed() {
    let mut flow = DeleteFlow::default();

    assert!(flow.choose_trash().is_none());
    assert!(flow.confirm_permanent().is_none());
    assert!(!flow.finish(Ok("unexpected".to_string())));
    assert_eq!(flow.state(), &DeleteState::Idle);

    flow.request(entry("target", GitStatus::Ignored));
    flow.request(entry("other", GitStatus::Tracked));
    assert!(matches!(
        flow.state(),
        DeleteState::Confirming { entry, .. } if entry.name == "target"
    ));
    flow.cancel();
    assert_eq!(flow.state(), &DeleteState::Idle);

    flow.request(entry("src", GitStatus::Tracked));
    assert!(flow.confirm_permanent().is_none());
    flow.cancel();
    assert_eq!(flow.state(), &DeleteState::Idle);

    flow.request(entry("target", GitStatus::Ignored));
    flow.choose_trash().expect("authorized trash request");
    flow.request(entry("other", GitStatus::Ignored));
    assert!(flow.choose_trash().is_none());
    assert!(flow.confirm_permanent().is_none());
    flow.cancel();
    assert!(matches!(flow.state(), DeleteState::Running { entry, .. } if entry.name == "target"));
}

fn entry(name: &str, git_status: GitStatus) -> BrowserEntry {
    BrowserEntry {
        path: PathBuf::from(format!("/workspace/repo/{name}")),
        name: name.to_string(),
        size_bytes: 1,
        reclaimable_bytes: 1,
        size_status: SizeStatus::Complete,
        entry_kind: EntryKind::Directory,
        git_status,
        git_context: GitContext::default(),
        risk_level: RiskLevel::Low,
    }
}
