pub mod discover;
pub mod size;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::classify::git::classify_git_status;
use crate::classify::ownership::{infer_project_roots, resolve_owner_project};
use crate::classify::risk::classify_risk_level;
use crate::model::{CandidateDir, Project};
use crate::rules::{Rule, default_rules};
use discover::discover_candidates;
use size::dir_size_bytes;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub candidates: Vec<CandidateDir>,
    pub projects: Vec<Project>,
}

pub fn scan_workspace(roots: &[PathBuf]) -> ScanReport {
    let rules = default_rules();
    let ownership_markers = collect_ownership_markers(roots);
    let project_roots = infer_project_roots(&ownership_markers);

    let mut candidates = Vec::new();

    for discovered in discover_candidates(roots, &rules) {
        let project_root = resolve_owner_project(&discovered.path, &project_roots)
            .or_else(|| {
                roots.iter()
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

    ScanReport { candidates, projects }
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
