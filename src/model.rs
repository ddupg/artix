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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum HeadState {
    Branch,
    Detached,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitContext {
    pub repo_root: Option<PathBuf>,
    pub git_dir: Option<PathBuf>,
    pub common_dir: Option<PathBuf>,
    pub worktree_root: Option<PathBuf>,
    pub branch_name: Option<String>,
    pub head_ref: Option<String>,
    pub head_state: HeadState,
    pub is_worktree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Parent,
    Directory,
    CleanupCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserEntry {
    pub path: PathBuf,
    pub name: String,
    pub size_bytes: u64,
    pub reclaimable_bytes: u64,
    pub entry_kind: EntryKind,
    pub git_status: GitStatus,
    pub git_context: GitContext,
    pub risk_level: RiskLevel,
    pub candidate_kind: Option<String>,
    pub is_visible_candidate: bool,
}

impl BrowserEntry {
    pub fn parent(path: PathBuf) -> Self {
        Self {
            path,
            name: "..".into(),
            size_bytes: 0,
            reclaimable_bytes: 0,
            entry_kind: EntryKind::Parent,
            git_status: GitStatus::Unknown,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Hidden,
            candidate_kind: None,
            is_visible_candidate: false,
        }
    }
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
