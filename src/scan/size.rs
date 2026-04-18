use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::{AppContext, Config, SizeBudgetConfig, SizeTraversalOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirSize {
    pub bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone, Copy)]
struct SizeBudget {
    remaining_entries: Option<u64>,
    deadline: Option<Instant>,
}

impl SizeBudget {
    fn from_config(config: SizeBudgetConfig) -> Self {
        Self {
            remaining_entries: config.max_entries,
            deadline: config
                .timeout_ms
                .map(|ms| Instant::now() + Duration::from_millis(ms)),
        }
    }

    fn step(&mut self) -> bool {
        if let Some(deadline) = self.deadline
            && Instant::now() > deadline
        {
            return false;
        }

        if let Some(remaining) = &mut self.remaining_entries {
            if *remaining == 0 {
                return false;
            }
            *remaining = remaining.saturating_sub(1);
        }

        true
    }
}

#[cfg(unix)]
fn dir_key(meta: &fs::Metadata) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    Some((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn dir_key(_meta: &fs::Metadata) -> Option<(u64, u64)> {
    None
}

fn dir_size_bytes_sync_inner(
    path: &Path,
    traversal: SizeTraversalOptions,
    visited_dirs: &mut HashSet<(u64, u64)>,
    budget: &mut Option<SizeBudget>,
) -> DirSize {
    let Ok(entries) = fs::read_dir(path) else {
        return DirSize {
            bytes: 0,
            complete: true,
        };
    };

    let mut total = 0u64;

    for entry in entries.flatten() {
        if let Some(budget) = budget.as_mut()
            && !budget.step()
        {
            return DirSize {
                bytes: total,
                complete: false,
            };
        }

        let entry_path = entry.path();

        let meta_link = match fs::symlink_metadata(&entry_path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let is_symlink = meta_link.file_type().is_symlink();

        if is_symlink && !traversal.follow_symlinks {
            total = total.saturating_add(meta_link.len());
            continue;
        }

        let meta = if is_symlink && traversal.follow_symlinks {
            match fs::metadata(&entry_path) {
                Ok(meta) => meta,
                Err(_) => {
                    total = total.saturating_add(meta_link.len());
                    continue;
                }
            }
        } else {
            meta_link
        };

        if meta.file_type().is_dir() {
            if traversal.dedup_dir_inodes
                && let Some(key) = dir_key(&meta)
                && !visited_dirs.insert(key)
            {
                continue;
            }

            let sub = dir_size_bytes_sync_inner(&entry_path, traversal, visited_dirs, budget);
            total = total.saturating_add(sub.bytes);
            if !sub.complete {
                return DirSize {
                    bytes: total,
                    complete: false,
                };
            }
        } else {
            total = total.saturating_add(meta.len());
        }
    }

    DirSize {
        bytes: total,
        complete: true,
    }
}

fn dir_size_with_budget(
    path: &Path,
    traversal: SizeTraversalOptions,
    budget: Option<SizeBudget>,
) -> DirSize {
    let mut visited_dirs = HashSet::<(u64, u64)>::new();
    if traversal.dedup_dir_inodes
        && let Ok(meta) = fs::metadata(path)
        && let Some(key) = dir_key(&meta)
    {
        let _ = visited_dirs.insert(key);
    }

    let mut budget = budget;
    dir_size_bytes_sync_inner(path, traversal, &mut visited_dirs, &mut budget)
}

pub(crate) fn dir_size_bytes_sync_with_config(path: &Path, config: &Config) -> u64 {
    dir_size_with_budget(path, config.scan.size_traversal, None).bytes
}

pub async fn dir_size_bytes(path: &Path, ctx: &AppContext) -> u64 {
    let sem = ctx.fs_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");
    let path = path.to_path_buf();
    let config = ctx.config().clone();
    tokio::task::spawn_blocking(move || dir_size_bytes_sync_with_config(&path, &config))
        .await
        .unwrap_or(0)
}

pub async fn dir_size_bytes_budgeted(path: &Path, ctx: &AppContext) -> DirSize {
    let sem = ctx.fs_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");
    let path: PathBuf = path.to_path_buf();
    let config = ctx.config().clone();
    tokio::task::spawn_blocking(move || {
        let budget = Some(SizeBudget::from_config(config.scan.tui_size_budget));
        dir_size_with_budget(&path, config.scan.size_traversal, budget)
    })
    .await
    .unwrap_or(DirSize {
        bytes: 0,
        complete: false,
    })
}
