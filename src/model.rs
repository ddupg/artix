use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatus {
    Ignored,
    Untracked,
    Tracked,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDir {
    pub path: PathBuf,
    pub project_root: PathBuf,
    pub kind: String,
    pub size_bytes: u64,
    pub git_status: GitStatus,
    pub risk_level: RiskLevel,
    pub last_modified_epoch_secs: Option<u64>,
    pub rule_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub language_hint: Option<String>,
    pub reclaimable_bytes: u64,
    pub candidate_count: usize,
}
