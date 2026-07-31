use std::fs;
use std::path::{Path, PathBuf};

use crate::candidate::{classify_dir_name, descriptor_for};
use crate::classify::git::{classify_path_git_status, resolve_git_context};
use crate::config::AppContext;
use crate::model::{BrowserEntry, CandidateKind, EntryKind, GitContext, GitStatus, RiskLevel};

use super::size::{SizeMeasurement, measure_size};

#[derive(Debug, Clone)]
pub(crate) struct BrowserEntrySeed {
    path: PathBuf,
    name: String,
    is_file: bool,
    candidate_kind: Option<CandidateKind>,
}

impl BrowserEntrySeed {
    pub(crate) fn into_placeholder(self, current_context: Option<GitContext>) -> BrowserEntry {
        let git_context = self.git_context(current_context.as_ref());
        self.into_browser_entry(
            SizeMeasurement::complete(0),
            GitStatus::Unknown,
            git_context.unwrap_or_default(),
        )
    }

    pub(crate) async fn into_enriched(
        self,
        current_context: Option<GitContext>,
        ctx: &AppContext,
    ) -> BrowserEntry {
        let measurement = self.size(ctx).await;
        let git_context = self.git_context(current_context.as_ref());
        let git_status = classify_path_git_status(&self.path, git_context.as_ref(), ctx).await;

        self.into_browser_entry(measurement, git_status, git_context.unwrap_or_default())
    }

    async fn size(&self, ctx: &AppContext) -> SizeMeasurement {
        measure_size(&self.path, ctx).await
    }

    fn git_context(&self, current_context: Option<&GitContext>) -> Option<GitContext> {
        resolve_git_context(&self.path).or_else(|| current_context.cloned())
    }

    fn into_browser_entry(
        self,
        measurement: SizeMeasurement,
        git_status: GitStatus,
        git_context: GitContext,
    ) -> BrowserEntry {
        let size_bytes = measurement.bytes();
        let entry_kind = self.entry_kind();
        let risk_level = self
            .candidate_kind
            .map(|kind| descriptor_for(kind).default_risk.clone())
            .unwrap_or(RiskLevel::Hidden);

        BrowserEntry {
            path: self.path,
            name: self.name,
            size_bytes,
            reclaimable_bytes: size_bytes,
            size_status: measurement.status(),
            entry_kind,
            git_status,
            git_context,
            risk_level,
        }
    }

    fn entry_kind(&self) -> EntryKind {
        if self.is_file {
            EntryKind::File
        } else if let Some(kind) = self.candidate_kind {
            EntryKind::CleanupCandidate(kind)
        } else {
            EntryKind::Directory
        }
    }
}

pub(crate) fn parent_entry_for(path: &Path, root_dir: &Path) -> Option<BrowserEntry> {
    if path == root_dir {
        return None;
    }

    path.parent()
        .map(|parent| BrowserEntry::parent(parent.to_path_buf()))
}

pub(crate) fn read_browser_entry_seeds(path: &Path) -> Result<Vec<BrowserEntrySeed>, String> {
    let read_dir = fs::read_dir(path).map_err(|err| err.to_string())?;
    let mut seeds = Vec::new();

    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        let is_dir = entry_path.is_dir();
        let is_file = entry_path.is_file();
        if !is_dir && !is_file {
            continue;
        }

        let file_name = entry_path.file_name().unwrap_or_default();
        let name = file_name.to_string_lossy().into_owned();
        if is_dir && file_name == ".git" {
            continue;
        }

        let candidate_kind = if is_dir {
            classify_dir_name(file_name)
        } else {
            None
        };

        seeds.push(BrowserEntrySeed {
            path: entry_path,
            name,
            is_file,
            candidate_kind,
        });
    }

    Ok(seeds)
}

pub(crate) fn sort_browser_entries_by_size(entries: &mut Vec<BrowserEntry>) {
    let (parents, mut rest): (Vec<_>, Vec<_>) = entries
        .drain(..)
        .partition(|entry| matches!(entry.entry_kind, EntryKind::Parent));
    rest.sort_by(|left, right| {
        right
            .reclaimable_bytes
            .cmp(&left.reclaimable_bytes)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.path.cmp(&right.path))
    });
    entries.extend(parents.into_iter().chain(rest));
}

pub(crate) fn sort_placeholder_entries(entries: &mut Vec<BrowserEntry>) {
    let (parents, mut rest): (Vec<_>, Vec<_>) = entries
        .drain(..)
        .partition(|entry| matches!(entry.entry_kind, EntryKind::Parent));
    rest.sort_by(|left, right| {
        placeholder_sort_rank(left)
            .cmp(&placeholder_sort_rank(right))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.path.cmp(&right.path))
    });
    entries.extend(parents.into_iter().chain(rest));
}

pub(crate) fn apply_browser_entry_update(entries: &mut [BrowserEntry], update: &BrowserEntry) {
    for entry in entries {
        if entry.path == update.path {
            entry.size_bytes = update.size_bytes;
            entry.reclaimable_bytes = update.reclaimable_bytes;
            entry.size_status = update.size_status;
            entry.git_status = update.git_status.clone();
            entry.git_context = update.git_context.clone();
            entry.risk_level = update.risk_level.clone();
            entry.entry_kind = update.entry_kind;
            break;
        }
    }
}

fn placeholder_sort_rank(entry: &BrowserEntry) -> u8 {
    match entry.entry_kind {
        EntryKind::Parent => 0,
        EntryKind::CleanupCandidate(_) => 1,
        EntryKind::Directory | EntryKind::File => 2,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::model::{CandidateKind, EntryKind};

    use super::{
        parent_entry_for, read_browser_entry_seeds, sort_browser_entries_by_size,
        sort_placeholder_entries,
    };

    #[test]
    fn placeholder_entries_apply_shared_entry_shape() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join(".git")).expect("create git");
        fs::create_dir_all(root.join("target/debug")).expect("create target");
        fs::create_dir_all(root.join("src")).expect("create src");
        fs::write(root.join("large.log"), "x").expect("write file");

        let seeds = read_browser_entry_seeds(root).expect("seeds");
        let mut entries = seeds
            .into_iter()
            .map(|seed| seed.into_placeholder(None))
            .collect::<Vec<_>>();
        sort_placeholder_entries(&mut entries);

        let names = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.entry_kind))
            .collect::<Vec<_>>();

        assert!(!names.iter().any(|(name, _)| *name == ".git"));
        assert_eq!(
            names[0],
            (
                "target",
                EntryKind::CleanupCandidate(CandidateKind::RustTarget),
            )
        );
        assert!(names.contains(&("src", EntryKind::Directory)));
        assert!(names.contains(&("large.log", EntryKind::File)));
    }

    #[test]
    fn enriched_sort_keeps_parent_first_then_reclaimable_bytes() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        let child = root.join("child");
        fs::create_dir_all(&child).expect("create child");
        fs::create_dir_all(child.join("target")).expect("create target");
        fs::create_dir_all(child.join("src")).expect("create src");

        let mut entries = vec![parent_entry_for(&child, root).expect("parent")];
        entries.extend(
            read_browser_entry_seeds(&child)
                .expect("seeds")
                .into_iter()
                .map(|seed| seed.into_placeholder(None)),
        );
        for entry in &mut entries {
            if entry.name == "target" {
                entry.reclaimable_bytes = 10;
            } else if entry.name == "src" {
                entry.reclaimable_bytes = 20;
            }
        }

        sort_browser_entries_by_size(&mut entries);

        assert_eq!(entries[0].name, "..");
        assert_eq!(entries[1].name, "src");
        assert_eq!(entries[2].name, "target");
    }
}
