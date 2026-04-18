pub mod discover;
pub mod size;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::classify::git::{classify_git_status, classify_path_git_status, resolve_git_context};
use crate::classify::ownership::{infer_project_roots, resolve_owner_project};
use crate::classify::risk::classify_risk_level;
use crate::model::{BrowserEntry, CandidateDir, EntryKind, Project, RiskLevel};
use crate::rules::{Rule, default_rules};
use discover::discover_candidates;
use size::dir_size_bytes;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub candidates: Vec<CandidateDir>,
    pub projects: Vec<Project>,
}

pub async fn browse_directory(path: &Path, root_dir: &Path) -> Result<Vec<BrowserEntry>, String> {
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
    let mut jobs = JoinSet::new();
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

        let candidate_rule = rules.iter().find(|rule| rule.dir_name == name).cloned();
        let current_context = current_context.clone();

        jobs.spawn(async move {
            let entry_kind = if candidate_rule.is_some() {
                EntryKind::CleanupCandidate
            } else {
                EntryKind::Directory
            };

            let size_bytes = dir_size_bytes(&entry_path).await;

            let git_context = resolve_git_context(&entry_path).or_else(|| current_context);
            let git_status = classify_path_git_status(&entry_path, git_context.as_ref()).await;
            let risk_level = candidate_rule
                .as_ref()
                .map(|rule| classify_risk_level(rule, &git_status))
                .unwrap_or(RiskLevel::Hidden);

            let is_visible_candidate = matches!(entry_kind, EntryKind::CleanupCandidate);

            BrowserEntry {
                path: entry_path,
                name,
                size_bytes,
                reclaimable_bytes: size_bytes,
                size_complete: true,
                entry_kind,
                git_status,
                git_context: git_context.unwrap_or_default(),
                risk_level,
                candidate_kind: candidate_rule.map(|rule| rule.kind.to_string()),
                is_visible_candidate,
            }
        });
    }

    while let Some(res) = jobs.join_next().await {
        if let Ok(entry) = res {
            entries.push(entry);
        }
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

pub async fn scan_workspace(roots: &[PathBuf]) -> ScanReport {
    let rules = default_rules();
    let roots_cloned = roots.to_vec();
    let ownership_markers =
        tokio::task::spawn_blocking(move || collect_ownership_markers(&roots_cloned))
            .await
            .unwrap_or_default();
    let project_roots = infer_project_roots(&ownership_markers);

    let rules_for_discover = rules.clone();
    let roots_cloned = roots.to_vec();
    let discovered = tokio::task::spawn_blocking(move || {
        discover_candidates(&roots_cloned, &rules_for_discover)
    })
    .await
    .unwrap_or_default();

    let fs_limit = fs_concurrency_limit();
    let fs_sem = std::sync::Arc::new(Semaphore::new(fs_limit));

    let roots = std::sync::Arc::new(roots.to_vec());
    let project_roots = std::sync::Arc::new(project_roots);

    let mut handles = Vec::with_capacity(discovered.len());
    for (idx, discovered) in discovered.into_iter().enumerate() {
        let fs_sem = fs_sem.clone();
        let project_roots = project_roots.clone();
        let roots = roots.clone();
        handles.push(tokio::spawn(async move {
            let _permit = fs_sem
                .acquire()
                .await
                .expect("semaphore must not be closed");

            let project_root = resolve_owner_project(&discovered.path, project_roots.as_ref())
                .or_else(|| {
                    roots
                        .iter()
                        .filter(|root| discovered.path.starts_with(root))
                        .max_by_key(|root| root.components().count())
                        .cloned()
                })
                .unwrap_or_else(|| discovered.path.clone());

            let path = discovered.path.clone();
            let rule = discovered.rule.clone();

            let candidate = tokio::task::spawn_blocking(move || {
                let git_status = classify_git_status(&path, &project_root, &rule);
                let risk_level = classify_risk_level(&rule, &git_status);
                let size_bytes = crate::scan::size::dir_size_bytes_sync(&path);

                CandidateDir {
                    path,
                    project_root,
                    kind: rule.kind.to_string(),
                    size_bytes,
                    git_status,
                    risk_level,
                    last_modified_epoch_secs: None,
                    rule_id: rule.id.to_string(),
                }
            })
            .await
            .map_err(|err| err.to_string());

            (idx, candidate)
        }));
    }

    let mut candidates_with_idx = Vec::new();
    for handle in handles {
        if let Ok((idx, Ok(candidate))) = handle.await {
            candidates_with_idx.push((idx, candidate));
        }
    }
    candidates_with_idx.sort_by_key(|(idx, _)| *idx);
    let candidates = candidates_with_idx
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect::<Vec<_>>();

    let projects = summarize_projects(&candidates, &rules);

    ScanReport {
        candidates,
        projects,
    }
}

fn fs_concurrency_limit() -> usize {
    let default = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let default = (default.saturating_mul(2)).clamp(2, 16);
    env::var("ARTIX_FS_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
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
