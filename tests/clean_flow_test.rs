use std::path::PathBuf;

use artix::clean::{CleanCommand, CleanCommandResult, CleanPlan, CleanRunSummary, ProjectKind};
use artix::clean_flow::{CleanFlow, CleanState};

#[test]
fn empty_plan_cannot_enter_confirmation() {
    let mut flow = CleanFlow::default();
    flow.request(CleanPlan {
        project_root: "/workspace/repo".into(),
        commands: Vec::new(),
    });

    assert_eq!(flow.state(), &CleanState::Idle);
    assert!(flow.confirm().is_none());
}

#[test]
fn successful_completion_closes_immediately_and_returns_refresh_root() {
    let root = PathBuf::from("/workspace/repo");
    let plan = clean_plan(root.clone());
    let mut flow = CleanFlow::default();
    flow.request(plan.clone());
    let request = flow.confirm().expect("clean request");

    assert_eq!(request.into_plan(), plan.clone());
    assert!(matches!(flow.state(), CleanState::Running { .. }));

    let refresh_root = flow
        .finish(clean_summary(plan, true))
        .expect("matching completion");

    assert_eq!(refresh_root, root);
    assert_eq!(flow.state(), &CleanState::Idle);
}

#[test]
fn failure_stays_visible_until_dismissed_and_still_returns_refresh_root() {
    let root = PathBuf::from("/workspace/repo");
    let plan = clean_plan(root.clone());
    let mut flow = CleanFlow::default();
    flow.request(plan.clone());
    flow.confirm().expect("clean request");

    let refresh_root = flow
        .finish(clean_summary(plan, false))
        .expect("matching completion");

    assert_eq!(refresh_root, root);
    assert!(matches!(
        flow.state(),
        CleanState::Failed { message }
            if message.contains("cargo clean") && message.contains("permission denied")
    ));

    flow.cancel();
    assert_eq!(flow.state(), &CleanState::Idle);
}

#[test]
fn completion_for_another_project_fails_closed() {
    let plan = clean_plan("/workspace/repo-a".into());
    let mut flow = CleanFlow::default();
    flow.request(plan);
    flow.confirm().expect("clean request");

    let other = clean_plan("/workspace/repo-b".into());
    assert!(flow.finish(clean_summary(other, true)).is_none());
    assert!(matches!(flow.state(), CleanState::Running { .. }));
}

fn clean_plan(project_root: PathBuf) -> CleanPlan {
    CleanPlan {
        project_root: project_root.clone(),
        commands: vec![CleanCommand {
            kind: ProjectKind::Rust,
            program: "cargo".to_string(),
            args: vec!["clean".to_string()],
            cwd: project_root,
        }],
    }
}

fn clean_summary(plan: CleanPlan, success: bool) -> CleanRunSummary {
    CleanRunSummary {
        project_root: plan.project_root,
        results: plan
            .commands
            .into_iter()
            .map(|command| CleanCommandResult {
                command,
                success,
                status_code: success.then_some(0),
                stdout: String::new(),
                stderr: if success {
                    String::new()
                } else {
                    "permission denied".to_string()
                },
            })
            .collect(),
    }
}
