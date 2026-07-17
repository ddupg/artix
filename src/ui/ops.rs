#[derive(Debug)]
pub(super) struct OperationTracker {
    next_request_id: u64,
    pending_delete_id: Option<u64>,
    pending_clean_id: Option<u64>,
}

impl OperationTracker {
    pub(super) fn new() -> Self {
        Self {
            next_request_id: 1,
            pending_delete_id: None,
            pending_clean_id: None,
        }
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

#[cfg(test)]
mod tests {
    use super::OperationTracker;

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
}
