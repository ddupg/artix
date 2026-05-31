mod ops;
mod theme;

use std::collections::HashMap;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};

use crate::classify::git::resolve_git_context;
use crate::clean::{
    CleanPlan, CleanRunSummary, ProjectProfile, detect_project_profile, execute_clean_plan,
};
use crate::config::AppContext;
use crate::delete::DeleteMode;
use crate::delete_flow::{delete_intent_for, execute_delete_with_config};
use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, Project};
use crate::rules::default_rules;
use crate::scan::entry as browser_entries;

use tokio::sync::mpsc;
use tokio::task::JoinSet;

pub use crate::delete_flow::{DeleteIntent, DeleteState, DeleteTargetKind};

const CLEAN_FINISHED_DISMISS_AFTER: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanState {
    Idle,
    Confirming {
        plan: CleanPlan,
    },
    Running {
        plan: CleanPlan,
    },
    Finished {
        summary: CleanRunSummary,
        dismiss_at: Instant,
    },
}

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
    current_project_profile: Option<ProjectProfile>,
    entries: Vec<BrowserEntry>,
    filter_mode: FilterMode,
    selected_index: usize,
    delete_state: DeleteState,
    clean_state: CleanState,
}

impl AppState {
    pub fn new(current_dir: PathBuf, entries: Vec<BrowserEntry>) -> Self {
        Self {
            current_git_context: resolve_git_context(&current_dir).unwrap_or_default(),
            current_project_profile: None,
            current_dir,
            entries,
            filter_mode: FilterMode::All,
            selected_index: 0,
            delete_state: DeleteState::Idle,
            clean_state: CleanState::Idle,
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
        self.current_project_profile = None;
        self.current_dir = current_dir;
        self.entries = entries;
        self.selected_index = 0;
    }

    pub fn replace_entries_preserving_selection(
        &mut self,
        current_dir: PathBuf,
        entries: Vec<BrowserEntry>,
        selected_path: Option<&Path>,
    ) {
        self.replace_entries(current_dir, entries);
        self.restore_selection_by_path(selected_path);
    }

    pub fn filter_mode(&self) -> FilterMode {
        self.filter_mode
    }

    pub fn current_git_context(&self) -> &GitContext {
        &self.current_git_context
    }

    pub fn current_project_profile(&self) -> Option<&ProjectProfile> {
        self.current_project_profile.as_ref()
    }

    pub fn set_current_project_profile(&mut self, profile: Option<ProjectProfile>) {
        self.current_project_profile = profile;
        if !self.can_clean_current_dir() && !matches!(self.clean_state, CleanState::Finished { .. })
        {
            self.clean_state = CleanState::Idle;
        }
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
            self.selected_index = if self.selected_index + 1 >= len {
                0
            } else {
                self.selected_index + 1
            };
        }
    }

    pub fn move_selection_up(&mut self) {
        let len = self.visible_entries().len();
        if len == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = if self.selected_index == 0 {
                len.saturating_sub(1)
            } else {
                self.selected_index.saturating_sub(1)
            };
        }
    }

    pub fn jump_to_first(&mut self) {
        self.selected_index = 0;
    }

    pub fn jump_to_last(&mut self) {
        let len = self.visible_entries().len();
        self.selected_index = if len == 0 { 0 } else { len - 1 };
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

    pub fn clean_state(&self) -> &CleanState {
        &self.clean_state
    }

    pub fn can_clean_current_dir(&self) -> bool {
        self.current_project_profile
            .as_ref()
            .is_some_and(ProjectProfile::can_clean)
    }

    pub fn request_clean_current_dir(&mut self) {
        if !matches!(self.delete_state, DeleteState::Idle)
            || !matches!(
                self.clean_state,
                CleanState::Idle | CleanState::Finished { .. }
            )
        {
            return;
        }
        let Some(profile) = &self.current_project_profile else {
            return;
        };
        if !profile.can_clean() {
            return;
        }
        self.clean_state = CleanState::Confirming {
            plan: profile.clean_plan.clone(),
        };
    }

    pub fn set_clean_running(&mut self) {
        if let CleanState::Confirming { plan } = &self.clean_state {
            self.clean_state = CleanState::Running { plan: plan.clone() };
        }
    }

    pub fn finish_clean(&mut self, summary: CleanRunSummary, now: Instant) {
        self.clean_state = CleanState::Finished {
            summary,
            dismiss_at: now + CLEAN_FINISHED_DISMISS_AFTER,
        };
    }

    pub fn dismiss_expired_clean_result(&mut self, now: Instant) {
        if let CleanState::Finished { dismiss_at, .. } = &self.clean_state
            && now >= *dismiss_at
        {
            self.clean_state = CleanState::Idle;
        }
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

    pub fn clear_transient_state(&mut self) {
        self.delete_state = DeleteState::Idle;
        self.clean_state = CleanState::Idle;
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

    fn restore_selection_by_path(&mut self, selected_path: Option<&Path>) {
        let Some(selected_path) = selected_path else {
            self.clamp_selection();
            return;
        };

        let visible = self.visible_entries();
        if let Some(idx) = visible.iter().position(|entry| entry.path == selected_path) {
            self.selected_index = idx;
        } else {
            self.clamp_selection();
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
    let ctx = AppContext::default();
    run_tui_with_context(start_dir, ctx).await
}

pub async fn run_tui_with_context(start_dir: PathBuf, ctx: AppContext) -> Result<(), String> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, start_dir, ctx);
    ratatui::restore();
    result.map_err(|err| err.to_string())
}

fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    start_dir: PathBuf,
    ctx: AppContext,
) -> io::Result<()> {
    let mut app = BrowserApp::new(start_dir, ctx).map_err(io::Error::other)?;

    loop {
        app.pump_background();
        app.state.dismiss_expired_clean_result(Instant::now());
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
            KeyCode::Char('g') => app.state.jump_to_first(),
            KeyCode::Char('G') => app.state.jump_to_last(),
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                app.enter_selected().map_err(io::Error::other)?;
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                app.enter_parent().map_err(io::Error::other)?;
            }
            KeyCode::Char('f') => app.state.cycle_filter_mode(),
            KeyCode::Char('d') => app.state.request_delete_for_selected(),
            KeyCode::Char('x') => app.state.request_clean_current_dir(),
            KeyCode::Char('t') => app
                .run_delete(DeleteMode::Trash)
                .map_err(io::Error::other)?,
            KeyCode::Char('y') => {
                app.confirm_action().map_err(io::Error::other)?;
            }
            KeyCode::Esc => app.state.clear_transient_state(),
            _ => {}
        }
    }

    Ok(())
}

#[derive(Debug)]
struct BrowserApp {
    state: AppState,
    ctx: AppContext,
    root_dir: PathBuf,
    cache: HashMap<PathBuf, Vec<BrowserEntry>>,
    profile_cache: HashMap<PathBuf, Option<ProjectProfile>>,
    icon_mode: theme::IconMode,
    ops: ops::OperationTracker,

    bg_tx: mpsc::UnboundedSender<BgRequest>,
    bg_rx: mpsc::UnboundedReceiver<BgResponse>,

    loading_paths: HashSet<PathBuf>,
    spinner_tick: usize,
}

#[derive(Debug)]
enum BgRequest {
    LoadDirectory {
        request_id: u64,
        dir: PathBuf,
    },
    Delete {
        request_id: u64,
        entry: Box<BrowserEntry>,
        mode: DeleteMode,
    },
    Clean {
        request_id: u64,
        plan: CleanPlan,
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
        entry: Box<BrowserEntry>,
    },
    ProjectProfileLoaded {
        request_id: u64,
        dir: PathBuf,
        result: Result<Option<ProjectProfile>, String>,
    },
    DeleteFinished {
        request_id: u64,
        entry_path: PathBuf,
        result: Result<String, String>,
    },
    CleanFinished {
        request_id: u64,
        project_root: PathBuf,
        summary: CleanRunSummary,
    },
}

impl BrowserApp {
    fn new(start_dir: PathBuf, ctx: AppContext) -> Result<Self, String> {
        let (bg_tx, mut bg_req_rx) = mpsc::unbounded_channel::<BgRequest>();
        let (bg_resp_tx, bg_rx) = mpsc::unbounded_channel::<BgResponse>();

        let root_dir = start_dir.clone();
        let bg_ctx = ctx.clone();
        let icon_mode = theme::IconMode::from_enabled(ctx.config().ui.icons);
        tokio::spawn(async move {
            let mut active_load: Option<tokio::task::JoinHandle<()>> = None;
            while let Some(req) = bg_req_rx.recv().await {
                match req {
                    BgRequest::LoadDirectory { request_id, dir } => {
                        if let Some(handle) = active_load.take() {
                            handle.abort();
                        }
                        let root_dir = root_dir.clone();
                        let bg_resp_tx = bg_resp_tx.clone();
                        let ctx = bg_ctx.clone();
                        let entry_limit = ctx.config().performance.tui_entry_concurrency;
                        active_load = Some(tokio::spawn(async move {
                            // Send a quick listing first (0B sizes) so UI can populate even if
                            // the in-thread quick path failed.
                            let initial = quick_browse_directory(&dir, &root_dir, &ctx);
                            let _ = bg_resp_tx.send(BgResponse::DirectoryLoaded {
                                request_id,
                                dir: dir.clone(),
                                result: initial,
                            });

                            let profile_tx = bg_resp_tx.clone();
                            let profile_scan_dir = dir.clone();
                            let profile_response_dir = dir.clone();
                            tokio::spawn(async move {
                                let profile_result = tokio::task::spawn_blocking(move || {
                                    detect_project_profile(&profile_scan_dir)
                                })
                                .await
                                .map_err(|err| err.to_string())
                                .and_then(|result| result);
                                let _ = profile_tx.send(BgResponse::ProjectProfileLoaded {
                                    request_id,
                                    dir: profile_response_dir,
                                    result: profile_result,
                                });
                            });

                            // Then progressively compute size/git per entry and stream updates.
                            let rules = default_rules();
                            let Ok(seeds) = browser_entries::read_browser_entry_seeds(&dir, &rules)
                            else {
                                return;
                            };
                            let current_context = resolve_git_context(&dir);

                            let mut jobs = JoinSet::new();
                            for seed in seeds {
                                while jobs.len() >= entry_limit {
                                    if let Some(res) = jobs.join_next().await
                                        && let Ok(entry) = res
                                    {
                                        let _ = bg_resp_tx.send(BgResponse::EntryUpdated {
                                            request_id,
                                            dir: dir.clone(),
                                            entry: Box::new(entry),
                                        });
                                    }
                                }

                                let current_context = current_context.clone();
                                let ctx = ctx.clone();
                                jobs.spawn(async move {
                                    seed.into_enriched(
                                        current_context,
                                        &ctx,
                                        browser_entries::EntrySizeMode::Budgeted,
                                    )
                                    .await
                                });
                            }

                            while let Some(res) = jobs.join_next().await {
                                if let Ok(entry) = res {
                                    let _ = bg_resp_tx.send(BgResponse::EntryUpdated {
                                        request_id,
                                        dir: dir.clone(),
                                        entry: Box::new(entry),
                                    });
                                }
                            }
                        }));
                    }
                    BgRequest::Delete {
                        request_id,
                        entry,
                        mode,
                    } => {
                        let bg_resp_tx = bg_resp_tx.clone();
                        let delete_config = bg_ctx.config().delete;
                        tokio::spawn(async move {
                            let entry_path = entry.path.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                execute_delete_with_config(&entry, mode, &delete_config)
                            })
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
                    BgRequest::Clean { request_id, plan } => {
                        let bg_resp_tx = bg_resp_tx.clone();
                        tokio::spawn(async move {
                            let project_root = plan.project_root.clone();
                            let summary = execute_clean_plan(plan).await;
                            let _ = bg_resp_tx.send(BgResponse::CleanFinished {
                                request_id,
                                project_root,
                                summary,
                            });
                        });
                    }
                }
            }
        });

        let mut app = Self {
            state: AppState::new(start_dir.clone(), Vec::new()),
            ctx,
            root_dir: start_dir.clone(),
            cache: HashMap::new(),
            profile_cache: HashMap::new(),
            icon_mode,
            ops: ops::OperationTracker::new(),

            bg_tx,
            bg_rx,

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
                    if !self.ops.is_pending_load(request_id) {
                        continue;
                    }

                    match result {
                        Ok(entries) => {
                            self.cache.insert(dir.clone(), entries.clone());
                            if self.state.current_dir() == dir.as_path()
                                && self.state.entries.is_empty()
                            {
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
                    let entry = *entry;
                    if !self.ops.is_pending_load(request_id) {
                        continue;
                    }
                    if self.state.current_dir() != dir.as_path() {
                        continue;
                    }

                    if let Some(entries) = self.cache.get_mut(&dir) {
                        browser_entries::apply_browser_entry_update(entries, &entry);
                        browser_entries::sort_browser_entries_by_size(entries);
                    }
                    browser_entries::apply_browser_entry_update(&mut self.state.entries, &entry);
                    self.resort_visible_entries_preserving_selection();
                    self.loading_paths.remove(&entry.path);
                }
                BgResponse::ProjectProfileLoaded {
                    request_id,
                    dir,
                    result,
                } => {
                    if !self.ops.is_pending_load(request_id) {
                        continue;
                    }
                    let profile = result.unwrap_or(None);
                    self.profile_cache.insert(dir.clone(), profile.clone());
                    if self.state.current_dir() == dir.as_path() {
                        self.state.set_current_project_profile(profile);
                    }
                }
                BgResponse::DeleteFinished {
                    request_id,
                    entry_path,
                    result,
                } => {
                    if !self.ops.finish_delete(request_id) {
                        continue;
                    }

                    match result {
                        Ok(_message) => {
                            ops::invalidate_related_paths(&mut self.cache, &entry_path);
                            let current = self.state.current_dir().to_path_buf();
                            self.state.clear_delete_state();
                            self.load_directory(current);
                        }
                        Err(err) => self.state.finish_delete_failure(err),
                    }
                }
                BgResponse::CleanFinished {
                    request_id,
                    project_root,
                    summary,
                } => {
                    if !self.ops.finish_clean(request_id) {
                        continue;
                    }
                    ops::invalidate_related_paths(&mut self.cache, &project_root);
                    self.profile_cache.remove(&project_root);
                    self.state.finish_clean(summary, Instant::now());
                    let current = self.state.current_dir().to_path_buf();
                    let selected_path = self.state.selected_entry().map(|entry| entry.path);
                    self.load_directory_preserving_selection(current, selected_path);
                }
            }
        }
    }

    fn enter_selected(&mut self) -> Result<(), String> {
        let Some(entry) = self.state.selected_entry() else {
            return Ok(());
        };
        if matches!(entry.entry_kind, EntryKind::File) {
            return Ok(());
        }
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
        self.load_directory_preserving_selection(dir, None);
    }

    fn load_directory_preserving_selection(
        &mut self,
        dir: PathBuf,
        selected_path: Option<PathBuf>,
    ) {
        self.loading_paths.clear();
        if self.state.current_dir() != dir.as_path() {
            self.state.clean_state = CleanState::Idle;
        }
        if let Some(entries) = self.cache.get(&dir).cloned() {
            self.state.replace_entries_preserving_selection(
                dir.clone(),
                entries,
                selected_path.as_deref(),
            );
            let profile = self
                .profile_cache
                .get(self.state.current_dir())
                .cloned()
                .unwrap_or(None);
            self.state.set_current_project_profile(profile);
            self.loading_paths.extend(
                self.state
                    .entries()
                    .iter()
                    .filter(|entry| !matches!(entry.entry_kind, EntryKind::Parent))
                    .filter(|entry| entry.size_bytes == 0 && entry.git_status == GitStatus::Unknown)
                    .map(|entry| entry.path.clone()),
            );
            if self.profile_cache.contains_key(self.state.current_dir()) {
                return;
            }
        }

        // Provide a fast placeholder listing so the UI is immediately usable.
        if let Ok(entries) = quick_browse_directory(&dir, &self.root_dir, &self.ctx) {
            self.cache.insert(dir.clone(), entries.clone());
            self.state.replace_entries_preserving_selection(
                dir.clone(),
                entries,
                selected_path.as_deref(),
            );
            self.loading_paths.extend(
                self.state
                    .entries()
                    .iter()
                    .filter(|entry| !matches!(entry.entry_kind, EntryKind::Parent))
                    .map(|entry| entry.path.clone()),
            );
        } else {
            // Optimistically switch directory; entries will be populated asynchronously.
            self.state.replace_entries_preserving_selection(
                dir.clone(),
                Vec::new(),
                selected_path.as_deref(),
            );
        }

        let request_id = self.ops.start_load();
        let _ = self
            .bg_tx
            .send(BgRequest::LoadDirectory { request_id, dir });
    }

    fn resort_visible_entries_preserving_selection(&mut self) {
        let selected_path = self.state.selected_entry().map(|entry| entry.path);
        browser_entries::sort_browser_entries_by_size(&mut self.state.entries);

        let Some(selected_path) = selected_path else {
            self.state.clamp_selection();
            return;
        };

        let visible = self.state.visible_entries();
        if let Some(idx) = visible.iter().position(|entry| entry.path == selected_path) {
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

                let request_id = self.ops.start_delete();
                let _ = self.bg_tx.send(BgRequest::Delete {
                    request_id,
                    entry: Box::new(entry),
                    mode,
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn confirm_action(&mut self) -> Result<(), String> {
        match self.state.clean_state() {
            CleanState::Confirming { plan } => {
                let plan = plan.clone();
                self.state.set_clean_running();
                let request_id = self.ops.start_clean();
                let _ = self.bg_tx.send(BgRequest::Clean { request_id, plan });
                Ok(())
            }
            _ if matches!(
                self.state.delete_state(),
                DeleteState::AwaitingExtraConfirmation { .. }
            ) =>
            {
                self.confirm_extra_delete()
            }
            _ if matches!(self.state.delete_state(), DeleteState::Confirming { .. }) => {
                self.run_delete(DeleteMode::Permanent { confirmed: true })
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

        let request_id = self.ops.start_delete();
        let _ = self.bg_tx.send(BgRequest::Delete {
            request_id,
            entry: Box::new(entry),
            mode,
        });
        Ok(())
    }
}

fn quick_browse_directory(
    path: &Path,
    root_dir: &Path,
    _ctx: &AppContext,
) -> Result<Vec<BrowserEntry>, String> {
    let rules = default_rules();
    let current_context = resolve_git_context(path);
    let mut entries = Vec::new();

    if let Some(parent) = browser_entries::parent_entry_for(path, root_dir) {
        entries.push(parent);
    }

    entries.extend(
        browser_entries::read_browser_entry_seeds(path, &rules)?
            .into_iter()
            .map(|seed| seed.into_placeholder(current_context.clone())),
    );
    browser_entries::sort_placeholder_entries(&mut entries);

    Ok(entries)
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
    render_list(
        frame,
        left,
        &app.state,
        &app.icon_mode,
        &app.loading_paths,
        app.spinner_tick,
    );
    frame.render_widget(render_context(&app.state), right);
    render_footer(frame, footer, &app.state);

    if !matches!(app.state.delete_state(), DeleteState::Idle) {
        let popup = centered_rect(area, 70, 45);
        frame.render_widget(Clear, popup);
        frame.render_widget(render_delete_dialog(app.state.delete_state()), popup);
    } else if !matches!(app.state.clean_state(), CleanState::Idle) {
        let popup = centered_rect(area, 70, 45);
        frame.render_widget(Clear, popup);
        frame.render_widget(render_clean_dialog(app.state.clean_state()), popup);
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
    frame: &mut ratatui::Frame,
    area: Rect,
    state: &AppState,
    icon_mode: &theme::IconMode,
    loading_paths: &HashSet<PathBuf>,
    spinner_tick: usize,
) {
    let block = Block::default().borders(Borders::ALL).title("Browser");
    frame.render_widget(block.clone(), area);

    let inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let visible = state.visible_entries();
    let len = visible.len();
    let selected_index = if len == 0 {
        0
    } else {
        state.selected_index.min(len - 1)
    };

    // Draw "x of y" on the list border (inside the frame line).
    render_list_counter(frame, area, selected_index, len);
    let selected_path = visible.get(selected_index).map(|entry| entry.path.clone());

    let viewport_len = inner.height as usize;
    let scroll_offset = compute_scroll_offset(len, selected_index, viewport_len);

    let needs_scrollbar = len > viewport_len && viewport_len > 0 && inner.width > 1;
    let list_area = if needs_scrollbar {
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width.saturating_sub(1),
            height: inner.height,
        }
    } else {
        inner
    };
    let scrollbar_area = Rect {
        x: inner.x + inner.width.saturating_sub(1),
        y: inner.y,
        width: 1,
        height: inner.height,
    };

    let start = scroll_offset.min(len);
    let end = (start + viewport_len).min(len);
    let items = visible[start..end]
        .iter()
        .map(|entry| {
            let is_selected = selected_path
                .as_ref()
                .is_some_and(|path| path == &entry.path);
            list_item_for_entry(entry, is_selected, icon_mode, loading_paths, spinner_tick)
        })
        .collect::<Vec<_>>();

    frame.render_widget(List::new(items), list_area);

    if needs_scrollbar {
        let mut sb_state = ScrollbarState::new(len)
            .viewport_content_length(viewport_len)
            // In ratatui, the thumb is computed from `[position, position + viewport_len]`.
            // Using the selected index keeps the thumb in sync with the cursor: when the cursor
            // reaches the last item, the thumb reaches the bottom.
            .position(selected_index);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(sb, scrollbar_area, &mut sb_state);
    }
}

fn render_list_counter(frame: &mut ratatui::Frame, area: Rect, selected_index: usize, len: usize) {
    if area.width < 3 || area.height < 2 {
        return;
    }

    let x = if len == 0 {
        0
    } else {
        selected_index.saturating_add(1)
    };
    let y = len;
    let counter = format!("{x} of {y}");
    let label = format!(" {counter} ");

    // Render over the bottom border line, excluding the corners.
    let border_inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(area.height.saturating_sub(1)),
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(label).alignment(Alignment::Center),
        border_inner,
    );
}

fn list_item_for_entry(
    entry: &BrowserEntry,
    is_selected: bool,
    icon_mode: &theme::IconMode,
    loading_paths: &HashSet<PathBuf>,
    spinner_tick: usize,
) -> ListItem<'static> {
    let size_label =
        if loading_paths.contains(&entry.path) && !matches!(entry.entry_kind, EntryKind::Parent) {
            spinner_label(spinner_tick).to_string()
        } else {
            let mut label = human_bytes(entry.reclaimable_bytes);
            if !entry.size_complete && !matches!(entry.entry_kind, EntryKind::Parent) {
                label.push('~');
            }
            label
        };

    let mut spans = vec![Span::styled(
        format!("{:>8} ", size_label),
        theme::size_style(),
    )];

    let display_name = if icon_mode.is_fancy() {
        format!("{}  {}", theme::icon_for_entry(entry), entry.name)
    } else {
        entry.name.clone()
    };
    spans.push(Span::styled(
        display_name,
        theme::name_style(entry, is_selected),
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

    if (entry
        .git_context
        .worktree_root
        .as_ref()
        .is_some_and(|root| root == &entry.path)
        || entry
            .git_context
            .repo_root
            .as_ref()
            .is_some_and(|root| root == &entry.path))
        && let Some(branch) = &entry.git_context.branch_name
    {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("<{branch}>"), theme::branch_style()));
    }

    let style = if is_selected {
        Style::default().bg(Color::Blue).fg(Color::White)
    } else {
        Style::default()
    };
    ListItem::new(Line::from(spans)).style(style)
}

fn compute_scroll_offset(len: usize, selected_index: usize, viewport_len: usize) -> usize {
    if viewport_len == 0 || len <= viewport_len {
        return 0;
    }

    let selected_index = selected_index.min(len.saturating_sub(1));
    let half = viewport_len / 2;
    let desired = selected_index.saturating_sub(half);
    let max_offset = len.saturating_sub(viewport_len);
    desired.min(max_offset)
}

fn render_context(state: &AppState) -> Paragraph<'static> {
    let lines = if let Some(entry) = state.selected_entry() {
        let size_label = if entry.size_complete {
            human_bytes(entry.size_bytes)
        } else {
            format!("{} (partial)", human_bytes(entry.size_bytes))
        };
        let reclaimable_label = if entry.size_complete {
            human_bytes(entry.reclaimable_bytes)
        } else {
            format!("{} (partial)", human_bytes(entry.reclaimable_bytes))
        };
        let mut lines = vec![
            Line::raw(format!("path: {}", entry.path.display())),
            Line::raw(format!("size: {}", size_label)),
            Line::raw(format!("reclaimable: {}", reclaimable_label)),
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
                entry.candidate_kind.unwrap_or_else(|| {
                    if matches!(entry.entry_kind, EntryKind::File) {
                        "file".into()
                    } else {
                        "directory".into()
                    }
                })
            )),
        ];

        if let Some(profile) = state.current_project_profile() {
            lines.push(Line::raw(format!("project: {}", profile.kind_label())));
            lines.push(Line::raw(format!(
                "languages: {}",
                profile.language_label()
            )));
            let clean_label = if profile.can_clean() {
                profile
                    .clean_plan
                    .commands
                    .iter()
                    .map(|command| command.display())
                    .collect::<Vec<_>>()
                    .join("; ")
            } else {
                "none".to_string()
            };
            lines.push(Line::raw(format!("clean: {clean_label}")));
        } else {
            lines.push(Line::raw("project: —"));
            lines.push(Line::raw("languages: —"));
            lines.push(Line::raw("clean: —"));
        }

        lines
    } else {
        vec![Line::raw("No entry selected")]
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Context"))
        .wrap(Wrap { trim: true })
}

fn render_footer(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let hint = match (state.delete_state(), state.clean_state()) {
        (DeleteState::Idle, CleanState::Idle) => {
            if state.can_clean_current_dir() {
                "q quit | j/k move | enter open | h back | f filter | d delete | x clean"
            } else {
                "q quit | j/k move | enter open | h back | f filter | d delete"
            }
        }
        (DeleteState::Confirming { .. }, _) => "t trash | y permanent | esc cancel",
        (DeleteState::AwaitingExtraConfirmation { .. }, _) => {
            "y confirm dangerous delete | esc cancel"
        }
        (DeleteState::Running { .. }, _) => "running delete...",
        (DeleteState::Failed { .. }, _) => "esc dismiss",
        (DeleteState::Idle, CleanState::Confirming { .. }) => "y run clean | esc cancel",
        (DeleteState::Idle, CleanState::Running { .. }) => "running clean...",
        (DeleteState::Idle, CleanState::Finished { .. }) => "clean result closes automatically",
    };

    let block = Block::default().borders(Borders::ALL).title("Keys");
    frame.render_widget(block.clone(), area);

    let inner = block.inner(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    frame.render_widget(Paragraph::new(hint), inner);
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
            Line::raw("y: permanent delete"),
            Line::raw("esc: cancel"),
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

fn render_clean_dialog(state: &CleanState) -> Paragraph<'static> {
    let lines = match state {
        CleanState::Confirming { plan } => {
            let mut lines = vec![
                Line::raw(format!("Clean {}", plan.project_root.display())),
                Line::raw("Commands:"),
            ];
            for command in &plan.commands {
                lines.push(Line::raw(format!("  {}", command.display())));
            }
            lines.push(Line::raw(""));
            lines.push(Line::raw("y: run clean"));
            lines.push(Line::raw("esc: cancel"));
            lines
        }
        CleanState::Running { plan } => {
            let first = plan
                .commands
                .first()
                .map(|command| command.display())
                .unwrap_or_else(|| "clean".to_string());
            vec![Line::raw(format!("Running {first} ..."))]
        }
        CleanState::Finished {
            summary,
            dismiss_at,
        } => vec![
            Line::raw(summary.message()),
            Line::raw(format!(
                "Closing in {}s",
                clean_result_remaining_secs(*dismiss_at, Instant::now())
            )),
        ],
        CleanState::Idle => vec![Line::raw("")],
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Clean"))
        .wrap(Wrap { trim: true })
}

fn clean_result_remaining_secs(dismiss_at: Instant, now: Instant) -> u64 {
    let remaining = dismiss_at.saturating_duration_since(now);
    if remaining.is_zero() {
        0
    } else {
        remaining.as_secs().saturating_add(1)
    }
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

#[cfg(test)]
mod tests {
    use super::quick_browse_directory;
    use crate::config::AppContext;
    use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel};
    use crate::scan::entry as browser_entries;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn apply_entry_update_updates_matching_path_in_place() {
        let mut entries = vec![BrowserEntry {
            path: "/tmp/a".into(),
            name: "a".into(),
            size_bytes: 0,
            reclaimable_bytes: 0,
            size_complete: true,
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
            size_complete: true,
            entry_kind: EntryKind::CleanupCandidate,
            git_status: GitStatus::Ignored,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Low,
            candidate_kind: Some("rust-target".into()),
            is_visible_candidate: true,
        };

        browser_entries::apply_browser_entry_update(&mut entries, &update);

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
                size_complete: true,
                entry_kind: EntryKind::Directory,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
                candidate_kind: None,
                is_visible_candidate: false,
            },
            BrowserEntry {
                path: "/tmp/big.log".into(),
                name: "big.log".into(),
                size_bytes: 250,
                reclaimable_bytes: 250,
                size_complete: true,
                entry_kind: EntryKind::File,
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
                size_complete: true,
                entry_kind: EntryKind::Directory,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
                candidate_kind: None,
                is_visible_candidate: false,
            },
        ];

        browser_entries::sort_browser_entries_by_size(&mut entries);

        assert_eq!(entries[0].entry_kind, EntryKind::Parent);
        assert_eq!(entries[1].name, "big.log");
        assert_eq!(entries[1].entry_kind, EntryKind::File);
        assert_eq!(entries[2].name, "a");
        assert_eq!(entries[3].name, "b");
    }

    #[test]
    fn quick_browse_directory_sorts_files_dirs_and_candidates_without_cycle() {
        let temp = tempdir().expect("tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join("target")).expect("create target");
        fs::create_dir_all(root.join("aaa")).expect("create directory");
        fs::write(root.join("mmm.log"), "artifact").expect("write file");

        let entries =
            quick_browse_directory(root, root, &AppContext::default()).expect("quick browse");
        let names = entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.entry_kind.clone()))
            .collect::<Vec<_>>();

        assert_eq!(names[0], ("target", EntryKind::CleanupCandidate));
        assert_eq!(names[1], ("aaa", EntryKind::Directory));
        assert_eq!(names[2], ("mmm.log", EntryKind::File));
    }
}
