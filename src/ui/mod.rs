mod theme;

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::classify::git::resolve_git_context;
use crate::classify::risk::classify_risk_level;
use crate::delete::DeleteMode;
use crate::delete_flow::{delete_intent_for, execute_delete};
use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, Project, RiskLevel};
use crate::rules::default_rules;

use tokio::sync::mpsc;

pub use crate::delete_flow::{DeleteIntent, DeleteState, DeleteTargetKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewRow {
    pub project_name: String,
    pub reclaimable_bytes: u64,
    pub candidate_count: usize,
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    current_dir: PathBuf,
    current_git_context: GitContext,
    entries: Vec<BrowserEntry>,
    filter_mode: FilterMode,
    selected_index: usize,
    delete_state: DeleteState,
}

impl AppState {
    pub fn new(current_dir: PathBuf, entries: Vec<BrowserEntry>) -> Self {
        Self {
            current_git_context: resolve_git_context(&current_dir).unwrap_or_default(),
            current_dir,
            entries,
            filter_mode: FilterMode::All,
            selected_index: 0,
            delete_state: DeleteState::Idle,
        }
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    pub fn entries(&self) -> &[BrowserEntry] {
        &self.entries
    }

    pub fn replace_entries(&mut self, current_dir: PathBuf, entries: Vec<BrowserEntry>) {
        self.current_git_context = resolve_git_context(&current_dir).unwrap_or_default();
        self.current_dir = current_dir;
        self.entries = entries;
        self.selected_index = 0;
    }

    pub fn filter_mode(&self) -> FilterMode {
        self.filter_mode
    }

    pub fn current_git_context(&self) -> &GitContext {
        &self.current_git_context
    }

    pub fn set_filter_mode(&mut self, filter_mode: FilterMode) {
        self.filter_mode = filter_mode;
        self.clamp_selection();
    }

    pub fn cycle_filter_mode(&mut self) {
        self.filter_mode = self.filter_mode.next();
        self.clamp_selection();
    }

    pub fn visible_entries(&self) -> Vec<BrowserEntry> {
        self.entries
            .iter()
            .filter(|entry| self.is_visible(entry))
            .cloned()
            .collect()
    }

    pub fn selected_entry(&self) -> Option<BrowserEntry> {
        self.visible_entries().get(self.selected_index).cloned()
    }

    pub fn move_selection_down(&mut self) {
        let len = self.visible_entries().len();
        if len == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = (self.selected_index + 1).min(len.saturating_sub(1));
        }
    }

    pub fn move_selection_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    pub fn delete_intent_for(&self, entry: &BrowserEntry) -> DeleteIntent {
        delete_intent_for(entry)
    }

    pub fn request_delete_for_selected(&mut self) {
        let Some(entry) = self.selected_entry() else {
            return;
        };
        let DeleteIntent::Confirm {
            target_kind,
            requires_extra_confirmation,
        } = self.delete_intent_for(&entry);
        self.delete_state = DeleteState::Confirming {
            entry,
            target_kind,
            requires_extra_confirmation,
            requested_mode: None,
        };
    }

    pub fn delete_state(&self) -> &DeleteState {
        &self.delete_state
    }

    pub fn set_delete_mode(&mut self, mode: DeleteMode) {
        if let DeleteState::Confirming { requested_mode, .. } = &mut self.delete_state {
            *requested_mode = Some(mode);
        }
    }

    pub fn set_delete_running(&mut self) {
        if let DeleteState::Confirming {
            entry,
            requested_mode: Some(mode),
            ..
        } = &self.delete_state
        {
            self.delete_state = DeleteState::Running {
                entry: entry.clone(),
                mode: mode.clone(),
            };
        }
    }

    pub fn request_extra_confirmation(&mut self) {
        if let DeleteState::Confirming {
            entry,
            target_kind,
            requires_extra_confirmation: true,
            requested_mode: Some(mode),
        } = &self.delete_state
        {
            self.delete_state = DeleteState::AwaitingExtraConfirmation {
                entry: entry.clone(),
                mode: mode.clone(),
                target_kind: target_kind.clone(),
            };
        }
    }

    pub fn finish_delete_failure(&mut self, message: String) {
        self.delete_state = DeleteState::Failed { message };
    }

    pub fn clear_delete_state(&mut self) {
        self.delete_state = DeleteState::Idle;
    }

    fn is_visible(&self, entry: &BrowserEntry) -> bool {
        if matches!(entry.entry_kind, EntryKind::Parent) {
            return true;
        }

        match self.filter_mode {
            FilterMode::All => true,
            FilterMode::CleanupFocus => !matches!(entry.git_status, GitStatus::Tracked),
            FilterMode::IgnoredOnly => matches!(entry.git_status, GitStatus::Ignored),
            FilterMode::UntrackedAndIgnored => {
                matches!(entry.git_status, GitStatus::Ignored | GitStatus::Untracked)
            }
        }
    }

    fn clamp_selection(&mut self) {
        let len = self.visible_entries().len();
        if len == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = self.selected_index.min(len - 1);
        }
    }
}

pub fn build_overview_rows(mut projects: Vec<Project>) -> Vec<OverviewRow> {
    projects.sort_by(|left, right| right.reclaimable_bytes.cmp(&left.reclaimable_bytes));

    projects
        .into_iter()
        .map(|project| OverviewRow {
            project_name: project.name,
            reclaimable_bytes: project.reclaimable_bytes,
            candidate_count: project.candidate_count,
        })
        .collect()
}

pub async fn run_tui(start_dir: PathBuf) -> Result<(), String> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, start_dir);
    ratatui::restore();
    result.map_err(|err| err.to_string())
}

fn run_app(terminal: &mut ratatui::DefaultTerminal, start_dir: PathBuf) -> io::Result<()> {
    let mut app = BrowserApp::new(start_dir).map_err(io::Error::other)?;

    loop {
        app.pump_background();
        app.spinner_tick = app.spinner_tick.wrapping_add(1);
        terminal.draw(|frame| render(frame, &app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Char('q') => break,
            KeyCode::Down | KeyCode::Char('j') => app.state.move_selection_down(),
            KeyCode::Up | KeyCode::Char('k') => app.state.move_selection_up(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                app.enter_selected().map_err(io::Error::other)?;
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                app.enter_parent().map_err(io::Error::other)?;
            }
            KeyCode::Char('f') => app.state.cycle_filter_mode(),
            KeyCode::Char('d') => app.state.request_delete_for_selected(),
            KeyCode::Char('t') => app
                .run_delete(DeleteMode::Trash)
                .map_err(io::Error::other)?,
            KeyCode::Char('x') => app
                .run_delete(DeleteMode::Permanent { confirmed: true })
                .map_err(io::Error::other)?,
            KeyCode::Char('y') => app.confirm_extra_delete().map_err(io::Error::other)?,
            KeyCode::Esc => app.state.clear_delete_state(),
            _ => {}
        }
    }

    Ok(())
}

#[derive(Debug)]
struct BrowserApp {
    state: AppState,
    root_dir: PathBuf,
    cache: HashMap<PathBuf, Vec<BrowserEntry>>,
    icon_mode: theme::IconMode,

    bg_tx: mpsc::UnboundedSender<BgRequest>,
    bg_rx: mpsc::UnboundedReceiver<BgResponse>,
    next_request_id: u64,
    pending_load_id: Option<u64>,
    pending_delete_id: Option<u64>,

    loading_paths: HashSet<PathBuf>,
    spinner_tick: usize,
}

#[derive(Debug)]
enum BgRequest {
    LoadDirectory { request_id: u64, dir: PathBuf },
    Delete {
        request_id: u64,
        entry: BrowserEntry,
        mode: DeleteMode,
    },
}

#[derive(Debug)]
enum BgResponse {
    DirectoryLoaded {
        request_id: u64,
        dir: PathBuf,
        result: Result<Vec<BrowserEntry>, String>,
    },
    EntryUpdated {
        request_id: u64,
        dir: PathBuf,
        entry: BrowserEntry,
    },
    DeleteFinished {
        request_id: u64,
        entry_path: PathBuf,
        result: Result<String, String>,
    },
}

impl BrowserApp {
    fn new(start_dir: PathBuf) -> Result<Self, String> {
        let (bg_tx, mut bg_req_rx) = mpsc::unbounded_channel::<BgRequest>();
        let (bg_resp_tx, bg_rx) = mpsc::unbounded_channel::<BgResponse>();

        let root_dir = start_dir.clone();
        tokio::spawn(async move {
            while let Some(req) = bg_req_rx.recv().await {
                match req {
                    BgRequest::LoadDirectory { request_id, dir } => {
                        let root_dir = root_dir.clone();
                        let bg_resp_tx = bg_resp_tx.clone();
                        tokio::spawn(async move {
                            // Send a quick listing first (0B sizes) so UI can populate even if
                            // the in-thread quick path failed.
                            let initial = quick_browse_directory(&dir, &root_dir);
                            let _ = bg_resp_tx.send(BgResponse::DirectoryLoaded {
                                request_id,
                                dir: dir.clone(),
                                result: initial,
                            });

                            // Then progressively compute size/git per entry and stream updates.
                            let Ok(read_dir) = std::fs::read_dir(&dir) else {
                                return;
                            };
                            let rules = default_rules();
                            let current_context = resolve_git_context(&dir);

                            for entry in read_dir.flatten() {
                                let entry_path = entry.path();
                                if !entry_path.is_dir() {
                                    continue;
                                }
                                let name = entry_path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                if name == ".git" {
                                    continue;
                                }

                                let candidate_rule = rules
                                    .iter()
                                    .find(|rule| rule.dir_name == name)
                                    .cloned();
                                let entry_kind = if candidate_rule.is_some() {
                                    EntryKind::CleanupCandidate
                                } else {
                                    EntryKind::Directory
                                };
                                let candidate_kind = candidate_rule.as_ref().map(|rule| rule.kind.to_string());
                                let is_visible_candidate = candidate_rule.is_some();

                                let dir_for_msg = dir.clone();
                                let current_context = current_context.clone();
                                let bg_resp_tx = bg_resp_tx.clone();

                                tokio::spawn(async move {
                                    let size_bytes = crate::scan::size::dir_size_bytes(&entry_path).await;
                                    let git_context =
                                        resolve_git_context(&entry_path).or_else(|| current_context);
                                    let git_status = crate::classify::git::classify_path_git_status(
                                        &entry_path,
                                        git_context.as_ref(),
                                    )
                                    .await;

                                    let risk_level = candidate_rule
                                        .as_ref()
                                        .map(|rule| classify_risk_level(rule, &git_status))
                                        .unwrap_or(RiskLevel::Hidden);

                                    let entry = BrowserEntry {
                                        path: entry_path,
                                        name,
                                        size_bytes,
                                        reclaimable_bytes: size_bytes,
                                        entry_kind,
                                        git_status,
                                        git_context: git_context.unwrap_or_default(),
                                        risk_level,
                                        candidate_kind,
                                        is_visible_candidate,
                                    };

                                    let _ = bg_resp_tx.send(BgResponse::EntryUpdated {
                                        request_id,
                                        dir: dir_for_msg,
                                        entry,
                                    });
                                });
                            }
                        });
                    }
                    BgRequest::Delete {
                        request_id,
                        entry,
                        mode,
                    } => {
                        let bg_resp_tx = bg_resp_tx.clone();
                        tokio::spawn(async move {
                            let entry_path = entry.path.clone();
                            let result = tokio::task::spawn_blocking(move || execute_delete(&entry, mode))
                                .await
                                .map_err(|err| err.to_string())
                                .and_then(|res| res);
                            let _ = bg_resp_tx.send(BgResponse::DeleteFinished {
                                request_id,
                                entry_path,
                                result,
                            });
                        });
                    }
                }
            }
        });

        let mut app = Self {
            state: AppState::new(start_dir.clone(), Vec::new()),
            root_dir: start_dir.clone(),
            cache: HashMap::new(),
            icon_mode: theme::IconMode::detect(),

            bg_tx,
            bg_rx,
            next_request_id: 1,
            pending_load_id: None,
            pending_delete_id: None,

            loading_paths: HashSet::new(),
            spinner_tick: 0,
        };

        app.load_directory(start_dir);
        Ok(app)
    }

    fn pump_background(&mut self) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            match msg {
                BgResponse::DirectoryLoaded {
                    request_id,
                    dir,
                    result,
                } => {
                    if self.pending_load_id != Some(request_id) {
                        continue;
                    }

                    match result {
                        Ok(entries) => {
                            self.cache.insert(dir.clone(), entries.clone());
                            if self.state.current_dir() == dir.as_path() && self.state.entries.is_empty() {
                                self.state.replace_entries(dir, entries);
                            }
                        }
                        Err(err) => {
                            self.state.finish_delete_failure(err);
                        }
                    }
                }
                BgResponse::EntryUpdated {
                    request_id,
                    dir,
                    entry,
                } => {
                    if self.pending_load_id != Some(request_id) {
                        continue;
                    }
                    if self.state.current_dir() != dir.as_path() {
                        continue;
                    }

                    if let Some(entries) = self.cache.get_mut(&dir) {
                        apply_entry_update(entries, &entry);
                        sort_entries(entries);
                    }
                    apply_entry_update(&mut self.state.entries, &entry);
                    self.resort_visible_entries_preserving_selection();
                    self.loading_paths.remove(&entry.path);
                }
                BgResponse::DeleteFinished {
                    request_id,
                    entry_path,
                    result,
                } => {
                    if self.pending_delete_id != Some(request_id) {
                        continue;
                    }
                    self.pending_delete_id = None;

                    match result {
                        Ok(_message) => {
                            self.invalidate_related_paths(&entry_path);
                            let current = self.state.current_dir().to_path_buf();
                            self.state.clear_delete_state();
                            self.load_directory(current);
                        }
                        Err(err) => self.state.finish_delete_failure(err),
                    }
                }
            }
        }
    }

    fn enter_selected(&mut self) -> Result<(), String> {
        let Some(entry) = self.state.selected_entry() else {
            return Ok(());
        };
        self.load_directory(entry.path);
        Ok(())
    }

    fn enter_parent(&mut self) -> Result<(), String> {
        // Don't allow going above the root directory
        if self.state.current_dir() == self.root_dir {
            return Ok(());
        }
        let Some(parent) = self.state.current_dir().parent().map(Path::to_path_buf) else {
            return Ok(());
        };
        self.load_directory(parent);
        Ok(())
    }

    fn load_directory(&mut self, dir: PathBuf) {
        self.loading_paths.clear();
        if let Some(entries) = self.cache.get(&dir).cloned() {
            self.state.replace_entries(dir, entries);
            self.loading_paths.extend(
                self.state
                    .entries()
                    .iter()
                    .filter(|entry| !matches!(entry.entry_kind, EntryKind::Parent))
                    .filter(|entry| entry.size_bytes == 0 && entry.git_status == GitStatus::Unknown)
                    .map(|entry| entry.path.clone()),
            );
            return;
        }

        // Provide a fast placeholder listing so the UI is immediately usable.
        if let Ok(entries) = quick_browse_directory(&dir, &self.root_dir) {
            self.cache.insert(dir.clone(), entries.clone());
            self.state.replace_entries(dir.clone(), entries);
            self.loading_paths.extend(
                self.state
                    .entries()
                    .iter()
                    .filter(|entry| !matches!(entry.entry_kind, EntryKind::Parent))
                    .map(|entry| entry.path.clone()),
            );
        } else {
            // Optimistically switch directory; entries will be populated asynchronously.
            self.state.replace_entries(dir.clone(), Vec::new());
        }

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.pending_load_id = Some(request_id);
        let _ = self
            .bg_tx
            .send(BgRequest::LoadDirectory { request_id, dir });
    }

    fn resort_visible_entries_preserving_selection(&mut self) {
        let selected_path = self.state.selected_entry().map(|entry| entry.path);
        sort_entries(&mut self.state.entries);

        let Some(selected_path) = selected_path else {
            self.state.clamp_selection();
            return;
        };

        let visible = self.state.visible_entries();
        if let Some(idx) = visible
            .iter()
            .position(|entry| entry.path == selected_path)
        {
            self.state.selected_index = idx;
        } else {
            self.state.clamp_selection();
        }
    }

    fn run_delete(&mut self, mode: DeleteMode) -> Result<(), String> {
        match self.state.delete_state() {
            DeleteState::Confirming {
                requires_extra_confirmation: true,
                ..
            } if matches!(mode, DeleteMode::Permanent { .. }) => {
                self.state.set_delete_mode(mode);
                self.state.request_extra_confirmation();
                Ok(())
            }
            DeleteState::Confirming { .. } => {
                self.state.set_delete_mode(mode.clone());
                self.state.set_delete_running();
                let entry = match self.state.delete_state() {
                    DeleteState::Running { entry, .. } => entry.clone(),
                    _ => return Ok(()),
                };

                let request_id = self.next_request_id;
                self.next_request_id = self.next_request_id.saturating_add(1);
                self.pending_delete_id = Some(request_id);
                let _ = self.bg_tx.send(BgRequest::Delete {
                    request_id,
                    entry,
                    mode,
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn confirm_extra_delete(&mut self) -> Result<(), String> {
        let (entry, mode) = match self.state.delete_state() {
            DeleteState::AwaitingExtraConfirmation { entry, mode, .. } => {
                (entry.clone(), mode.clone())
            }
            _ => return Ok(()),
        };

        self.state.set_delete_running();

        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.pending_delete_id = Some(request_id);
        let _ = self.bg_tx.send(BgRequest::Delete {
            request_id,
            entry,
            mode,
        });
        Ok(())
    }

    fn invalidate_related_paths(&mut self, path: &Path) {
        self.cache.retain(|cached_dir, _| {
            !(path.starts_with(cached_dir.as_path()) || cached_dir.starts_with(path))
        });
    }
}

fn quick_browse_directory(path: &Path, root_dir: &Path) -> Result<Vec<BrowserEntry>, String> {
    let rules = default_rules();
    let current_context = resolve_git_context(path);
    let mut entries = Vec::new();

    if path != root_dir {
        if let Some(parent) = path.parent() {
            entries.push(BrowserEntry::parent(parent.to_path_buf()));
        }
    }

    let read_dir = std::fs::read_dir(path).map_err(|err| err.to_string())?;
    for entry in read_dir.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let name = entry_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string();
        if name == ".git" {
            continue;
        }

        let candidate_rule = rules.iter().find(|rule| rule.dir_name == name);
        let entry_kind = if candidate_rule.is_some() {
            EntryKind::CleanupCandidate
        } else {
            EntryKind::Directory
        };

        let git_context = resolve_git_context(&entry_path).or_else(|| current_context.clone());

        entries.push(BrowserEntry {
            path: entry_path,
            name,
            size_bytes: 0,
            reclaimable_bytes: 0,
            entry_kind,
            git_status: GitStatus::Unknown,
            git_context: git_context.unwrap_or_default(),
            risk_level: RiskLevel::Hidden,
            candidate_kind: candidate_rule.map(|rule| rule.kind.to_string()),
            is_visible_candidate: candidate_rule.is_some(),
        });
    }

    let (parents, mut rest): (Vec<_>, Vec<_>) = entries
        .into_iter()
        .partition(|entry| matches!(entry.entry_kind, EntryKind::Parent));
    rest.sort_by(|left, right| {
        match (&left.entry_kind, &right.entry_kind) {
            (EntryKind::CleanupCandidate, EntryKind::Directory) => std::cmp::Ordering::Less,
            (EntryKind::Directory, EntryKind::CleanupCandidate) => std::cmp::Ordering::Greater,
            _ => left.name.cmp(&right.name),
        }
    });

    Ok(parents.into_iter().chain(rest).collect())
}

fn sort_entries(entries: &mut Vec<BrowserEntry>) {
    let (parents, mut rest): (Vec<_>, Vec<_>) = entries
        .drain(..)
        .partition(|entry| matches!(entry.entry_kind, EntryKind::Parent));
    rest.sort_by(|left, right| {
        right
            .reclaimable_bytes
            .cmp(&left.reclaimable_bytes)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries.extend(parents.into_iter().chain(rest));
}

fn apply_entry_update(entries: &mut [BrowserEntry], update: &BrowserEntry) {
    for entry in entries {
        if entry.path == update.path {
            entry.size_bytes = update.size_bytes;
            entry.reclaimable_bytes = update.reclaimable_bytes;
            entry.git_status = update.git_status.clone();
            entry.git_context = update.git_context.clone();
            entry.risk_level = update.risk_level.clone();
            entry.entry_kind = update.entry_kind.clone();
            entry.candidate_kind = update.candidate_kind.clone();
            entry.is_visible_candidate = update.is_visible_candidate;
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::apply_entry_update;
    use super::sort_entries;
    use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel};

    #[test]
    fn apply_entry_update_updates_matching_path_in_place() {
        let mut entries = vec![BrowserEntry {
            path: "/tmp/a".into(),
            name: "a".into(),
            size_bytes: 0,
            reclaimable_bytes: 0,
            entry_kind: EntryKind::Directory,
            git_status: GitStatus::Unknown,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Hidden,
            candidate_kind: None,
            is_visible_candidate: false,
        }];

        let update = BrowserEntry {
            path: "/tmp/a".into(),
            name: "a".into(),
            size_bytes: 123,
            reclaimable_bytes: 123,
            entry_kind: EntryKind::CleanupCandidate,
            git_status: GitStatus::Ignored,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Low,
            candidate_kind: Some("rust-target".into()),
            is_visible_candidate: true,
        };

        apply_entry_update(&mut entries, &update);

        assert_eq!(entries[0].size_bytes, 123);
        assert_eq!(entries[0].reclaimable_bytes, 123);
        assert_eq!(entries[0].git_status, GitStatus::Ignored);
        assert_eq!(entries[0].risk_level, RiskLevel::Low);
        assert_eq!(entries[0].entry_kind, EntryKind::CleanupCandidate);
        assert_eq!(entries[0].candidate_kind.as_deref(), Some("rust-target"));
        assert!(entries[0].is_visible_candidate);
    }

    #[test]
    fn sort_entries_puts_parent_first_then_size_desc() {
        let mut entries = vec![
            BrowserEntry {
                path: "/tmp/b".into(),
                name: "b".into(),
                size_bytes: 10,
                reclaimable_bytes: 10,
                entry_kind: EntryKind::Directory,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
                candidate_kind: None,
                is_visible_candidate: false,
            },
            BrowserEntry::parent("/tmp".into()),
            BrowserEntry {
                path: "/tmp/a".into(),
                name: "a".into(),
                size_bytes: 99,
                reclaimable_bytes: 99,
                entry_kind: EntryKind::Directory,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
                candidate_kind: None,
                is_visible_candidate: false,
            },
        ];

        sort_entries(&mut entries);

        assert_eq!(entries[0].entry_kind, EntryKind::Parent);
        assert_eq!(entries[1].name, "a");
        assert_eq!(entries[2].name, "b");
    }
}

fn render(frame: &mut ratatui::Frame, app: &BrowserApp) {
    let area = frame.area();
    let [header, body, footer] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .areas(area);

    let horizontal = if area.width >= 110 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(body)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(body)
    };
    let left = horizontal[0];
    let right = horizontal[1];

    frame.render_widget(render_header(&app.state), header);
    frame.render_widget(
        render_list(&app.state, &app.icon_mode, &app.loading_paths, app.spinner_tick),
        left,
    );
    frame.render_widget(render_context(&app.state), right);
    frame.render_widget(render_footer(&app.state), footer);

    if !matches!(app.state.delete_state(), DeleteState::Idle) {
        let popup = centered_rect(area, 70, 45);
        frame.render_widget(Clear, popup);
        frame.render_widget(render_delete_dialog(app.state.delete_state()), popup);
    }
}

fn render_header(state: &AppState) -> Paragraph<'static> {
    let branch = state
        .current_git_context()
        .branch_name
        .clone()
        .map(|branch| format!("branch:{branch}"))
        .unwrap_or_else(|| "branch:—".into());
    let title = Line::from(vec![
        Span::styled(" artix ", Style::default().fg(Color::Black).bg(Color::Cyan)),
        Span::raw(format!(" {}", state.current_dir().display())),
        Span::raw("  "),
        Span::styled(
            format!(" {} ", branch),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!(" filter:{} ", state.filter_mode().label()),
            Style::default().fg(Color::Green),
        ),
    ]);

    Paragraph::new(title).block(Block::default().borders(Borders::ALL).title("Location"))
}

fn render_list(
    state: &AppState,
    icon_mode: &theme::IconMode,
    loading_paths: &HashSet<PathBuf>,
    spinner_tick: usize,
) -> List<'static> {
    let selected_path = state.selected_entry().map(|entry| entry.path);
    let items = state
        .visible_entries()
        .into_iter()
        .map(|entry| {
            let is_selected = selected_path
                .as_ref()
                .is_some_and(|path| path == &entry.path);

            let size_label = if loading_paths.contains(&entry.path)
                && !matches!(entry.entry_kind, EntryKind::Parent)
            {
                spinner_label(spinner_tick).to_string()
            } else {
                human_bytes(entry.reclaimable_bytes)
            };

            let mut spans = vec![
                Span::styled(
                    format!("{:>8} ", size_label),
                    theme::size_style(),
                ),
            ];

            // Icon + Name as a single span for consistent styling
            let display_name = if icon_mode.is_fancy() {
                format!("{}  {}", theme::icon_for_entry(&entry), entry.name)
            } else {
                entry.name.clone()
            };
            spans.push(Span::styled(
                display_name,
                theme::name_style(&entry, is_selected),
            ));

            if let Some(kind) = &entry.candidate_kind {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{kind}]"),
                    theme::candidate_badge_style(),
                ));
            }

            let git_label = if !matches!(entry.entry_kind, EntryKind::Parent) {
                match entry.git_status {
                    GitStatus::Ignored => Some("ignored"),
                    GitStatus::Tracked => Some("tracked"),
                    GitStatus::Untracked => Some("untracked"),
                    GitStatus::Unknown => Some("unknown"),
                }
            } else {
                None
            };
            if let Some(label) = git_label {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{label}]"),
                    theme::git_status_style(&entry.git_status),
                ));
            }

            if entry
                .git_context
                .worktree_root
                .as_ref()
                .is_some_and(|root| root == &entry.path)
                || entry
                    .git_context
                    .repo_root
                    .as_ref()
                    .is_some_and(|root| root == &entry.path)
            {
                if let Some(branch) = &entry.git_context.branch_name {
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        format!("<{branch}>"),
                        theme::branch_style(),
                    ));
                }
            }

            let style = if is_selected {
                Style::default().bg(Color::Blue).fg(Color::White)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(spans)).style(style)
        })
        .collect::<Vec<_>>();

    List::new(items).block(Block::default().borders(Borders::ALL).title("Browser"))
}

fn render_context(state: &AppState) -> Paragraph<'static> {
    let lines = if let Some(entry) = state.selected_entry() {
        vec![
            Line::raw(format!("path: {}", entry.path.display())),
            Line::raw(format!("size: {}", human_bytes(entry.size_bytes))),
            Line::raw(format!(
                "reclaimable: {}",
                human_bytes(entry.reclaimable_bytes)
            )),
            Line::raw(format!("git: {:?}", entry.git_status)),
            Line::raw(format!(
                "repo root: {}",
                entry
                    .git_context
                    .repo_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "—".into())
            )),
            Line::raw(format!(
                "worktree: {}",
                entry
                    .git_context
                    .worktree_root
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "—".into())
            )),
            Line::raw(format!(
                "branch: {}",
                entry.git_context.branch_name.unwrap_or_else(|| "—".into())
            )),
            Line::raw(format!(
                "candidate: {}",
                entry.candidate_kind.unwrap_or_else(|| "directory".into())
            )),
        ]
    } else {
        vec![Line::raw("No entry selected")]
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Context"))
        .wrap(Wrap { trim: true })
}

fn render_footer(state: &AppState) -> Paragraph<'static> {
    let hint = match state.delete_state() {
        DeleteState::Idle => "q quit | j/k move | enter open | h back | f filter | d delete",
        DeleteState::Confirming { .. } => "t trash | x permanent | esc cancel",
        DeleteState::AwaitingExtraConfirmation { .. } => "y confirm dangerous delete | esc cancel",
        DeleteState::Running { .. } => "running delete...",
        DeleteState::Failed { .. } => "esc dismiss",
    };
    Paragraph::new(hint).block(Block::default().borders(Borders::ALL).title("Keys"))
}

fn render_delete_dialog(state: &DeleteState) -> Paragraph<'static> {
    let lines = match state {
        DeleteState::Confirming {
            entry,
            requires_extra_confirmation,
            ..
        } => vec![
            Line::raw(format!("Delete {}", entry.path.display())),
            Line::raw(format!("git status: {:?}", entry.git_status)),
            Line::raw(format!(
                "risk: {}",
                if *requires_extra_confirmation {
                    "tracked/unknown, permanent delete needs extra confirmation"
                } else {
                    "cleanup candidate"
                }
            )),
            Line::raw("t: move to trash"),
            Line::raw("x: permanent delete"),
        ],
        DeleteState::AwaitingExtraConfirmation { entry, .. } => vec![
            Line::raw(format!("Dangerous delete for {}", entry.path.display())),
            Line::raw("This target is tracked or unknown."),
            Line::raw("Press y to confirm permanent delete."),
        ],
        DeleteState::Running { entry, .. } => {
            vec![Line::raw(format!("Deleting {} ...", entry.path.display()))]
        }
        DeleteState::Failed { message } => vec![Line::raw(message.clone())],
        DeleteState::Idle => vec![Line::raw("")],
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Delete"))
        .wrap(Wrap { trim: true })
}

fn centered_rect(
    area: ratatui::layout::Rect,
    width_percent: u16,
    height_percent: u16,
) -> ratatui::layout::Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area)[1];

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical)[1]
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{bytes}{}", UNITS[unit])
    } else {
        format!("{value:.1}{}", UNITS[unit])
    }
}

fn spinner_label(tick: usize) -> &'static str {
    const FRAMES: [&str; 4] = [".", "..", "...", "...."];
    FRAMES[tick % FRAMES.len()]
}
