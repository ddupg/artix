use std::path::Path;

use crate::model::{BrowserEntry, EntryKind, GitStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    All,
    CleanupFocus,
    IgnoredOnly,
    UntrackedAndIgnored,
}

impl FilterMode {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::CleanupFocus,
            Self::CleanupFocus => Self::IgnoredOnly,
            Self::IgnoredOnly => Self::UntrackedAndIgnored,
            Self::UntrackedAndIgnored => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::CleanupFocus => "Cleanup Focus",
            Self::IgnoredOnly => "Ignored Only",
            Self::UntrackedAndIgnored => "Untracked + Ignored",
        }
    }

    fn includes(self, entry: &BrowserEntry) -> bool {
        if matches!(entry.entry_kind, EntryKind::Parent) {
            return true;
        }

        match self {
            Self::All => true,
            Self::CleanupFocus => !matches!(entry.git_status, GitStatus::Tracked),
            Self::IgnoredOnly => matches!(entry.git_status, GitStatus::Ignored),
            Self::UntrackedAndIgnored => {
                matches!(entry.git_status, GitStatus::Ignored | GitStatus::Untracked)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowserList {
    entries: Vec<BrowserEntry>,
    filter_mode: FilterMode,
    selected_index: usize,
}

impl BrowserList {
    pub(super) fn new(entries: Vec<BrowserEntry>) -> Self {
        let mut list = Self {
            entries,
            filter_mode: FilterMode::All,
            selected_index: 0,
        };
        list.clamp_selection();
        list
    }

    pub(super) fn entries(&self) -> &[BrowserEntry] {
        &self.entries
    }

    pub(super) fn filter_mode(&self) -> FilterMode {
        self.filter_mode
    }

    pub(super) fn set_filter_mode(&mut self, filter_mode: FilterMode) {
        self.filter_mode = filter_mode;
        self.clamp_selection();
    }

    pub(super) fn cycle_filter_mode(&mut self) {
        self.set_filter_mode(self.filter_mode.next());
    }

    pub(super) fn reset(&mut self, entries: Vec<BrowserEntry>) {
        self.entries = entries;
        self.selected_index = 0;
        self.clamp_selection();
    }

    pub(super) fn reset_preserving_selection(
        &mut self,
        entries: Vec<BrowserEntry>,
        selected_path: Option<&Path>,
    ) {
        self.reset(entries);
        self.restore_selection_by_path(selected_path);
    }

    pub(super) fn refresh(&mut self, entries: Vec<BrowserEntry>) {
        let selected_path = self.selected().map(|entry| entry.path.clone());
        self.entries = entries;
        self.restore_selection_by_path(selected_path.as_deref());
    }

    pub(super) fn snapshot(&self) -> BrowserListSnapshot<'_> {
        BrowserListSnapshot {
            entries: self.visible_entries().collect(),
            selected_index: self.selected_index,
        }
    }

    pub(super) fn selected(&self) -> Option<&BrowserEntry> {
        self.visible_entries().nth(self.selected_index)
    }

    pub(super) fn move_next(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = if self.selected_index + 1 >= len {
                0
            } else {
                self.selected_index + 1
            };
        }
    }

    pub(super) fn move_previous(&mut self) {
        let len = self.visible_len();
        if len == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = if self.selected_index == 0 {
                len - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub(super) fn jump_first(&mut self) {
        self.selected_index = 0;
    }

    pub(super) fn jump_last(&mut self) {
        let len = self.visible_len();
        self.selected_index = len.saturating_sub(1);
    }

    fn visible_entries(&self) -> impl Iterator<Item = &BrowserEntry> {
        self.entries
            .iter()
            .filter(|entry| self.filter_mode.includes(entry))
    }

    fn visible_len(&self) -> usize {
        self.visible_entries().count()
    }

    fn clamp_selection(&mut self) {
        self.selected_index = self
            .selected_index
            .min(self.visible_len().saturating_sub(1));
    }

    fn restore_selection_by_path(&mut self, selected_path: Option<&Path>) {
        let restored_index = selected_path.and_then(|selected_path| {
            self.visible_entries()
                .position(|entry| entry.path == selected_path)
        });
        if let Some(index) = restored_index {
            self.selected_index = index;
            return;
        }

        self.clamp_selection();
    }
}

#[derive(Debug)]
pub(super) struct BrowserListSnapshot<'a> {
    entries: Vec<&'a BrowserEntry>,
    selected_index: usize,
}

impl<'a> BrowserListSnapshot<'a> {
    pub(super) fn entries(&self) -> &[&'a BrowserEntry] {
        &self.entries
    }

    pub(super) fn selected_index(&self) -> usize {
        self.selected_index
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel};

    use super::{BrowserList, FilterMode};

    #[test]
    fn refresh_preserves_selected_path_after_reorder() {
        let mut list = BrowserList::new(vec![entry("a"), entry("b"), entry("c")]);
        list.move_next();

        list.refresh(vec![entry("c"), entry("b"), entry("a")]);

        assert_eq!(list.selected().expect("selected").name, "b");
        assert_eq!(list.snapshot().selected_index(), 1);
    }

    #[test]
    fn refresh_clamps_the_previous_index_when_selected_path_disappears() {
        let mut list = BrowserList::new(vec![entry("a"), entry("b"), entry("c")]);
        list.jump_last();

        list.refresh(vec![entry("a"), entry("b")]);

        assert_eq!(list.selected().expect("selected").name, "b");
        assert_eq!(list.snapshot().selected_index(), 1);
    }

    #[test]
    fn filter_change_clamps_the_numeric_index() {
        let mut list = BrowserList::new(vec![
            BrowserEntry::parent(PathBuf::from("/workspace")),
            entry_with_status("tracked", GitStatus::Tracked),
            entry_with_status("ignored", GitStatus::Ignored),
            entry_with_status("unknown", GitStatus::Unknown),
        ]);
        list.move_next();
        list.move_next();

        list.set_filter_mode(FilterMode::CleanupFocus);

        assert_eq!(list.selected().expect("selected").name, "unknown");
        assert_eq!(list.snapshot().selected_index(), 2);
    }

    fn entry(name: &str) -> BrowserEntry {
        entry_with_status(name, GitStatus::Unknown)
    }

    fn entry_with_status(name: &str, git_status: GitStatus) -> BrowserEntry {
        BrowserEntry {
            path: PathBuf::from(format!("/workspace/{name}")),
            name: name.to_string(),
            size_bytes: 0,
            reclaimable_bytes: 0,
            entry_kind: EntryKind::Directory,
            git_status,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Hidden,
        }
    }
}
