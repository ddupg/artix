use artix::model::{CandidateDir, GitStatus, Project, RiskLevel};
use artix::rules::default_rules;

#[test]
fn candidate_dir_exposes_expected_fields() {
    let candidate = CandidateDir {
        path: "/tmp/ws/demo/target".into(),
        project_root: "/tmp/ws/demo".into(),
        kind: "rust-target".into(),
        size_bytes: 1024,
        git_status: GitStatus::Unknown,
        risk_level: RiskLevel::Low,
        last_modified_epoch_secs: Some(1),
        rule_id: "rust.target".into(),
    };

    assert_eq!(candidate.kind, "rust-target");
    assert_eq!(candidate.risk_level, RiskLevel::Low);
    assert_eq!(candidate.project_root.to_string_lossy(), "/tmp/ws/demo");
}

#[test]
fn project_tracks_reclaimable_bytes_and_candidates() {
    let project = Project {
        root: "/tmp/ws/demo".into(),
        name: "demo".into(),
        language_hint: Some("rust".into()),
        reclaimable_bytes: 4096,
        candidate_count: 2,
    };

    assert_eq!(project.name, "demo");
    assert_eq!(project.reclaimable_bytes, 4096);
    assert_eq!(project.candidate_count, 2);
}

// Task 2: rule table coverage.
#[test]
fn default_rules_include_core_ids_and_metadata() {
    let rules = default_rules();
    assert_eq!(
        rules
            .iter()
            .map(|rule| (rule.id, rule.kind, rule.language_hint))
            .collect::<Vec<_>>(),
        vec![
            ("rust.target", "rust-target", "rust"),
            ("node.node_modules", "node-modules", "node"),
            ("python.venv", "python-venv", "python"),
        ],
    );
}

#[test]
fn node_modules_rule_uses_expected_defaults() {
    let rule = default_rules()
        .into_iter()
        .find(|rule| rule.id == "node.node_modules")
        .expect("node_modules rule");

    assert_eq!(rule.default_risk, RiskLevel::Medium);
    assert!(rule.expect_git_ignored);
    assert_eq!(rule.dir_name, "node_modules");
    assert_eq!(rule.kind, "node-modules");
    assert_eq!(rule.language_hint, "node");
}

#[test]
fn python_venv_rule_keeps_expected_visibility_and_risk_defaults() {
    let rule = default_rules()
        .into_iter()
        .find(|rule| rule.id == "python.venv")
        .expect("python.venv rule");

    assert_eq!(rule.default_risk, RiskLevel::Medium);
    assert!(rule.expect_git_ignored);
    assert_eq!(rule.dir_name, ".venv");
    assert_eq!(rule.kind, "python-venv");
    assert_eq!(rule.language_hint, "python");
}
