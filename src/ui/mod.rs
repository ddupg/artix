use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::classify::git::resolve_git_context;
use crate::delete::DeleteMode;
use crate::delete_flow::{delete_intent_for, execute_delete};
use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, Project};
use crate::scan::browse_directory;

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

    pub fn finish_delete_success(&mut self, message: String) {
        self.delete_state = DeleteState::Succeeded { message };
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

pub fn run_tui(start_dir: PathBuf) -> Result<(), String> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, start_dir);
    ratatui::restore();
    result.map_err(|err| err.to_string())
}

fn run_app(terminal: &mut ratatui::DefaultTerminal, start_dir: PathBuf) -> io::Result<()> {
    let mut app = BrowserApp::new(start_dir).map_err(io::Error::other)?;

    loop {
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
}

impl BrowserApp {
    fn new(start_dir: PathBuf) -> Result<Self, String> {
        let entries = browse_directory(&start_dir, &start_dir)?;
        let mut cache = HashMap::new();
        cache.insert(start_dir.clone(), entries.clone());

        Ok(Self {
            state: AppState::new(start_dir.clone(), entries),
            root_dir: start_dir,
            cache,
        })
    }

    fn enter_selected(&mut self) -> Result<(), String> {
        let Some(entry) = self.state.selected_entry() else {
            return Ok(());
        };
        self.load_directory(entry.path)
    }

    fn enter_parent(&mut self) -> Result<(), String> {
        // Don't allow going above the root directory
        if self.state.current_dir() == self.root_dir {
            return Ok(());
        }
        let Some(parent) = self.state.current_dir().parent().map(Path::to_path_buf) else {
            return Ok(());
        };
        self.load_directory(parent)
    }

    fn load_directory(&mut self, dir: PathBuf) -> Result<(), String> {
        let entries = if let Some(entries) = self.cache.get(&dir) {
            entries.clone()
        } else {
            let entries = browse_directory(&dir, &self.root_dir)?;
            self.cache.insert(dir.clone(), entries.clone());
            entries
        };
        self.state.replace_entries(dir, entries);
        Ok(())
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
                match execute_delete(&entry, mode) {
                    Ok(message) => {
                        self.invalidate_related_paths(&entry.path);
                        self.load_directory(self.state.current_dir().to_path_buf())?;
                        self.state.finish_delete_success(message);
                    }
                    Err(err) => self.state.finish_delete_failure(err),
                }
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
        match execute_delete(&entry, mode) {
            Ok(message) => {
                self.invalidate_related_paths(&entry.path);
                self.load_directory(self.state.current_dir().to_path_buf())?;
                self.state.finish_delete_success(message);
            }
            Err(err) => self.state.finish_delete_failure(err),
        }
        Ok(())
    }

    fn invalidate_related_paths(&mut self, path: &Path) {
        self.cache.retain(|cached_dir, _| {
            !(path.starts_with(cached_dir.as_path()) || cached_dir.starts_with(path))
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

    frame.render_widget(render_header(&app.state), header);
    frame.render_widget(render_list(&app.state), left);
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

fn render_list(state: &AppState) -> List<'static> {
    let selected_path = state.selected_entry().map(|entry| entry.path);
    let items = state
        .visible_entries()
        .into_iter()
        .map(|entry| {
            let mut spans = vec![
                Span::styled(
                    format!("{:>8} ", human_bytes(entry.reclaimable_bytes)),
                    Style::default().fg(Color::Magenta),
                ),
                Span::raw(entry.name.clone()),
            ];
            if let Some(kind) = &entry.candidate_kind {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{kind}]"),
                    Style::default().fg(Color::LightBlue),
                ));
            }
            let git_label = match entry.git_status {
                GitStatus::Ignored => Some(("ignored", Color::Green)),
                GitStatus::Tracked => Some(("tracked", Color::DarkGray)),
                GitStatus::Untracked => Some(("untracked", Color::Yellow)),
                GitStatus::Unknown => Some(("unknown", Color::Red)),
            };
            if let Some((label, color)) = git_label {
                spans.push(Span::raw(" "));
                spans.push(Span::styled(
                    format!("[{label}]"),
                    Style::default().fg(color),
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
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            }

            let style = if selected_path
                .as_ref()
                .is_some_and(|path| path == &entry.path)
            {
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
        DeleteState::Succeeded { .. } | DeleteState::Failed { .. } => "esc dismiss",
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
        DeleteState::Succeeded { message } => vec![Line::raw(message.clone())],
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
