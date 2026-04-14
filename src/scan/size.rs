use std::fs;
use std::path::Path;

pub fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };

    let mut total = 0;

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() {
            total += dir_size_bytes(&entry_path);
        } else if let Ok(metadata) = entry.metadata() {
            total += metadata.len();
        }
    }

    total
}
