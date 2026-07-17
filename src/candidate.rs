use std::ffi::OsStr;

use crate::model::{CandidateKind, RiskLevel};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDescriptor {
    pub kind: CandidateKind,
    pub label: &'static str,
    pub dir_name: &'static str,
    pub language_hint: &'static str,
    pub default_risk: RiskLevel,
}

const CANDIDATES: [CandidateDescriptor; 3] = [
    CandidateDescriptor {
        kind: CandidateKind::RustTarget,
        label: "rust-target",
        dir_name: "target",
        language_hint: "rust",
        default_risk: RiskLevel::Low,
    },
    CandidateDescriptor {
        kind: CandidateKind::NodeModules,
        label: "node-modules",
        dir_name: "node_modules",
        language_hint: "node",
        default_risk: RiskLevel::Medium,
    },
    CandidateDescriptor {
        kind: CandidateKind::PythonVenv,
        label: "python-venv",
        dir_name: ".venv",
        language_hint: "python",
        default_risk: RiskLevel::Medium,
    },
];

pub fn descriptors() -> &'static [CandidateDescriptor] {
    &CANDIDATES
}

pub fn classify_dir_name(name: &OsStr) -> Option<CandidateKind> {
    descriptors()
        .iter()
        .find(|candidate| name == candidate.dir_name)
        .map(|candidate| candidate.kind)
}

pub fn descriptor_for(kind: CandidateKind) -> &'static CandidateDescriptor {
    match kind {
        CandidateKind::RustTarget => &CANDIDATES[0],
        CandidateKind::NodeModules => &CANDIDATES[1],
        CandidateKind::PythonVenv => &CANDIDATES[2],
    }
}
