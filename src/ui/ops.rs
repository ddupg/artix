use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::model::BrowserEntry;

#[derive(Debug)]
pub(super) struct OperationTracker {
    next_request_id: u64,
    pending_load_id: Option<u64>,
    pending_delete_id: Option<u64>,
    pending_clean_id: Option<u64>,
}

impl OperationTracker {
    pub(super) fn new() -> Self {
        Self {
            next_request_id: 1,
            pending_load_id: None,
            pending_delete_id: None,
            pending_clean_id: None,
        }
    }

    pub(super) fn start_load(&mut self) -> u64 {
        let request_id = self.allocate_request_id();
        self.pending_load_id = Some(request_id);
        request_id
    }

    pub(super) fn is_pending_load(&self, request_id: u64) -> bool {
        self.pending_load_id == Some(request_id)
    }

    pub(super) fn start_delete(&mut self) -> u64 {
        let request_id = self.allocate_request_id();
        self.pending_delete_id = Some(request_id);
        request_id
    }

    pub(super) fn finish_delete(&mut self, request_id: u64) -> bool {
        if self.pending_delete_id != Some(request_id) {
            return false;
        }

        self.pending_delete_id = None;
        true
    }

    pub(super) fn start_clean(&mut self) -> u64 {
        let request_id = self.allocate_request_id();
        self.pending_clean_id = Some(request_id);
        request_id
    }

    pub(super) fn finish_clean(&mut self, request_id: u64) -> bool {
        if self.pending_clean_id != Some(request_id) {
            return false;
        }

        self.pending_clean_id = None;
        true
    }

    fn allocate_request_id(&mut self) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        request_id
    }
}

pub(super) fn invalidate_related_paths(
    cache: &mut HashMap<PathBuf, Vec<BrowserEntry>>,
    path: &Path,
) {
    cache.retain(|cached_dir, _| {
        !(path.starts_with(cached_dir.as_path()) || cached_dir.starts_with(path))
    });
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::{OperationTracker, invalidate_related_paths};

    #[test]
    fn load_requests_replace_the_pending_load() {
        let mut ops = OperationTracker::new();

        let first = ops.start_load();
        let second = ops.start_load();

        assert!(!ops.is_pending_load(first));
        assert!(ops.is_pending_load(second));
    }

    #[test]
    fn finish_delete_only_clears_the_matching_request() {
        let mut ops = OperationTracker::new();

        let request_id = ops.start_delete();

        assert!(!ops.finish_delete(request_id + 1));
        assert!(ops.finish_delete(request_id));
        assert!(!ops.finish_delete(request_id));
    }

    #[test]
    fn finish_clean_only_clears_the_matching_request() {
        let mut ops = OperationTracker::new();

        let request_id = ops.start_clean();

        assert!(!ops.finish_clean(request_id + 1));
        assert!(ops.finish_clean(request_id));
        assert!(!ops.finish_clean(request_id));
    }

    #[test]
    fn invalidation_removes_ancestors_and_descendants() {
        let mut cache = HashMap::from([
            (PathBuf::from("/repo"), Vec::new()),
            (PathBuf::from("/repo/target"), Vec::new()),
            (PathBuf::from("/repo/src"), Vec::new()),
            (PathBuf::from("/other"), Vec::new()),
        ]);

        invalidate_related_paths(&mut cache, PathBuf::from("/repo/target").as_path());

        assert!(!cache.contains_key(&PathBuf::from("/repo")));
        assert!(!cache.contains_key(&PathBuf::from("/repo/target")));
        assert!(cache.contains_key(&PathBuf::from("/repo/src")));
        assert!(cache.contains_key(&PathBuf::from("/other")));
    }
}
