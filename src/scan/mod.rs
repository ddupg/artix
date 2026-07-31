mod discovery;
pub(crate) mod entry;
pub mod size;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::candidate::descriptor_for;
use crate::classify::git::{classify_path_git_status, resolve_git_context};
use crate::config::AppContext;
use crate::model::{BrowserEntry, CandidateDir, Project};
use crate::project::project_language_hint;
use discovery::discover_workspace;
use entry::{parent_entry_for, read_browser_entry_seeds, sort_browser_entries_by_size};
use tokio::task::JoinSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanReport {
    pub candidates: Vec<CandidateDir>,
    pub projects: Vec<Project>,
}

pub async fn browse_directory(path: &Path, root_dir: &Path) -> Result<Vec<BrowserEntry>, String> {
    let ctx = AppContext::default();
    browse_directory_with_context(path, root_dir, &ctx).await
}

pub async fn browse_directory_with_context(
    path: &Path,
    root_dir: &Path,
    ctx: &AppContext,
) -> Result<Vec<BrowserEntry>, String> {
    let current_context = resolve_git_context(path);
    let mut entries = Vec::new();

    if let Some(parent) = parent_entry_for(path, root_dir) {
        entries.push(parent);
    }

    let mut jobs = JoinSet::new();
    for seed in read_browser_entry_seeds(path)? {
        let current_context = current_context.clone();
        let ctx = ctx.clone();
        jobs.spawn(async move { seed.into_enriched(current_context, &ctx).await });
    }

    while let Some(res) = jobs.join_next().await {
        let entry = res.map_err(|err| format!("browser entry enrichment failed: {err}"))?;
        entries.push(entry);
    }

    sort_browser_entries_by_size(&mut entries);

    Ok(entries)
}

pub async fn scan_workspace(roots: &[PathBuf]) -> ScanReport {
    let ctx = AppContext::default();
    scan_workspace_with_context(roots, &ctx).await
}

pub async fn scan_workspace_with_context(roots: &[PathBuf], ctx: &AppContext) -> ScanReport {
    let roots_cloned = roots.to_vec();
    let discovered = tokio::task::spawn_blocking(move || discover_workspace(&roots_cloned))
        .await
        .unwrap_or_default();

    let fs_sem = ctx.fs_semaphore();

    let mut handles = Vec::with_capacity(discovered.len());
    for (idx, discovered) in discovered.into_iter().enumerate() {
        let descriptor = descriptor_for(discovered.kind);
        let fallback = CandidateDir {
            path: discovered.path.clone(),
            project_root: discovered.project_root.clone(),
            kind: discovered.kind,
            size_bytes: 0,
            size_status: crate::model::SizeStatus::Incomplete,
            git_status: crate::model::GitStatus::Unknown,
            risk_level: descriptor.default_risk.clone(),
            last_modified_epoch_secs: None,
        };
        let fs_sem = fs_sem.clone();
        let config = ctx.config().clone();
        let ctx = ctx.clone();
        let task_candidate = fallback.clone();
        let handle = tokio::spawn(async move {
            let git_context = resolve_git_context(&task_candidate.path)
                .or_else(|| resolve_git_context(&task_candidate.project_root));
            let git_status =
                classify_path_git_status(&task_candidate.path, git_context.as_ref(), &ctx).await;

            let _permit = fs_sem
                .acquire()
                .await
                .expect("semaphore must not be closed");
            let size_path = task_candidate.path.clone();
            let measurement = tokio::task::spawn_blocking(move || {
                crate::scan::size::measure_path_sync_with_config(&size_path, &config)
            })
            .await
            .unwrap_or_else(|_| crate::scan::size::SizeMeasurement::incomplete(0));

            CandidateDir {
                size_bytes: measurement.bytes(),
                size_status: measurement.status(),
                git_status,
                ..task_candidate
            }
        });
        handles.push((idx, fallback, handle));
    }

    let mut candidates_with_idx = Vec::new();
    for (idx, fallback, handle) in handles {
        let candidate = handle.await.unwrap_or(fallback);
        candidates_with_idx.push((idx, candidate));
    }
    candidates_with_idx.sort_by_key(|(idx, _)| *idx);
    let candidates = candidates_with_idx
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect::<Vec<_>>();

    let projects = summarize_projects(&candidates);

    ScanReport {
        candidates,
        projects,
    }
}

fn summarize_projects(candidates: &[CandidateDir]) -> Vec<Project> {
    let mut projects = BTreeMap::<PathBuf, Project>::new();

    for candidate in candidates {
        let language_hint = Some(
            project_language_hint(&candidate.project_root)
                .unwrap_or(descriptor_for(candidate.kind).language_hint)
                .to_string(),
        );
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
