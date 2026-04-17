use std::fs;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;

pub(crate) fn dir_size_bytes_sync(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };

    let mut total = 0;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            total += dir_size_bytes_sync(&entry_path);
        } else if let Ok(metadata) = entry.metadata() {
            total += metadata.len();
        }
    }

    total
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
    let _permit = sem
        .acquire()
        .await
        .expect("semaphore must not be closed");
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || dir_size_bytes_sync(&path))
        .await
        .unwrap_or(0)
}
