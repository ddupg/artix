use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::AppContext;
use crate::model::SizeStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SizeTraversalOptions {
    follow_symlinks: bool,
    dedup_dir_inodes: bool,
}

const DEFAULT_TRAVERSAL: SizeTraversalOptions = SizeTraversalOptions {
    follow_symlinks: false,
    dedup_dir_inodes: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeMeasurement {
    bytes: u64,
    status: SizeStatus,
}

impl SizeMeasurement {
    pub(crate) const fn complete(bytes: u64) -> Self {
        Self {
            bytes,
            status: SizeStatus::Complete,
        }
    }

    pub(crate) const fn incomplete(bytes: u64) -> Self {
        Self {
            bytes,
            status: SizeStatus::Incomplete,
        }
    }

    pub const fn bytes(self) -> u64 {
        self.bytes
    }

    pub const fn status(self) -> SizeStatus {
        self.status
    }

    fn add(&mut self, other: Self) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.status = self.status.combine(other.status);
    }

    fn mark_incomplete(&mut self) {
        self.status = SizeStatus::Incomplete;
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
) -> SizeMeasurement {
    let mut measurement = SizeMeasurement::complete(0);
    let mut pending_dirs = vec![PathBuf::from(path)];

    while let Some(dir) = pending_dirs.pop() {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(_) => {
                measurement.mark_incomplete();
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    measurement.mark_incomplete();
                    continue;
                }
            };
            let entry_path = entry.path();

            let meta_link = match fs::symlink_metadata(&entry_path) {
                Ok(meta) => meta,
                Err(_) => {
                    measurement.mark_incomplete();
                    continue;
                }
            };
            let is_symlink = meta_link.file_type().is_symlink();

            if is_symlink && !traversal.follow_symlinks {
                measurement.add(SizeMeasurement::complete(meta_link.len()));
                continue;
            }

            let meta = if is_symlink && traversal.follow_symlinks {
                match fs::metadata(&entry_path) {
                    Ok(meta) => meta,
                    Err(_) => {
                        measurement.add(SizeMeasurement::incomplete(meta_link.len()));
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

                pending_dirs.push(entry_path);
            } else {
                measurement.add(SizeMeasurement::complete(meta.len()));
            }
        }
    }

    measurement
}

fn measure_path(path: &Path, traversal: SizeTraversalOptions) -> SizeMeasurement {
    let root_link_meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(_) => return SizeMeasurement::incomplete(0),
    };
    let root_is_symlink = root_link_meta.file_type().is_symlink();
    if root_is_symlink && !traversal.follow_symlinks {
        return SizeMeasurement::complete(root_link_meta.len());
    }
    let root_meta = if root_is_symlink {
        match fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => return SizeMeasurement::incomplete(root_link_meta.len()),
        }
    } else {
        root_link_meta
    };
    if !root_meta.file_type().is_dir() {
        return SizeMeasurement::complete(root_meta.len());
    }

    let mut visited_dirs = HashSet::<(u64, u64)>::new();
    if traversal.dedup_dir_inodes
        && let Some(key) = dir_key(&root_meta)
    {
        let _ = visited_dirs.insert(key);
    }

    dir_size_bytes_sync_inner(path, traversal, &mut visited_dirs)
}

pub(crate) fn measure_path_sync(path: &Path) -> SizeMeasurement {
    measure_path(path, DEFAULT_TRAVERSAL)
}

pub async fn measure_size(path: &Path, ctx: &AppContext) -> SizeMeasurement {
    let sem = ctx.fs_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || measure_path_sync(&path))
        .await
        .unwrap_or_else(|_| SizeMeasurement::incomplete(0))
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs;
    use std::process::Command;

    use tempfile::tempdir;

    use super::{DEFAULT_TRAVERSAL, SizeMeasurement, SizeTraversalOptions, measure_path};
    use crate::model::SizeStatus;

    #[test]
    fn missing_path_is_an_incomplete_zero_measurement() {
        let dir = tempdir().expect("tempdir");
        let measurement = measure_path(&dir.path().join("missing"), DEFAULT_TRAVERSAL);

        assert_eq!(measurement, SizeMeasurement::incomplete(0));
    }

    #[test]
    fn readable_directory_is_complete() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("artifact"), "12345").expect("artifact");

        let measurement = measure_path(dir.path(), DEFAULT_TRAVERSAL);

        assert_eq!(measurement.bytes(), 5);
        assert_eq!(measurement.status(), SizeStatus::Complete);
    }

    #[cfg(unix)]
    #[test]
    fn deep_tree_measurement_stays_complete_under_low_fd_limit() {
        const CHILD_ENV: &str = "ARTIX_LOW_FD_SIZE_TEST_CHILD";

        if env::var_os(CHILD_ENV).is_none() {
            let current_exe = env::current_exe().expect("current test executable");
            let status = Command::new("sh")
                .args(["-c", "ulimit -n 32 && exec \"$@\"", "sh"])
                .arg(current_exe)
                .args([
                    "--exact",
                    "scan::size::tests::deep_tree_measurement_stays_complete_under_low_fd_limit",
                    "--nocapture",
                ])
                .env(CHILD_ENV, "1")
                .status()
                .expect("run low-fd child test");
            assert!(status.success(), "low-fd child test failed: {status}");
            return;
        }

        let temp = tempdir().expect("tempdir");
        let root = temp.path().join("root");
        let mut path = root.clone();
        let mut directories = vec![root.clone()];
        fs::create_dir(&root).expect("create root");
        for _ in 0..64 {
            path.push("d");
            fs::create_dir(&path).expect("create nested directory");
            directories.push(path.clone());
        }
        let leaf = path.join("artifact");
        fs::write(&leaf, "12345").expect("write artifact");

        let measurement = measure_path(&root, DEFAULT_TRAVERSAL);

        fs::remove_file(leaf).expect("remove artifact");
        for directory in directories.into_iter().rev() {
            fs::remove_dir(directory).expect("remove fixture directory");
        }

        assert_eq!(measurement, SizeMeasurement::complete(5));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_symlink_target_keeps_partial_bytes_and_marks_incomplete() {
        use std::os::unix::fs::symlink;

        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("artifact"), "12345").expect("artifact");
        let link_target = "missing-target";
        symlink(link_target, dir.path().join("broken")).expect("broken symlink");
        let traversal = SizeTraversalOptions {
            follow_symlinks: true,
            dedup_dir_inodes: true,
        };

        let measurement = measure_path(dir.path(), traversal);

        assert_eq!(measurement.bytes(), 5 + link_target.len() as u64);
        assert_eq!(measurement.status(), SizeStatus::Incomplete);
    }
}
