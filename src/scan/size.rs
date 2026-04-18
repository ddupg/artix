use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;

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
    fn from_env_for_tui() -> Self {
        // Defaults are chosen to prevent pathological directories from hanging the UI
        // while still allowing most directories to complete.
        let remaining_entries = match std::env::var("ARTIX_SIZE_MAX_ENTRIES") {
            Ok(value) => match value.parse::<u64>() {
                Ok(0) | Err(_) => None,
                Ok(n) => Some(n),
            },
            Err(_) => Some(1_000_000),
        };

        let deadline = match std::env::var("ARTIX_SIZE_TIMEOUT_MS") {
            Ok(value) => match value.parse::<u64>() {
                Ok(0) | Err(_) => None,
                Ok(ms) => Some(Instant::now() + Duration::from_millis(ms)),
            },
            Err(_) => Some(Instant::now() + Duration::from_millis(3_000)),
        };

        Self {
            remaining_entries,
            deadline,
        }
    }

    fn step(&mut self) -> bool {
        if let Some(deadline) = self.deadline {
            if Instant::now() > deadline {
                return false;
            }
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

fn env_flag(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

fn should_follow_symlinks() -> bool {
    env_flag("ARTIX_SIZE_FOLLOW_SYMLINKS")
}

fn should_dedup_dir_inodes() -> bool {
    // Enabled by default on unix; can be disabled for troubleshooting.
    std::env::var("ARTIX_SIZE_DEDUP_DIR_INODES")
        .ok()
        .map(|value| value != "0")
        .unwrap_or(true)
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
    follow_symlinks: bool,
    dedup_dir_inodes: bool,
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
        if let Some(budget) = budget.as_mut() {
            if !budget.step() {
                return DirSize {
                    bytes: total,
                    complete: false,
                };
            }
        }

        let entry_path = entry.path();

        // Always lstat first so we can detect symlinks.
        let meta_link = match fs::symlink_metadata(&entry_path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let is_symlink = meta_link.file_type().is_symlink();

        if is_symlink && !follow_symlinks {
            // Count the symlink itself (small) but do not recurse into its target.
            total = total.saturating_add(meta_link.len());
            continue;
        }

        let meta = if is_symlink && follow_symlinks {
            // Follow symlink to target.
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
            if dedup_dir_inodes {
                if let Some(key) = dir_key(&meta) {
                    if !visited_dirs.insert(key) {
                        continue;
                    }
                }
            }

            let sub = dir_size_bytes_sync_inner(
                &entry_path,
                follow_symlinks,
                dedup_dir_inodes,
                visited_dirs,
                budget,
            );
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

pub(crate) fn dir_size_bytes_sync(path: &Path) -> u64 {
    let follow_symlinks = should_follow_symlinks();
    let dedup_dir_inodes = should_dedup_dir_inodes();
    let mut visited_dirs = HashSet::<(u64, u64)>::new();
    let mut budget = None;

    if dedup_dir_inodes {
        if let Ok(meta) = fs::metadata(path) {
            if let Some(key) = dir_key(&meta) {
                let _ = visited_dirs.insert(key);
            }
        }
    }

    dir_size_bytes_sync_inner(
        path,
        follow_symlinks,
        dedup_dir_inodes,
        &mut visited_dirs,
        &mut budget,
    )
    .bytes
}

static SIZE_CONCURRENCY: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn size_semaphore() -> Arc<Semaphore> {
    SIZE_CONCURRENCY
        .get_or_init(|| Arc::new(Semaphore::new(size_concurrency_limit())))
        .clone()
}

fn size_concurrency_limit() -> usize {
    let default = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let default = (default.saturating_mul(2)).clamp(2, 16);

    std::env::var("ARTIX_FS_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(default)
}

pub async fn dir_size_bytes(path: &Path) -> u64 {
    let sem = size_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || dir_size_bytes_sync(&path))
        .await
        .unwrap_or(0)
}

pub async fn dir_size_bytes_budgeted(path: &Path) -> DirSize {
    let sem = size_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");
    let path: PathBuf = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let follow_symlinks = should_follow_symlinks();
        let dedup_dir_inodes = should_dedup_dir_inodes();
        let mut visited_dirs = HashSet::<(u64, u64)>::new();
        if dedup_dir_inodes {
            if let Ok(meta) = fs::metadata(&path) {
                if let Some(key) = dir_key(&meta) {
                    let _ = visited_dirs.insert(key);
                }
            }
        }
        let mut budget = Some(SizeBudget::from_env_for_tui());
        dir_size_bytes_sync_inner(
            &path,
            follow_symlinks,
            dedup_dir_inodes,
            &mut visited_dirs,
            &mut budget,
        )
    })
    .await
    .unwrap_or(DirSize {
        bytes: 0,
        complete: false,
    })
}
