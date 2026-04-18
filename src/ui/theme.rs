use ratatui::style::{Color, Modifier, Style};

use crate::model::{BrowserEntry, EntryKind, GitStatus, RiskLevel};

// ---------------------------------------------------------------------------
// Icon mode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct IconMode {
    fancy: bool,
}

impl IconMode {
    pub fn from_enabled(enabled: bool) -> Self {
        Self { fancy: enabled }
    }

    pub fn is_fancy(&self) -> bool {
        self.fancy
    }
}

// ---------------------------------------------------------------------------
// Nerd Font codepoints
// ---------------------------------------------------------------------------

mod nerd {
    pub const PARENT: &str = "\u{f062}"; // FA arrow-up
    pub const FOLDER: &str = "\u{f115}"; // FA folder-open
    pub const GIT: &str = "\u{f1d3}"; // FA git-square
    pub const GITHUB: &str = "\u{f408}"; // FA github
    pub const CONFIG: &str = "\u{f013}"; // FA cog
    pub const CACHE: &str = "\u{f017}"; // FA clock
    pub const RUST: &str = "\u{e7a8}"; // Devicons rust
    pub const NODE: &str = "\u{e718}"; // Devicons nodejs-small
    pub const PYTHON: &str = "\u{e73c}"; // Devicons python
    pub const CODE: &str = "\u{f121}"; // FA code
    pub const BOOK: &str = "\u{f02d}"; // FA book
    pub const CUBE: &str = "\u{f1b2}"; // FA cube
    pub const HOME: &str = "\u{f015}"; // FA home
    pub const LOCK: &str = "\u{f023}"; // FA lock
    pub const KEY: &str = "\u{f084}"; // FA key
    pub const DOCKER: &str = "\u{f308}"; // FA docker
    pub const DATABASE: &str = "\u{f1c0}"; // FA database
}

// ---------------------------------------------------------------------------
// Icon lookup
// ---------------------------------------------------------------------------

pub fn icon_for_entry(entry: &BrowserEntry) -> &'static str {
    if matches!(entry.entry_kind, EntryKind::Parent) {
        return nerd::PARENT;
    }
    if let Some(kind) = &entry.candidate_kind
        && let Some(icon) = icon_by_candidate_kind(kind)
    {
        return icon;
    }
    if let Some(icon) = icon_by_name(&entry.name) {
        return icon;
    }
    nerd::FOLDER
}

fn icon_by_candidate_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "rust-target" => Some(nerd::RUST),
        "node-modules" => Some(nerd::NODE),
        "python-venv" => Some(nerd::PYTHON),
        _ => None,
    }
}

fn icon_by_name(name: &str) -> Option<&'static str> {
    match name {
        // VCS
        ".git" => Some(nerd::GIT),
        ".github" => Some(nerd::GITHUB),
        // Config / toolchain
        ".config" => Some(nerd::CONFIG),
        ".cache" => Some(nerd::CACHE),
        ".cargo" | ".rustup" => Some(nerd::RUST),
        ".npm" | ".yarn" | ".pnpm-store" => Some(nerd::NODE),
        ".ssh" => Some(nerd::KEY),
        ".docker" => Some(nerd::DOCKER),
        // Source dirs
        "src" | "lib" | "pkg" | "bin" | "scripts" => Some(nerd::CODE),
        // Test dirs
        "test" | "tests" | "spec" | "specs" | "__tests__" => Some(nerd::BOOK),
        // Docs
        "docs" | "doc" => Some(nerd::BOOK),
        // Build output
        "build" | "dist" | "out" => Some(nerd::CUBE),
        // Data
        "data" | "db" => Some(nerd::DATABASE),
        // Home
        "home" => Some(nerd::HOME),
        // Lock files (as directories, e.g. nix store paths)
        _ if name.ends_with("-lock") => Some(nerd::LOCK),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Color constants (ANSI 256-color palette)
// ---------------------------------------------------------------------------

mod colors {
    use ratatui::style::Color;

    // Directory colors
    pub const DIR_DEFAULT: Color = Color::Indexed(33); // DodgerBlue1 #0087ff
    pub const DIR_HIDDEN: Color = Color::Indexed(245); // Grey78 #afafaf
    pub const DIR_SOURCE: Color = Color::Indexed(75); // DeepSkyBlue3 #5fafd7
    pub const DIR_TEST: Color = Color::Indexed(150); // DarkOliveGreen3 #afd787
    pub const DIR_BUILD: Color = Color::Indexed(180); // LightSteelBlue3 #d7d7af

    // Candidate colors (by language)
    pub const CAND_RUST: Color = Color::Indexed(215); // LightSalmon1 #ffaf5f
    pub const CAND_NODE: Color = Color::Indexed(113); // MediumSpringGreen #87d75f
    pub const CAND_PYTHON: Color = Color::Indexed(220); // Gold1 #ffd700

    // Parent
    pub const PARENT: Color = Color::DarkGray;

    // Other UI elements
    pub const SIZE: Color = Color::Indexed(183); // Thistle1 #d7afff
    pub const BADGE: Color = Color::Indexed(153); // LightSteelBlue #afd7ff
    pub const BRANCH: Color = Color::Indexed(51); // Cyan1 #00ffff

    // Git status
    pub const GIT_IGNORED: Color = Color::Indexed(245); // Grey
    pub const GIT_TRACKED: Color = Color::Indexed(40); // Green3 #00d700
    pub const GIT_UNTRACKED: Color = Color::Indexed(220); // Gold1 #ffd700
    pub const GIT_UNKNOWN: Color = Color::Indexed(244); // Grey66
}

// ---------------------------------------------------------------------------
// Style functions
// ---------------------------------------------------------------------------

/// Style for the icon + name span.
/// When selected, returns default style so the ListItem's selection highlight
/// (White on Blue) takes effect without color conflicts.
pub fn name_style(entry: &BrowserEntry, is_selected: bool) -> Style {
    if is_selected {
        return Style::default();
    }
    let color = name_color(entry);
    let mut style = Style::default().fg(color);
    if matches!(entry.entry_kind, EntryKind::CleanupCandidate) {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

fn name_color(entry: &BrowserEntry) -> Color {
    if matches!(entry.entry_kind, EntryKind::Parent) {
        return colors::PARENT;
    }

    // Candidate kinds get language-specific colors
    if let Some(kind) = &entry.candidate_kind {
        return match kind.as_str() {
            "rust-target" => colors::CAND_RUST,
            "node-modules" => colors::CAND_NODE,
            "python-venv" => colors::CAND_PYTHON,
            _ => colors::DIR_DEFAULT,
        };
    }

    // Named directories get semantic colors
    if let Some(color) = directory_color(&entry.name) {
        return color;
    }

    // Hidden directories (starting with .) get muted color
    if entry.name.starts_with('.') {
        return colors::DIR_HIDDEN;
    }

    // Risk-level fallback for non-candidate dirs
    match entry.risk_level {
        RiskLevel::Low => colors::DIR_DEFAULT,
        RiskLevel::Medium => colors::DIR_DEFAULT,
        RiskLevel::Hidden => colors::DIR_HIDDEN,
    }
}

fn directory_color(name: &str) -> Option<Color> {
    match name {
        // Source dirs — bright blue
        "src" | "lib" | "pkg" | "bin" | "scripts" => Some(colors::DIR_SOURCE),
        // Test dirs — green
        "test" | "tests" | "spec" | "specs" | "__tests__" => Some(colors::DIR_TEST),
        // Build output — muted blue
        "build" | "dist" | "out" => Some(colors::DIR_BUILD),
        // Docs — green
        "docs" | "doc" => Some(colors::DIR_TEST),
        _ => None,
    }
}

/// Style for the reclaimable size span.
pub fn size_style() -> Style {
    Style::default().fg(colors::SIZE)
}

/// Style for the [candidate-kind] badge.
pub fn candidate_badge_style() -> Style {
    Style::default().fg(colors::BADGE)
}

/// Style for the [git-status] badge.
pub fn git_status_style(status: &GitStatus) -> Style {
    match status {
        GitStatus::Ignored => Style::default().fg(colors::GIT_IGNORED),
        GitStatus::Tracked => Style::default().fg(colors::GIT_TRACKED),
        GitStatus::Untracked => Style::default().fg(colors::GIT_UNTRACKED),
        GitStatus::Unknown => Style::default().fg(colors::GIT_UNKNOWN),
    }
}

/// Style for the <branch-name> span.
pub fn branch_style() -> Style {
    Style::default()
        .fg(colors::BRANCH)
        .add_modifier(Modifier::BOLD)
}
