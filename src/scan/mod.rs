pub mod discover;
pub mod size;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::classify::git::{classify_git_status, classify_path_git_status, resolve_git_context};
use crate::classify::ownership::{infer_project_roots, resolve_owner_project};
use crate::classify::risk::classify_risk_level;
use crate::model::{BrowserEntry, CandidateDir, EntryKind, Project, RiskLevel};
use crate::rules::{Rule, default_rules};
use discover::discover_candidates;
use size::dir_size_bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub candidates: Vec<CandidateDir>,
    pub projects: Vec<Project>,
}

pub fn browse_directory(path: &Path, root_dir: &Path) -> Result<Vec<BrowserEntry>, String> {
    let rules = default_rules();
    let current_context = resolve_git_context(path);
    let mut entries = Vec::new();

    // Only add ".." if we're not at the root directory (the starting directory)
    if path != root_dir {
        if let Some(parent) = path.parent() {
            entries.push(BrowserEntry::parent(parent.to_path_buf()));
        }
    }

    let read_dir = fs::read_dir(path).map_err(|err| err.to_string())?;
    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let name = entry_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string();
        if name == ".git" {
            continue;
        }
        let candidate_rule = rules.iter().find(|rule| rule.dir_name == name);
        let size_bytes = dir_size_bytes(&entry_path);
        let git_context = resolve_git_context(&entry_path).or_else(|| current_context.clone());
        let git_status = classify_path_git_status(&entry_path, git_context.as_ref());
        let risk_level = candidate_rule
            .map(|rule| classify_risk_level(rule, &git_status))
            .unwrap_or(RiskLevel::Hidden);
        let entry_kind = if candidate_rule.is_some() {
            EntryKind::CleanupCandidate
        } else {
            EntryKind::Directory
        };

        entries.push(BrowserEntry {
            path: entry_path,
            name,
            size_bytes,
            reclaimable_bytes: size_bytes,
            entry_kind,
            git_status,
            git_context: git_context.unwrap_or_default(),
            risk_level,
            candidate_kind: candidate_rule.map(|rule| rule.kind.to_string()),
            is_visible_candidate: candidate_rule.is_some(),
        });
    }

    let (parents, mut rest): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|entry| matches!(entry.entry_kind, EntryKind::Parent));
    rest.sort_by(|left, right| {
        right
            .reclaimable_bytes
            .cmp(&left.reclaimable_bytes)
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(parents.into_iter().chain(rest).collect())
}

pub fn scan_workspace(roots: &[PathBuf]) -> ScanReport {
    let rules = default_rules();
    let ownership_markers = collect_ownership_markers(roots);
    let project_roots = infer_project_roots(&ownership_markers);

    let mut candidates = Vec::new();

    for discovered in discover_candidates(roots, &rules) {
        let project_root = resolve_owner_project(&discovered.path, &project_roots)
            .or_else(|| {
                roots
                    .iter()
                    .filter(|root| discovered.path.starts_with(root))
                    .max_by_key(|root| root.components().count())
                    .cloned()
            })
            .unwrap_or_else(|| discovered.path.clone());
        let git_status = classify_git_status(&discovered.path, &project_root, &discovered.rule);
        let risk_level = classify_risk_level(&discovered.rule, &git_status);

        candidates.push(CandidateDir {
            path: discovered.path.clone(),
            project_root,
            kind: discovered.rule.kind.to_string(),
            size_bytes: dir_size_bytes(&discovered.path),
            git_status,
            risk_level,
            last_modified_epoch_secs: None,
            rule_id: discovered.rule.id.to_string(),
        });
    }

    let projects = summarize_projects(&candidates, &rules);

    ScanReport {
        candidates,
        projects,
    }
}

fn collect_ownership_markers(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut markers = Vec::new();

    for root in roots {
        collect_ownership_markers_from_path(root, &mut markers);
    }

    markers
}

fn collect_ownership_markers_from_path(path: &Path, markers: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();

        if entry_path.is_file() {
            let Some(file_name) = entry_path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            if matches!(file_name, "Cargo.toml" | "package.json" | "pyproject.toml") {
                markers.push(entry_path);
            }

            continue;
        }

        if entry_path.is_dir() {
            collect_ownership_markers_from_path(&entry_path, markers);
        }
    }
}

fn summarize_projects(candidates: &[CandidateDir], rules: &[Rule]) -> Vec<Project> {
    let mut projects = BTreeMap::<PathBuf, Project>::new();

    for candidate in candidates {
        let language_hint = rules
            .iter()
            .find(|rule| rule.id == candidate.rule_id)
            .map(|rule| rule.language_hint.to_string());
        let project = projects
            .entry(candidate.project_root.clone())
            .or_insert_with(|| Project {
                root: candidate.project_root.clone(),
                name: candidate
                    .project_root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("workspace")
                    .to_string(),
                language_hint,
                reclaimable_bytes: 0,
                candidate_count: 0,
            });

        project.reclaimable_bytes += candidate.size_bytes;
        project.candidate_count += 1;
    }

    projects.into_values().collect()
}
