mod browser_list;
mod load;
mod ops;
mod theme;

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Wrap,
};

use crate::candidate::descriptor_for;
use crate::classify::git::resolve_git_context;
use crate::clean::{
    CleanPlan, CleanRunSummary, ProjectProfile, detect_project_profile, execute_clean_plan,
};
use crate::clean_flow::{CleanFlow, CleanRequest};
use crate::config::AppContext;
use crate::delete::DeleteMode;
use crate::delete_flow::{DeleteFlow, DeleteRequest, execute_delete_with_config};
use crate::git_storage::{
    GitGcResult, GitStorageAnalysis, GitStorageTarget, analyze_git_storage, execute_git_gc,
};
use crate::git_storage_flow::{GitStorageFlow, GitStorageRequest};
use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, Project};
use browser_list::{BrowserList, BrowserListSnapshot};
use load::{DirectoryLoadEvent, DirectoryLoads, WorkerCommand};

use tokio::sync::mpsc;
use tokio::task::JoinSet;

pub use crate::clean_flow::CleanState;
pub use crate::delete_flow::DeleteState;
pub use crate::git_storage_flow::GitStorageState;
pub use browser_list::FilterMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewRow {
    pub project_name: String,
    pub reclaimable_bytes: u64,
    pub candidate_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    current_dir: PathBuf,
    current_git_context: GitContext,
    current_project_profile: Option<ProjectProfile>,
    browser_list: BrowserList,
    delete_flow: DeleteFlow,
    clean_flow: CleanFlow,
    git_storage: Option<GitStorageView>,
    git_storage_flow: GitStorageFlow,
}

impl AppState {
    pub fn new(current_dir: PathBuf, entries: Vec<BrowserEntry>) -> Self {
        Self {
            current_git_context: resolve_git_context(&current_dir).unwrap_or_default(),
            current_project_profile: None,
            current_dir,
            browser_list: BrowserList::new(entries),
            delete_flow: DeleteFlow::default(),
            clean_flow: CleanFlow::default(),
            git_storage: None,
            git_storage_flow: GitStorageFlow::default(),
        }
    }

    pub fn current_dir(&self) -> &Path {
        &self.current_dir
    }

    pub fn entries(&self) -> &[BrowserEntry] {
        self.browser_list.entries()
    }

    pub fn replace_entries(&mut self, current_dir: PathBuf, entries: Vec<BrowserEntry>) {
        self.switch_directory(current_dir);
        self.browser_list.reset(entries);
    }

    pub fn replace_entries_preserving_selection(
        &mut self,
        current_dir: PathBuf,
        entries: Vec<BrowserEntry>,
        selected_path: Option<&Path>,
    ) {
        self.switch_directory(current_dir);
        self.browser_list
            .reset_preserving_selection(entries, selected_path);
    }

    pub fn filter_mode(&self) -> FilterMode {
        self.browser_list.filter_mode()
    }

    pub fn current_git_context(&self) -> &GitContext {
        &self.current_git_context
    }

    pub fn current_project_profile(&self) -> Option<&ProjectProfile> {
        self.current_project_profile.as_ref()
    }

    pub fn set_current_project_profile(&mut self, profile: Option<ProjectProfile>) {
        self.current_project_profile = profile;
        if !self.can_clean_current_dir() {
            self.clean_flow.cancel_confirmation();
        }
    }

    pub fn set_filter_mode(&mut self, filter_mode: FilterMode) {
        self.browser_list.set_filter_mode(filter_mode);
    }

    pub fn cycle_filter_mode(&mut self) {
        self.browser_list.cycle_filter_mode();
    }

    pub fn visible_entries(&self) -> Vec<BrowserEntry> {
        self.browser_list
            .snapshot()
            .entries()
            .iter()
            .copied()
            .cloned()
            .collect()
    }

    pub fn selected_entry(&self) -> Option<BrowserEntry> {
        self.browser_list.selected().cloned()
    }

    pub fn move_selection_down(&mut self) {
        self.browser_list.move_next();
    }

    pub fn move_selection_up(&mut self) {
        self.browser_list.move_previous();
    }

    pub fn jump_to_first(&mut self) {
        self.browser_list.jump_first();
    }

    pub fn jump_to_last(&mut self) {
        self.browser_list.jump_last();
    }

    pub fn request_delete_for_selected(&mut self) {
        if !matches!(self.clean_flow.state(), CleanState::Idle)
            || !matches!(self.git_storage_flow.state(), GitStorageState::Idle)
        {
            return;
        }
        let Some(entry) = self.selected_entry() else {
            return;
        };
        if matches!(entry.entry_kind, EntryKind::GitStorage) {
            return;
        }
        self.delete_flow.request(entry);
    }

    pub fn delete_state(&self) -> &DeleteState {
        self.delete_flow.state()
    }

    pub fn clean_state(&self) -> &CleanState {
        self.clean_flow.state()
    }

    pub fn git_storage_state(&self) -> &GitStorageState {
        self.git_storage_flow.state()
    }

    pub fn can_clean_current_dir(&self) -> bool {
        self.current_project_profile
            .as_ref()
            .is_some_and(ProjectProfile::can_clean)
    }

    pub fn request_clean_current_dir(&mut self) {
        if !matches!(self.delete_flow.state(), DeleteState::Idle)
            || !matches!(self.clean_flow.state(), CleanState::Idle)
            || !matches!(self.git_storage_flow.state(), GitStorageState::Idle)
        {
            return;
        }
        if self
            .selected_entry_ref()
            .is_some_and(|entry| matches!(entry.entry_kind, EntryKind::GitStorage))
        {
            if let Some(GitStorageView::Available(analysis)) = &self.git_storage {
                self.git_storage_flow.request(analysis.clone());
            }
            return;
        }
        let Some(profile) = &self.current_project_profile else {
            return;
        };
        self.clean_flow.request(profile.clean_plan.clone());
    }

    pub fn confirm_clean(&mut self) -> Option<CleanRequest> {
        self.clean_flow.confirm()
    }

    pub fn finish_clean(&mut self, summary: CleanRunSummary) -> Option<PathBuf> {
        self.clean_flow.finish(summary)
    }

    fn confirm_git_gc(&mut self) -> Option<GitStorageRequest> {
        self.git_storage_flow.confirm()
    }

    fn finish_git_gc(&mut self, result: GitGcResult) -> Option<GitStorageTarget> {
        self.git_storage_flow.finish(result)
    }

    fn choose_trash_delete(&mut self) -> Option<DeleteRequest> {
        self.delete_flow.choose_trash()
    }

    fn confirm_permanent_delete(&mut self) -> Option<DeleteRequest> {
        self.delete_flow.confirm_permanent()
    }

    fn finish_delete(&mut self, result: Result<String, String>) -> bool {
        self.delete_flow.finish(result)
    }

    pub fn clear_transient_state(&mut self) {
        self.delete_flow.cancel();
        self.clean_flow.cancel();
        self.git_storage_flow.cancel();
    }

    fn selected_entry_ref(&self) -> Option<&BrowserEntry> {
        self.browser_list.selected()
    }

    fn list_snapshot(&self) -> BrowserListSnapshot<'_> {
        self.browser_list.snapshot()
    }

    fn refresh_entries(&mut self, entries: Vec<BrowserEntry>) {
        self.browser_list.refresh(entries);
    }

    fn switch_directory(&mut self, current_dir: PathBuf) {
        self.current_git_context = resolve_git_context(&current_dir).unwrap_or_default();
        self.current_project_profile = None;
        self.clean_flow.cancel();
        self.git_storage = None;
        self.git_storage_flow.cancel();
        self.current_dir = current_dir;
    }

    fn set_git_storage(&mut self, view: Option<GitStorageView>) {
        self.git_storage = view;
    }

    fn git_storage_analysis(&self) -> Option<&GitStorageAnalysis> {
        match &self.git_storage {
            Some(GitStorageView::Available(analysis)) => Some(analysis),
            Some(GitStorageView::Pending(_)) | Some(GitStorageView::Unavailable { .. }) | None => {
                None
            }
        }
    }

    fn git_storage_error(&self) -> Option<&str> {
        match &self.git_storage {
            Some(GitStorageView::Unavailable { message, .. }) => Some(message),
            _ => None,
        }
    }

    fn git_storage_is_loading(&self) -> bool {
        matches!(self.git_storage, Some(GitStorageView::Pending(_)))
    }

    fn can_gc_selected(&self) -> bool {
        self.selected_entry_ref()
            .is_some_and(|entry| matches!(entry.entry_kind, EntryKind::GitStorage))
            && self.git_storage_analysis().is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GitStorageView {
    Pending(GitStorageTarget),
    Available(GitStorageAnalysis),
    Unavailable {
        target: GitStorageTarget,
        message: String,
    },
}

impl GitStorageView {
    fn entry(&self) -> BrowserEntry {
        match self {
            Self::Pending(target) | Self::Unavailable { target, .. } => target.placeholder_entry(),
            Self::Available(analysis) => analysis.browser_entry(),
        }
    }

    fn retarget(&self, target: GitStorageTarget) -> Self {
        match self {
            Self::Pending(_) => Self::Pending(target),
            Self::Available(analysis) => {
                let mut analysis = analysis.clone();
                analysis.target = target;
                Self::Available(analysis)
            }
            Self::Unavailable { message, .. } => Self::Unavailable {
                target,
                message: message.clone(),
            },
        }
    }
}

pub fn build_overview_rows(mut projects: Vec<Project>) -> Vec<OverviewRow> {
    projects.sort_by_key(|project| std::cmp::Reverse(project.reclaimable_bytes));

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
            KeyCode::Char('t') => app.run_trash_delete().map_err(io::Error::other)?,
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
    root_dir: PathBuf,
    loads: DirectoryLoads,
    icon_mode: theme::IconMode,
    ops: ops::OperationTracker,
    git_storage_cache: HashMap<PathBuf, GitStorageCacheEntry>,
    next_git_storage_request_id: u64,

    bg_tx: mpsc::UnboundedSender<BgRequest>,
    bg_rx: mpsc::UnboundedReceiver<BgResponse>,

    spinner_tick: usize,
}

#[derive(Debug)]
enum BgRequest {
    LoadDirectory(Box<load::DirectoryLoadRequest>),
    CancelDirectoryLoad,
    Delete {
        request_id: u64,
        entry: Box<BrowserEntry>,
        mode: DeleteMode,
    },
    Clean {
        request_id: u64,
        plan: CleanPlan,
    },
    AnalyzeGitStorage {
        request_id: u64,
        target: GitStorageTarget,
    },
    GitGc {
        request_id: u64,
        target: GitStorageTarget,
    },
}

#[derive(Debug)]
enum BgResponse {
    DirectoryLoad(DirectoryLoadEvent),
    DeleteFinished {
        request_id: u64,
        entry_path: PathBuf,
        result: Result<String, String>,
    },
    CleanFinished {
        request_id: u64,
        summary: CleanRunSummary,
    },
    GitStorageAnalyzed {
        request_id: u64,
        target: GitStorageTarget,
        result: Box<Result<GitStorageAnalysis, String>>,
    },
    GitGcFinished {
        request_id: u64,
        result: GitGcResult,
    },
}

#[derive(Debug, Clone)]
struct GitStorageCacheEntry {
    request_id: u64,
    view: GitStorageView,
}

impl BrowserApp {
    fn new(start_dir: PathBuf, ctx: AppContext) -> Result<Self, String> {
        let (bg_tx, mut bg_req_rx) = mpsc::unbounded_channel::<BgRequest>();
        let (bg_resp_tx, bg_rx) = mpsc::unbounded_channel::<BgResponse>();

        let bg_ctx = ctx.clone();
        let icon_mode = theme::IconMode::from_enabled(ctx.config().ui.icons);
        tokio::spawn(async move {
            let mut active_load: Option<tokio::task::JoinHandle<()>> = None;
            while let Some(req) = bg_req_rx.recv().await {
                match req {
                    BgRequest::LoadDirectory(request) => {
                        if let Some(handle) = active_load.take() {
                            handle.abort();
                        }
                        let (token, dir, seeds, current_context) = (*request).into_parts();
                        let ctx = bg_ctx.clone();
                        let entry_limit = ctx.config().performance.tui_entry_concurrency;
                        let load_tx = bg_resp_tx.clone();
                        active_load = Some(tokio::spawn(async move {
                            let entry_tx = load_tx.clone();
                            let entry_dir = dir.clone();
                            let enrichment = async move {
                                let mut jobs = JoinSet::new();
                                for seed in seeds {
                                    while jobs.len() >= entry_limit {
                                        if let Some(res) = jobs.join_next().await
                                            && let Ok(entry) = res
                                        {
                                            let _ = entry_tx.send(BgResponse::DirectoryLoad(
                                                DirectoryLoadEvent::EntryUpdated {
                                                    token,
                                                    dir: entry_dir.clone(),
                                                    entry: Box::new(entry),
                                                },
                                            ));
                                        }
                                    }

                                    let current_context = current_context.clone();
                                    let ctx = ctx.clone();
                                    jobs.spawn(async move {
                                        seed.into_enriched(current_context, &ctx).await
                                    });
                                }

                                while let Some(res) = jobs.join_next().await {
                                    if let Ok(entry) = res {
                                        let _ = entry_tx.send(BgResponse::DirectoryLoad(
                                            DirectoryLoadEvent::EntryUpdated {
                                                token,
                                                dir: entry_dir.clone(),
                                                entry: Box::new(entry),
                                            },
                                        ));
                                    }
                                }
                                let _ = entry_tx.send(BgResponse::DirectoryLoad(
                                    DirectoryLoadEvent::EntriesFinished {
                                        token,
                                        dir: entry_dir,
                                    },
                                ));
                            };

                            let profile_tx = load_tx;
                            let profile_scan_dir = dir.clone();
                            let profile = async move {
                                let profile_result = tokio::task::spawn_blocking(move || {
                                    detect_project_profile(&profile_scan_dir)
                                })
                                .await
                                .map_err(|err| err.to_string())
                                .and_then(|result| result);
                                let _ = profile_tx.send(BgResponse::DirectoryLoad(
                                    DirectoryLoadEvent::ProfileFinished {
                                        token,
                                        dir,
                                        result: profile_result,
                                    },
                                ));
                            };

                            tokio::join!(enrichment, profile);
                        }));
                    }
                    BgRequest::CancelDirectoryLoad => {
                        if let Some(handle) = active_load.take() {
                            handle.abort();
                        }
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
                            let summary = execute_clean_plan(plan).await;
                            let _ = bg_resp_tx.send(BgResponse::CleanFinished {
                                request_id,
                                summary,
                            });
                        });
                    }
                    BgRequest::AnalyzeGitStorage { request_id, target } => {
                        let bg_resp_tx = bg_resp_tx.clone();
                        let ctx = bg_ctx.clone();
                        tokio::spawn(async move {
                            let result = analyze_git_storage(target.clone(), &ctx).await;
                            let _ = bg_resp_tx.send(BgResponse::GitStorageAnalyzed {
                                request_id,
                                target,
                                result: Box::new(result),
                            });
                        });
                    }
                    BgRequest::GitGc { request_id, target } => {
                        let bg_resp_tx = bg_resp_tx.clone();
                        let ctx = bg_ctx.clone();
                        tokio::spawn(async move {
                            let result = execute_git_gc(target, &ctx).await;
                            let _ =
                                bg_resp_tx.send(BgResponse::GitGcFinished { request_id, result });
                        });
                    }
                }
            }
        });

        let mut app = Self {
            state: AppState::new(start_dir.clone(), Vec::new()),
            root_dir: start_dir.clone(),
            loads: DirectoryLoads::new(),
            icon_mode,
            ops: ops::OperationTracker::new(),
            git_storage_cache: HashMap::new(),
            next_git_storage_request_id: 1,

            bg_tx,
            bg_rx,

            spinner_tick: 0,
        };

        app.load_directory(start_dir);
        Ok(app)
    }

    fn pump_background(&mut self) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            match msg {
                BgResponse::DirectoryLoad(event) => {
                    let dir = event.dir().to_path_buf();
                    if self.loads.apply(event) && self.state.current_dir() == dir.as_path() {
                        self.sync_directory_view(&dir);
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

                    if self.state.finish_delete(result) {
                        self.loads.invalidate_related(&entry_path);
                        let current = self.state.current_dir().to_path_buf();
                        self.load_directory(current);
                    }
                }
                BgResponse::CleanFinished {
                    request_id,
                    summary,
                } => {
                    if !self.ops.finish_clean(request_id) {
                        continue;
                    }
                    let Some(project_root) = self.state.finish_clean(summary) else {
                        continue;
                    };
                    self.loads.invalidate_related(&project_root);
                    let current = self.state.current_dir().to_path_buf();
                    self.load_directory(current);
                }
                BgResponse::GitStorageAnalyzed {
                    request_id,
                    target,
                    result,
                } => {
                    let Some(cached) = self.git_storage_cache.get_mut(&target.common_dir) else {
                        continue;
                    };
                    if cached.request_id != request_id {
                        continue;
                    }
                    cached.view = match *result {
                        Ok(analysis) => GitStorageView::Available(analysis),
                        Err(message) => GitStorageView::Unavailable { target, message },
                    };
                    let current = self.state.current_dir().to_path_buf();
                    self.sync_directory_view(&current);
                }
                BgResponse::GitGcFinished { request_id, result } => {
                    if !self.ops.finish_git_gc(request_id) {
                        continue;
                    }
                    let Some(target) = self.state.finish_git_gc(result) else {
                        continue;
                    };
                    self.git_storage_cache.remove(&target.common_dir);
                    let current = self.state.current_dir().to_path_buf();
                    self.loads.invalidate_related(&current);
                    self.load_directory(current);
                }
            }
        }
    }

    fn enter_selected(&mut self) -> Result<(), String> {
        let Some(entry) = self.state.selected_entry() else {
            return Ok(());
        };
        if matches!(entry.entry_kind, EntryKind::File | EntryKind::GitStorage) {
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
        let command = self.loads.open(dir.clone(), &self.root_dir);
        self.ensure_git_storage_analysis(&dir);
        self.sync_directory_view(&dir);
        let request = match command {
            WorkerCommand::Start(request) => BgRequest::LoadDirectory(request),
            WorkerCommand::Cancel => BgRequest::CancelDirectoryLoad,
        };
        let _ = self.bg_tx.send(request);
    }

    fn sync_directory_view(&mut self, dir: &Path) {
        let mut entries = self.loads.entries(dir).unwrap_or_default().to_vec();
        let git_storage = self.git_storage_view_for(dir);
        if let Some(view) = &git_storage {
            entries.push(view.entry());
            crate::scan::entry::sort_browser_entries_by_size(&mut entries);
        }
        if self.state.current_dir() == dir {
            self.state.refresh_entries(entries);
        } else {
            self.state.replace_entries(dir.to_path_buf(), entries);
        }
        self.state.set_git_storage(git_storage);
        self.state
            .set_current_project_profile(self.loads.profile(dir).cloned());
    }

    fn ensure_git_storage_analysis(&mut self, dir: &Path) {
        let context = resolve_git_context(dir).unwrap_or_default();
        let Some(target) = GitStorageTarget::for_repo_root(dir, &context) else {
            return;
        };
        if self.git_storage_cache.contains_key(&target.common_dir) {
            return;
        }

        let request_id = self.next_git_storage_request_id;
        self.next_git_storage_request_id = self.next_git_storage_request_id.saturating_add(1);
        self.git_storage_cache.insert(
            target.common_dir.clone(),
            GitStorageCacheEntry {
                request_id,
                view: GitStorageView::Pending(target.clone()),
            },
        );
        let _ = self
            .bg_tx
            .send(BgRequest::AnalyzeGitStorage { request_id, target });
    }

    fn git_storage_view_for(&self, dir: &Path) -> Option<GitStorageView> {
        let context = resolve_git_context(dir)?;
        let target = GitStorageTarget::for_repo_root(dir, &context)?;
        self.git_storage_cache
            .get(&target.common_dir)
            .map(|cached| cached.view.retarget(target))
    }

    fn run_trash_delete(&mut self) -> Result<(), String> {
        if let Some(request) = self.state.choose_trash_delete() {
            self.dispatch_delete(request);
        }
        Ok(())
    }

    fn confirm_action(&mut self) -> Result<(), String> {
        if let Some(request) = self.state.confirm_git_gc() {
            self.dispatch_git_gc(request);
        } else if let Some(request) = self.state.confirm_clean() {
            let request_id = self.ops.start_clean();
            let _ = self.bg_tx.send(BgRequest::Clean {
                request_id,
                plan: request.into_plan(),
            });
        } else if let Some(request) = self.state.confirm_permanent_delete() {
            self.dispatch_delete(request);
        }

        Ok(())
    }

    fn dispatch_git_gc(&mut self, request: GitStorageRequest) {
        let request_id = self.ops.start_git_gc();
        let _ = self.bg_tx.send(BgRequest::GitGc {
            request_id,
            target: request.into_target(),
        });
    }

    fn dispatch_delete(&mut self, request: DeleteRequest) {
        let (entry, mode) = request.into_parts();
        let request_id = self.ops.start_delete();
        let _ = self.bg_tx.send(BgRequest::Delete {
            request_id,
            entry: Box::new(entry),
            mode,
        });
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
    let current_dir = app.state.current_dir();

    frame.render_widget(render_header(&app.state), header);
    render_list(
        frame,
        left,
        &app.state,
        &app.icon_mode,
        app.loads.loading_paths(current_dir),
        app.loads.load_error(current_dir),
        app.spinner_tick,
    );
    frame.render_widget(
        render_context(&app.state, app.loads.profile_error(current_dir)),
        right,
    );
    render_footer(frame, footer, &app.state);

    if !matches!(app.state.delete_state(), DeleteState::Idle) {
        let popup = centered_rect(area, 70, 45);
        frame.render_widget(Clear, popup);
        frame.render_widget(render_delete_dialog(app.state.delete_state()), popup);
    } else if !matches!(app.state.clean_state(), CleanState::Idle) {
        let popup = centered_rect(area, 70, 45);
        frame.render_widget(Clear, popup);
        frame.render_widget(render_clean_dialog(app.state.clean_state()), popup);
    } else if !matches!(app.state.git_storage_state(), GitStorageState::Idle) {
        let popup = centered_rect(area, 70, 45);
        frame.render_widget(Clear, popup);
        frame.render_widget(
            render_git_storage_dialog(app.state.git_storage_state()),
            popup,
        );
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
    loading_paths: Option<&HashSet<PathBuf>>,
    load_error: Option<&str>,
    spinner_tick: usize,
) {
    let block = Block::default().borders(Borders::ALL).title("Browser");
    frame.render_widget(block.clone(), area);

    let mut inner = area.inner(Margin {
        vertical: 1,
        horizontal: 1,
    });
    if let Some(error) = load_error
        && inner.height > 0
    {
        let error_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(format!("load failed: {error}")).style(Style::default().fg(Color::Red)),
            error_area,
        );
        inner = Rect {
            x: inner.x,
            y: inner.y.saturating_add(1),
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };
    }
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let snapshot = state.list_snapshot();
    let visible = snapshot.entries();
    let len = visible.len();
    let selected_index = snapshot.selected_index();

    // Draw "x of y" on the list border (inside the frame line).
    render_list_counter(frame, area, selected_index, len);
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
        .enumerate()
        .map(|(offset, entry)| {
            let is_selected = start + offset == selected_index;
            list_item_for_entry(
                entry,
                is_selected,
                icon_mode,
                loading_paths,
                state.git_storage_is_loading(),
                spinner_tick,
            )
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
    loading_paths: Option<&HashSet<PathBuf>>,
    git_storage_loading: bool,
    spinner_tick: usize,
) -> ListItem<'static> {
    let is_loading = (loading_paths.is_some_and(|paths| paths.contains(&entry.path))
        && !matches!(entry.entry_kind, EntryKind::Parent))
        || (git_storage_loading && matches!(entry.entry_kind, EntryKind::GitStorage));
    let size_label = if is_loading {
        spinner_label(spinner_tick).to_string()
    } else {
        let bytes = if matches!(entry.entry_kind, EntryKind::GitStorage) {
            entry.size_bytes
        } else {
            entry.reclaimable_bytes
        };
        let label = human_bytes(bytes);
        if entry.size_status.is_complete() {
            label
        } else {
            format!("~{label}")
        }
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

    if let Some(kind) = entry.entry_kind.candidate_kind() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("[{}]", descriptor_for(kind).label),
            theme::candidate_badge_style(),
        ));
    }
    if matches!(entry.entry_kind, EntryKind::GitStorage) {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            "[git-storage]",
            theme::candidate_badge_style(),
        ));
    }

    let git_label = if !matches!(entry.entry_kind, EntryKind::Parent | EntryKind::GitStorage) {
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

fn render_context(state: &AppState, profile_error: Option<&str>) -> Paragraph<'static> {
    if state
        .selected_entry_ref()
        .is_some_and(|entry| matches!(entry.entry_kind, EntryKind::GitStorage))
    {
        return render_git_storage_context(state);
    }
    let mut lines = if let Some(entry) = state.selected_entry_ref() {
        let size_label = human_bytes(entry.size_bytes);
        let reclaimable_label = human_bytes(entry.reclaimable_bytes);
        let mut lines = vec![
            Line::raw(format!("path: {}", entry.path.display())),
            Line::raw(format!("size: {}", size_label)),
            Line::raw(format!("reclaimable: {}", reclaimable_label)),
            Line::raw(format!(
                "size status: {}",
                if entry.size_status.is_complete() {
                    "complete"
                } else {
                    "incomplete"
                }
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
                entry.git_context.branch_name.as_deref().unwrap_or("—")
            )),
            Line::raw(format!(
                "candidate: {}",
                entry
                    .entry_kind
                    .candidate_kind()
                    .map(|kind| descriptor_for(kind).label)
                    .unwrap_or_else(|| if matches!(entry.entry_kind, EntryKind::File) {
                        "file"
                    } else {
                        "directory"
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

    if let Some(error) = profile_error {
        lines.push(Line::styled(
            format!("project profile unavailable: {error}"),
            Style::default().fg(Color::Red),
        ));
    }

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Context"))
        .wrap(Wrap { trim: true })
}

fn render_footer(frame: &mut ratatui::Frame, area: Rect, state: &AppState) {
    let hint = if !matches!(state.git_storage_state(), GitStorageState::Idle) {
        match state.git_storage_state() {
            GitStorageState::Confirming { .. } => "y run git gc | esc cancel",
            GitStorageState::Running { .. } => "running git gc...",
            GitStorageState::Failed { .. } => "esc dismiss",
            GitStorageState::Idle => unreachable!("checked above"),
        }
    } else {
        match (state.delete_state(), state.clean_state()) {
            (DeleteState::Idle, CleanState::Idle) => {
                if state.can_gc_selected() {
                    "q quit | j/k move | h back | f filter | x git gc"
                } else if state.can_clean_current_dir() {
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
            (DeleteState::Idle, CleanState::Failed { .. }) => "esc dismiss",
        }
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
        DeleteState::AwaitingExtraConfirmation { entry } => vec![
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
        CleanState::Failed { message } => {
            vec![Line::raw(message.clone()), Line::raw("esc: dismiss")]
        }
        CleanState::Idle => vec![Line::raw("")],
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Clean"))
        .wrap(Wrap { trim: true })
}

fn render_git_storage_context(state: &AppState) -> Paragraph<'static> {
    let lines = if let Some(analysis) = state.git_storage_analysis() {
        vec![
            Line::raw(format!(
                "common dir: {}",
                analysis.target.common_dir.display()
            )),
            Line::raw(format!(
                "physical total: {}{}",
                if analysis.total_size_status.is_complete() {
                    ""
                } else {
                    "~"
                },
                human_bytes(analysis.total_size_bytes)
            )),
            Line::raw(format!(
                "pack: {} ({} packs, {} objects)",
                human_bytes(analysis.pack_size_bytes),
                analysis.pack_count,
                analysis.packed_object_count
            )),
            Line::raw(format!(
                "loose: {} ({} objects)",
                human_bytes(analysis.loose_object_size_bytes),
                analysis.loose_object_count
            )),
            Line::raw(format!(
                "garbage: {} ({} files)",
                human_bytes(analysis.garbage_size_bytes),
                analysis.garbage_count
            )),
            Line::raw(format!(
                "prune-packable: {} objects",
                analysis.prune_packable_count
            )),
            Line::raw(format!(
                "Git LFS: {}{} (not cleaned)",
                if analysis.lfs_size_status.is_complete() {
                    ""
                } else {
                    "~"
                },
                human_bytes(analysis.lfs_size_bytes)
            )),
            Line::raw("reclaimable: confirmed after git gc"),
            Line::raw("action: x runs conservative `git gc`"),
        ]
    } else if let Some(error) = state.git_storage_error() {
        vec![
            Line::styled(
                format!("Git storage analysis unavailable: {error}"),
                Style::default().fg(Color::Red),
            ),
            Line::raw("git gc is disabled"),
        ]
    } else {
        vec![Line::raw("Analyzing Git storage ...")]
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Git storage"))
        .wrap(Wrap { trim: true })
}

fn render_git_storage_dialog(state: &GitStorageState) -> Paragraph<'static> {
    let lines = match state {
        GitStorageState::Confirming { analysis } => vec![
            Line::raw(format!("Maintain {}", analysis.target.common_dir.display())),
            Line::raw(format!(
                "identified garbage: {}",
                human_bytes(analysis.garbage_size_bytes)
            )),
            Line::raw("Command: git gc"),
            Line::raw("No --prune=now, --aggressive, or --force."),
            Line::raw(""),
            Line::raw("y: run git gc"),
            Line::raw("esc: cancel"),
        ],
        GitStorageState::Running { target } => vec![Line::raw(format!(
            "Running git gc for {} ...",
            target.common_dir.display()
        ))],
        GitStorageState::Failed { message } => {
            vec![Line::raw(message.clone()), Line::raw("esc: dismiss")]
        }
        GitStorageState::Idle => vec![Line::raw("")],
    };

    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Git GC"))
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

#[cfg(test)]
mod tests {
    use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel, SizeStatus};
    use crate::scan::entry as browser_entries;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::tempdir;

    use super::{AppState, BrowserApp, CleanState, DeleteState, GitStorageState, GitStorageView};
    use crate::config::AppContext;
    use crate::git_storage::{GitStorageAnalysis, GitStorageTarget};
    use crate::model::CandidateKind;

    #[tokio::test]
    async fn background_worker_completes_the_directory_load_session() {
        let temp = tempdir().expect("tempdir");
        fs::write(temp.path().join("README.md"), "hello").expect("file");
        let mut app =
            BrowserApp::new(temp.path().to_path_buf(), AppContext::default()).expect("browser app");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                app.pump_background();
                if app.loads.is_complete(temp.path()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("directory load should complete");

        assert!(app.loads.load_error(temp.path()).is_none());
        assert_eq!(app.state.entries().len(), 1);
        assert_eq!(app.state.entries()[0].name, "README.md");
    }

    #[tokio::test]
    async fn repo_root_streams_a_git_storage_entry() {
        let temp = tempdir().expect("tempdir");
        let status = std::process::Command::new("git")
            .current_dir(temp.path())
            .args(["init", "-q"])
            .status()
            .expect("git available");
        assert!(status.success());
        let mut app =
            BrowserApp::new(temp.path().to_path_buf(), AppContext::default()).expect("browser app");

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                app.pump_background();
                if app.loads.is_complete(temp.path())
                    && app
                        .state
                        .entries()
                        .iter()
                        .any(|entry| matches!(entry.entry_kind, EntryKind::GitStorage))
                    && app.state.git_storage_analysis().is_some()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Git storage analysis should complete");

        let entry = app
            .state
            .entries()
            .iter()
            .find(|entry| matches!(entry.entry_kind, EntryKind::GitStorage))
            .expect("Git storage entry");
        assert_eq!(entry.path, temp.path().join(".git"));
    }

    #[test]
    fn git_storage_entry_cannot_enter_delete_flow() {
        let root = PathBuf::from("/repo");
        let analysis = git_storage_analysis(&root);
        let mut state = AppState::new(root, vec![analysis.browser_entry()]);
        state.set_git_storage(Some(GitStorageView::Available(analysis)));

        state.request_delete_for_selected();

        assert_eq!(state.delete_state(), &DeleteState::Idle);
    }

    #[test]
    fn x_opens_git_gc_instead_of_project_clean_for_git_storage() {
        let root = PathBuf::from("/repo");
        let analysis = git_storage_analysis(&root);
        let mut state = AppState::new(root, vec![analysis.browser_entry()]);
        state.set_git_storage(Some(GitStorageView::Available(analysis)));

        state.request_clean_current_dir();

        assert!(matches!(
            state.git_storage_state(),
            GitStorageState::Confirming { .. }
        ));
        assert_eq!(state.clean_state(), &CleanState::Idle);
    }

    #[test]
    fn shared_storage_cache_retargets_the_current_worktree() {
        let first_root = PathBuf::from("/repo");
        let second_root = PathBuf::from("/worktree");
        let view = GitStorageView::Available(git_storage_analysis(&first_root));
        let mut target = git_storage_analysis(&first_root).target;
        target.repo_root = second_root.clone();
        target.git_context.repo_root = Some(second_root.clone());
        target.git_context.worktree_root = Some(second_root.clone());
        target.git_context.branch_name = Some("feature".to_string());

        let GitStorageView::Available(retargeted) = view.retarget(target) else {
            panic!("available view");
        };

        assert_eq!(retargeted.target.repo_root, second_root);
        assert_eq!(
            retargeted.target.git_context.branch_name.as_deref(),
            Some("feature")
        );
        assert_eq!(retargeted.total_size_bytes, 100);
    }

    #[test]
    fn apply_entry_update_updates_matching_path_in_place() {
        let mut entries = vec![BrowserEntry {
            path: "/tmp/a".into(),
            name: "a".into(),
            size_bytes: 0,
            reclaimable_bytes: 0,
            size_status: SizeStatus::Complete,
            entry_kind: EntryKind::Directory,
            git_status: GitStatus::Unknown,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Hidden,
        }];

        let update = BrowserEntry {
            path: "/tmp/a".into(),
            name: "a".into(),
            size_bytes: 123,
            reclaimable_bytes: 123,
            size_status: SizeStatus::Complete,
            entry_kind: EntryKind::CleanupCandidate(CandidateKind::RustTarget),
            git_status: GitStatus::Ignored,
            git_context: GitContext::default(),
            risk_level: RiskLevel::Low,
        };

        browser_entries::apply_browser_entry_update(&mut entries, &update);

        assert_eq!(entries[0].size_bytes, 123);
        assert_eq!(entries[0].reclaimable_bytes, 123);
        assert_eq!(entries[0].size_status, SizeStatus::Complete);
        assert_eq!(entries[0].git_status, GitStatus::Ignored);
        assert_eq!(entries[0].risk_level, RiskLevel::Low);
        assert_eq!(
            entries[0].entry_kind,
            EntryKind::CleanupCandidate(CandidateKind::RustTarget)
        );
    }

    #[test]
    fn sort_entries_puts_parent_first_then_size_desc() {
        let mut entries = vec![
            BrowserEntry {
                path: "/tmp/b".into(),
                name: "b".into(),
                size_bytes: 10,
                reclaimable_bytes: 10,
                size_status: SizeStatus::Complete,
                entry_kind: EntryKind::Directory,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
            },
            BrowserEntry {
                path: "/tmp/big.log".into(),
                name: "big.log".into(),
                size_bytes: 250,
                reclaimable_bytes: 250,
                size_status: SizeStatus::Complete,
                entry_kind: EntryKind::File,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
            },
            BrowserEntry::parent("/tmp".into()),
            BrowserEntry {
                path: "/tmp/a".into(),
                name: "a".into(),
                size_bytes: 99,
                reclaimable_bytes: 99,
                size_status: SizeStatus::Complete,
                entry_kind: EntryKind::Directory,
                git_status: GitStatus::Unknown,
                git_context: GitContext::default(),
                risk_level: RiskLevel::Hidden,
            },
        ];

        browser_entries::sort_browser_entries_by_size(&mut entries);

        assert_eq!(entries[0].entry_kind, EntryKind::Parent);
        assert_eq!(entries[1].name, "big.log");
        assert_eq!(entries[1].entry_kind, EntryKind::File);
        assert_eq!(entries[2].name, "a");
        assert_eq!(entries[3].name, "b");
    }

    fn git_storage_analysis(root: &Path) -> GitStorageAnalysis {
        let context = GitContext {
            repo_root: Some(root.to_path_buf()),
            common_dir: Some(root.join(".git")),
            worktree_root: Some(root.to_path_buf()),
            ..GitContext::default()
        };
        GitStorageAnalysis {
            target: GitStorageTarget {
                repo_root: root.to_path_buf(),
                common_dir: root.join(".git"),
                git_context: context,
            },
            total_size_bytes: 100,
            total_size_status: SizeStatus::Complete,
            loose_object_count: 1,
            loose_object_size_bytes: 10,
            packed_object_count: 2,
            pack_count: 1,
            pack_size_bytes: 80,
            prune_packable_count: 0,
            garbage_count: 1,
            garbage_size_bytes: 10,
            lfs_size_bytes: 0,
            lfs_size_status: SizeStatus::Complete,
        }
    }
}
