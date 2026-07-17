use std::ffi::OsStr;

use artix::candidate::{classify_dir_name, descriptor_for, descriptors};
use artix::model::{CandidateDir, CandidateKind, EntryKind, GitStatus, Project, RiskLevel};

#[test]
fn candidate_dir_uses_typed_identity() {
    let candidate = CandidateDir {
        path: "/tmp/ws/demo/target".into(),
        project_root: "/tmp/ws/demo".into(),
        kind: CandidateKind::RustTarget,
        size_bytes: 1024,
        git_status: GitStatus::Unknown,
        risk_level: RiskLevel::Low,
        last_modified_epoch_secs: Some(1),
    };

    assert_eq!(candidate.kind, CandidateKind::RustTarget);
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

#[test]
fn catalog_contains_every_candidate_descriptor() {
    assert_eq!(
        descriptors()
            .iter()
            .map(|candidate| {
                (
                    candidate.kind,
                    candidate.label,
                    candidate.dir_name,
                    candidate.language_hint,
                    candidate.default_risk.clone(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                CandidateKind::RustTarget,
                "rust-target",
                "target",
                "rust",
                RiskLevel::Low,
            ),
            (
                CandidateKind::NodeModules,
                "node-modules",
                "node_modules",
                "node",
                RiskLevel::Medium,
            ),
            (
                CandidateKind::PythonVenv,
                "python-venv",
                ".venv",
                "python",
                RiskLevel::Medium,
            ),
        ]
    );
}

#[test]
fn directory_classification_and_descriptor_lookup_round_trip() {
    for descriptor in descriptors() {
        let kind = classify_dir_name(OsStr::new(descriptor.dir_name))
            .expect("catalog directory name should classify");
        assert_eq!(kind, descriptor.kind);
        assert_eq!(descriptor_for(kind), descriptor);
    }

    assert_eq!(classify_dir_name(OsStr::new("src")), None);
}

#[test]
fn entry_kind_is_the_single_source_of_candidate_identity() {
    let entry_kind = EntryKind::CleanupCandidate(CandidateKind::NodeModules);

    assert_eq!(
        entry_kind.candidate_kind(),
        Some(CandidateKind::NodeModules)
    );
    assert_eq!(EntryKind::Directory.candidate_kind(), None);
}
