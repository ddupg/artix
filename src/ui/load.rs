use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::classify::git::resolve_git_context;
use crate::clean::ProjectProfile;
use crate::model::{BrowserEntry, EntryKind, GitContext};
use crate::scan::entry::{
    self as browser_entries, BrowserEntrySeed, apply_browser_entry_update,
    sort_browser_entries_by_size, sort_placeholder_entries,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LoadToken(u64);

#[derive(Debug)]
pub(super) struct DirectoryLoadRequest {
    token: LoadToken,
    dir: PathBuf,
    seeds: Vec<BrowserEntrySeed>,
    current_context: Option<GitContext>,
}

impl DirectoryLoadRequest {
    pub(super) fn into_parts(
        self,
    ) -> (
        LoadToken,
        PathBuf,
        Vec<BrowserEntrySeed>,
        Option<GitContext>,
    ) {
        (self.token, self.dir, self.seeds, self.current_context)
    }
}

#[derive(Debug)]
pub(super) enum WorkerCommand {
    Start(Box<DirectoryLoadRequest>),
    Cancel,
}

#[derive(Debug)]
pub(super) enum DirectoryLoadEvent {
    EntryUpdated {
        token: LoadToken,
        dir: PathBuf,
        entry: Box<BrowserEntry>,
    },
    EntriesFinished {
        token: LoadToken,
        dir: PathBuf,
    },
    ProfileFinished {
        token: LoadToken,
        dir: PathBuf,
        result: Result<Option<ProjectProfile>, String>,
    },
}

impl DirectoryLoadEvent {
    pub(super) fn dir(&self) -> &Path {
        match self {
            Self::EntryUpdated { dir, .. }
            | Self::EntriesFinished { dir, .. }
            | Self::ProfileFinished { dir, .. } => dir,
        }
    }

    fn token(&self) -> LoadToken {
        match self {
            Self::EntryUpdated { token, .. }
            | Self::EntriesFinished { token, .. }
            | Self::ProfileFinished { token, .. } => *token,
        }
    }
}

#[derive(Debug)]
pub(super) struct DirectoryLoads {
    next_token: u64,
    active: Option<ActiveLoad>,
    snapshots: HashMap<PathBuf, DirectorySnapshot>,
}

impl DirectoryLoads {
    pub(super) fn new() -> Self {
        Self {
            next_token: 1,
            active: None,
            snapshots: HashMap::new(),
        }
    }

    pub(super) fn open(&mut self, dir: PathBuf, root_dir: &Path) -> WorkerCommand {
        self.active = None;
        if self.is_complete(&dir) {
            return WorkerCommand::Cancel;
        }

        let current_context = resolve_git_context(&dir);
        let seeds = match browser_entries::read_browser_entry_seeds(&dir) {
            Ok(seeds) => seeds,
            Err(err) => {
                self.record_listing_failure(&dir, err);
                return WorkerCommand::Cancel;
            }
        };

        let mut entries = Vec::with_capacity(seeds.len().saturating_add(1));
        if let Some(parent) = browser_entries::parent_entry_for(&dir, root_dir) {
            entries.push(parent);
        }
        entries.extend(
            seeds
                .iter()
                .cloned()
                .map(|seed| seed.into_placeholder(current_context.clone())),
        );
        sort_placeholder_entries(&mut entries);

        let loading_paths = entries
            .iter()
            .filter(|entry| !matches!(entry.entry_kind, EntryKind::Parent))
            .map(|entry| entry.path.clone())
            .collect();
        let token = self.allocate_token();
        self.snapshots.insert(
            dir.clone(),
            DirectorySnapshot {
                entries,
                loading_paths,
                status: SnapshotStatus::Loading,
                profile: ProfileOutcome::Pending,
            },
        );
        self.active = Some(ActiveLoad {
            token,
            dir: dir.clone(),
            entries_finished: false,
            profile_finished: false,
        });

        WorkerCommand::Start(Box::new(DirectoryLoadRequest {
            token,
            dir,
            seeds,
            current_context,
        }))
    }

    pub(super) fn apply(&mut self, event: DirectoryLoadEvent) -> bool {
        let (active_dir, entries_finished, profile_finished) = {
            let Some(active) = self.active.as_ref() else {
                return false;
            };
            if active.token != event.token() || active.dir != event.dir() {
                return false;
            }
            (
                active.dir.clone(),
                active.entries_finished,
                active.profile_finished,
            )
        };

        match event {
            DirectoryLoadEvent::EntryUpdated { entry, .. } => {
                if entries_finished {
                    return false;
                }
                let snapshot = self
                    .snapshots
                    .get_mut(&active_dir)
                    .expect("active load owns a snapshot");
                if !snapshot.loading_paths.remove(&entry.path) {
                    return false;
                }
                apply_browser_entry_update(&mut snapshot.entries, &entry);
                sort_browser_entries_by_size(&mut snapshot.entries);
            }
            DirectoryLoadEvent::EntriesFinished { .. } => {
                if entries_finished {
                    return false;
                }
                self.active
                    .as_mut()
                    .expect("active load was checked")
                    .entries_finished = true;
                self.snapshots
                    .get_mut(&active_dir)
                    .expect("active load owns a snapshot")
                    .loading_paths
                    .clear();
            }
            DirectoryLoadEvent::ProfileFinished { result, .. } => {
                if profile_finished {
                    return false;
                }
                self.active
                    .as_mut()
                    .expect("active load was checked")
                    .profile_finished = true;
                self.snapshots
                    .get_mut(&active_dir)
                    .expect("active load owns a snapshot")
                    .profile = match result {
                    Ok(profile) => ProfileOutcome::Available(profile),
                    Err(message) => ProfileOutcome::Unavailable(message),
                };
            }
        }

        self.complete_if_ready();
        true
    }

    pub(super) fn entries(&self, dir: &Path) -> Option<&[BrowserEntry]> {
        self.snapshots
            .get(dir)
            .map(|snapshot| snapshot.entries.as_slice())
    }

    pub(super) fn is_complete(&self, dir: &Path) -> bool {
        self.snapshots
            .get(dir)
            .is_some_and(DirectorySnapshot::is_complete)
    }

    pub(super) fn profile(&self, dir: &Path) -> Option<&ProjectProfile> {
        let snapshot = self.snapshots.get(dir)?;
        if !snapshot.is_complete() {
            return None;
        }
        match &snapshot.profile {
            ProfileOutcome::Available(profile) => profile.as_ref(),
            ProfileOutcome::Pending
            | ProfileOutcome::NotLoaded
            | ProfileOutcome::Unavailable(_) => None,
        }
    }

    pub(super) fn loading_paths(&self, dir: &Path) -> Option<&HashSet<PathBuf>> {
        self.snapshots
            .get(dir)
            .map(|snapshot| &snapshot.loading_paths)
    }

    pub(super) fn load_error(&self, dir: &Path) -> Option<&str> {
        let snapshot = self.snapshots.get(dir)?;
        match &snapshot.status {
            SnapshotStatus::Failed(message) => Some(message),
            SnapshotStatus::Loading | SnapshotStatus::Complete => None,
        }
    }

    pub(super) fn profile_error(&self, dir: &Path) -> Option<&str> {
        let snapshot = self.snapshots.get(dir)?;
        if !snapshot.is_complete() {
            return None;
        }
        match &snapshot.profile {
            ProfileOutcome::Unavailable(message) => Some(message),
            ProfileOutcome::Pending | ProfileOutcome::NotLoaded | ProfileOutcome::Available(_) => {
                None
            }
        }
    }

    pub(super) fn invalidate_related(&mut self, path: &Path) {
        self.snapshots.retain(|cached_dir, _| {
            !(path.starts_with(cached_dir.as_path()) || cached_dir.starts_with(path))
        });
        if self.active.as_ref().is_some_and(|active| {
            path.starts_with(active.dir.as_path()) || active.dir.starts_with(path)
        }) {
            self.active = None;
        }
    }

    fn record_listing_failure(&mut self, dir: &Path, error: String) {
        let message = format!("failed to load {}: {error}", dir.display());
        let snapshot = self
            .snapshots
            .entry(dir.to_path_buf())
            .or_insert_with(DirectorySnapshot::empty);
        snapshot.loading_paths.clear();
        snapshot.status = SnapshotStatus::Failed(message);
        snapshot.profile = ProfileOutcome::NotLoaded;
    }

    fn complete_if_ready(&mut self) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if !active.entries_finished || !active.profile_finished {
            return;
        }

        self.snapshots
            .get_mut(&active.dir)
            .expect("active load owns a snapshot")
            .status = SnapshotStatus::Complete;
        self.active = None;
    }

    fn allocate_token(&mut self) -> LoadToken {
        let token = LoadToken(self.next_token);
        self.next_token = self.next_token.saturating_add(1);
        token
    }
}

#[derive(Debug)]
struct ActiveLoad {
    token: LoadToken,
    dir: PathBuf,
    entries_finished: bool,
    profile_finished: bool,
}

#[derive(Debug)]
struct DirectorySnapshot {
    entries: Vec<BrowserEntry>,
    loading_paths: HashSet<PathBuf>,
    status: SnapshotStatus,
    profile: ProfileOutcome,
}

impl DirectorySnapshot {
    fn empty() -> Self {
        Self {
            entries: Vec::new(),
            loading_paths: HashSet::new(),
            status: SnapshotStatus::Loading,
            profile: ProfileOutcome::NotLoaded,
        }
    }

    fn is_complete(&self) -> bool {
        matches!(self.status, SnapshotStatus::Complete)
    }
}

#[derive(Debug)]
enum SnapshotStatus {
    Loading,
    Complete,
    Failed(String),
}

#[derive(Debug)]
enum ProfileOutcome {
    Pending,
    NotLoaded,
    Available(Option<ProjectProfile>),
    Unavailable(String),
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::model::{CandidateKind, EntryKind, GitStatus};

    use super::{DirectoryLoadEvent, DirectoryLoads, LoadToken, WorkerCommand};

    #[test]
    fn incomplete_snapshot_starts_a_new_session_and_rejects_stale_events() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("target");
        let mut loads = DirectoryLoads::new();
        let first = start(&mut loads, temp.path(), temp.path());
        let second = start(&mut loads, temp.path(), temp.path());
        assert_ne!(first, second);

        assert!(!loads.apply(DirectoryLoadEvent::EntriesFinished {
            token: first,
            dir: temp.path().to_path_buf(),
        }));
        assert!(loads.apply(DirectoryLoadEvent::EntriesFinished {
            token: second,
            dir: temp.path().to_path_buf(),
        }));
    }

    #[test]
    fn open_builds_the_sorted_placeholder_snapshot() {
        let temp = tempdir().expect("tempdir");
        fs::create_dir(temp.path().join("target")).expect("target");
        fs::create_dir(temp.path().join("aaa")).expect("directory");
        fs::write(temp.path().join("mmm.log"), "artifact").expect("file");
        let mut loads = DirectoryLoads::new();

        start(&mut loads, temp.path(), temp.path());
        let names = loads
            .entries(temp.path())
            .expect("snapshot")
            .iter()
            .map(|entry| (entry.name.as_str(), entry.entry_kind))
            .collect::<Vec<_>>();

        assert_eq!(
            names[0],
            (
                "target",
                EntryKind::CleanupCandidate(CandidateKind::RustTarget),
            )
        );
        assert_eq!(names[1], ("aaa", EntryKind::Directory));
        assert_eq!(names[2], ("mmm.log", EntryKind::File));
    }

    #[test]
    fn entries_and_profile_must_both_finish_before_cache_is_complete() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("README.md"), "hello").expect("file");
        let mut loads = DirectoryLoads::new();
        let token = start(&mut loads, temp.path(), temp.path());

        assert!(loads.apply(DirectoryLoadEvent::EntriesFinished {
            token,
            dir: temp.path().to_path_buf(),
        }));
        assert!(!loads.is_complete(temp.path()));
        assert!(loads.apply(DirectoryLoadEvent::ProfileFinished {
            token,
            dir: temp.path().to_path_buf(),
            result: Ok(None),
        }));
        assert!(loads.is_complete(temp.path()));
        assert!(matches!(
            loads.open(temp.path().to_path_buf(), temp.path()),
            WorkerCommand::Cancel
        ));
    }

    #[test]
    fn listing_failure_is_visible_and_a_later_open_retries() {
        let temp = tempdir().expect("tempdir");
        let directory = temp.path().join("project");
        fs::create_dir(&directory).expect("project");
        fs::write(directory.join("README.md"), "cached").expect("file");
        let mut loads = DirectoryLoads::new();

        start(&mut loads, &directory, temp.path());
        assert_eq!(loads.entries(&directory).expect("snapshot").len(), 2);
        fs::remove_dir_all(&directory).expect("remove project");
        assert!(matches!(
            loads.open(directory.clone(), temp.path()),
            WorkerCommand::Cancel
        ));
        assert!(
            loads
                .load_error(&directory)
                .is_some_and(|message| message.contains("failed to load"))
        );
        assert_eq!(loads.entries(&directory).expect("stale snapshot").len(), 2);

        fs::create_dir(&directory).expect("recreate project");
        assert!(matches!(
            loads.open(directory.clone(), temp.path()),
            WorkerCommand::Start(_)
        ));
        assert!(loads.load_error(&directory).is_none());
    }

    #[test]
    fn profile_failure_is_non_fatal_and_preserved() {
        let temp = tempdir().expect("tempdir");
        let mut loads = DirectoryLoads::new();
        let token = start(&mut loads, temp.path(), temp.path());

        assert!(loads.apply(DirectoryLoadEvent::ProfileFinished {
            token,
            dir: temp.path().to_path_buf(),
            result: Err("language analysis failed".to_string()),
        }));
        assert!(loads.apply(DirectoryLoadEvent::EntriesFinished {
            token,
            dir: temp.path().to_path_buf(),
        }));

        assert_eq!(
            loads.profile_error(temp.path()),
            Some("language analysis failed")
        );
        assert!(loads.profile(temp.path()).is_none());
        assert!(matches!(
            loads.open(temp.path().to_path_buf(), temp.path()),
            WorkerCommand::Cancel
        ));
    }

    #[test]
    fn duplicate_and_out_of_order_events_fail_closed() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("README.md"), "hello").expect("file");
        let mut loads = DirectoryLoads::new();
        let token = start(&mut loads, temp.path(), temp.path());
        let mut entry = loads.entries(temp.path()).expect("snapshot")[0].clone();
        entry.size_bytes = 5;
        entry.reclaimable_bytes = 5;
        entry.git_status = GitStatus::Untracked;

        assert!(!loads.apply(DirectoryLoadEvent::EntriesFinished {
            token,
            dir: temp.path().join("other"),
        }));
        assert!(!loads.apply(DirectoryLoadEvent::EntryUpdated {
            token: LoadToken(token.0 + 1),
            dir: temp.path().to_path_buf(),
            entry: Box::new(entry.clone()),
        }));
        let mut unknown_entry = entry.clone();
        unknown_entry.path = temp.path().join("unknown");
        assert!(!loads.apply(DirectoryLoadEvent::EntryUpdated {
            token,
            dir: temp.path().to_path_buf(),
            entry: Box::new(unknown_entry),
        }));
        assert!(loads.apply(DirectoryLoadEvent::EntryUpdated {
            token,
            dir: temp.path().to_path_buf(),
            entry: Box::new(entry.clone()),
        }));
        assert!(!loads.apply(DirectoryLoadEvent::EntryUpdated {
            token,
            dir: temp.path().to_path_buf(),
            entry: Box::new(entry),
        }));

        assert!(loads.apply(DirectoryLoadEvent::EntriesFinished {
            token,
            dir: temp.path().to_path_buf(),
        }));
        assert!(!loads.apply(DirectoryLoadEvent::EntriesFinished {
            token,
            dir: temp.path().to_path_buf(),
        }));
        assert!(loads.apply(DirectoryLoadEvent::ProfileFinished {
            token,
            dir: temp.path().to_path_buf(),
            result: Ok(None),
        }));
        assert!(!loads.apply(DirectoryLoadEvent::ProfileFinished {
            token,
            dir: temp.path().to_path_buf(),
            result: Ok(None),
        }));
    }

    #[test]
    fn invalidation_removes_ancestor_and_descendant_snapshots() {
        let temp = tempdir().expect("tempdir");
        let child = temp.path().join("child");
        let sibling = temp.path().join("sibling");
        fs::create_dir(&child).expect("child");
        fs::create_dir(&sibling).expect("sibling");
        let mut loads = DirectoryLoads::new();
        start(&mut loads, temp.path(), temp.path());
        start(&mut loads, &child, temp.path());
        start(&mut loads, &sibling, temp.path());
        loads.invalidate_related(&child);

        assert!(loads.entries(temp.path()).is_none());
        assert!(loads.entries(&child).is_none());
        assert!(loads.entries(&sibling).is_some());
    }

    fn start(
        loads: &mut DirectoryLoads,
        dir: &std::path::Path,
        root: &std::path::Path,
    ) -> LoadToken {
        let WorkerCommand::Start(request) = loads.open(dir.to_path_buf(), root) else {
            panic!("expected load request");
        };
        request.token
    }
}
