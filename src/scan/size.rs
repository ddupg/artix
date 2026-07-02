use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::config::{AppContext, Config, SizeTraversalOptions};

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
) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };

    let mut total = 0u64;

    for entry in entries.flatten() {
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

            let sub = dir_size_bytes_sync_inner(&entry_path, traversal, visited_dirs);
            total = total.saturating_add(sub);
        } else {
            total = total.saturating_add(meta.len());
        }
    }

    total
}

fn dir_size(path: &Path, traversal: SizeTraversalOptions) -> u64 {
    let mut visited_dirs = HashSet::<(u64, u64)>::new();
    if traversal.dedup_dir_inodes
        && let Ok(meta) = fs::metadata(path)
        && let Some(key) = dir_key(&meta)
    {
        let _ = visited_dirs.insert(key);
    }

    dir_size_bytes_sync_inner(path, traversal, &mut visited_dirs)
}

pub(crate) fn dir_size_bytes_sync_with_config(path: &Path, config: &Config) -> u64 {
    dir_size(path, config.scan.size_traversal)
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
