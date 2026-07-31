# AGENTS.md

This file is a high-level, repo-specific architecture overview for agents and contributors.

## 1) Project Overview

`artix` is a developer-workspace disk cleanup tool with an interactive TUI. It focuses on common “rebuildable” directories (e.g. `target/`, `node_modules/`, `.venv/`) and adds Git/worktree context plus a safer delete flow.

The binary has two execution modes (see `src/main.rs`):

- **Interactive TUI:** Runs when `[ui].mode = "tui"`, or when `[ui].mode = "auto"` and stdout is a terminal.
- **Plain text overview:** Runs when `[ui].mode = "plain"`, or when `[ui].mode = "auto"` and stdout is not a terminal. Output format is tab-separated: `<project_name>\t<reclaimable_bytes>\t<candidate_count>` (documented in `README.md`).

### Core boundaries

The library exposes these top-level modules (see `src/lib.rs`):

- `model`: Domain/view models used across scan + UI.
- `candidate`: Typed cleanup-candidate identities and their built-in descriptor catalog.
- `scan`: Workspace scanning and directory browsing.
- `classify`: Git/worktree context, ownership heuristics, and risk classification.
- `ui`: TUI state, rendering, and event loop.
- `clean`: Current-directory project/language detection plus clean command planning and execution.
- `clean_flow`: Clean confirmation, running, completion, and failure state transitions.
- `delete` / `delete_flow`: Delete execution + confirmation state machine.

### Data model

Important types (see `src/model.rs`):

- `BrowserEntry`: The UI list item (dir or cleanup candidate) with size, size completeness, Git status, and context.
- `CandidateDir`: A discovered “cleanup candidate” directory with owner project root, typed candidate kind, size, size completeness, Git status, and risk level.
- `Project`: Aggregated per-project totals (name, reclaimable bytes, candidate count).
- `GitContext`: Repo/worktree roots plus branch/head metadata.
- `ProjectProfile` / `CleanPlan`: Current TUI directory project metadata and the clean commands available for that exact directory.

### Scanning pipeline (plain-text mode)

`scan::scan_workspace` (see `src/scan/mod.rs`) does roughly:

1. **Discover the workspace once**: `scan::discovery` walks the provided roots once to collect project markers and classify directories through the typed candidate catalog (see `src/scan/discovery.rs` and `src/candidate.rs`). Candidate directories and `.git` metadata are traversal boundaries, symlinked directories are not followed, overlapping roots are deduplicated, and each candidate is assigned to its nearest marker root (or nearest CLI root).
2. **Enrich candidates** (async, concurrency-limited):
   - Compute `size_bytes` plus `size_status`; traversal or task failures preserve partial bytes and mark the measurement incomplete.
   - Classify `git_status` and `risk_level`.
3. **Summarize projects**: aggregate candidates into `Project` rows for printing.

### TUI architecture

The TUI loop lives in `ui::run_tui` / `run_app` (see `src/ui/mod.rs`). A `BrowserApp` owns:

- `AppState`: current directory/project context plus clean and delete flows.
- `BrowserList` (see `src/ui/browser_list.rs`): entries, filter policy, cursor movement, and selection preservation across progressive refreshes.
- An in-memory cache mapping `PathBuf -> Vec<BrowserEntry>`.
- A project-profile cache mapping the current directory to optional clean/language metadata.
- A background request/response channel that streams directory load results and per-entry updates.

Directory loading is intentionally two-phase (see `BrowserApp::load_directory` and the background `BgRequest::LoadDirectory` worker in `src/ui/mod.rs`):

- **Quick placeholder listing**: returns entries with `size_bytes = 0` and `git_status = Unknown` so the UI becomes usable immediately.
- **Progressive enrichment**: per-entry tasks compute size + completeness + Git status and send `EntryUpdated` messages; the UI applies updates, resorts, and preserves the selected entry. If an enrichment task ends without an update, its placeholder becomes an incomplete measurement instead of silently looking like a real zero.
- **Project profile enrichment**: the current directory itself is checked for project markers (`Cargo.toml`, `package.json`, `pom.xml`, `build.gradle(.kts)`, `pyproject.toml`) and language summary/clean plan metadata is streamed back separately. This does not walk upward to find a parent project and does not recursively clean child projects.

Deletion is also handled asynchronously:

- UI triggers delete confirmation state transitions (`delete_flow`).
- Actual deletion executes in a blocking task (`execute_delete`) and then the UI invalidates affected cache entries and refreshes the current directory.

Clean is handled asynchronously as a separate TUI state:

- Idle `x` opens a clean confirmation only when the current directory has a detected project profile and a non-empty clean plan.
- `y` confirms clean in the clean dialog; `Esc` cancels/dismisses.
- Clean commands run with `cwd` set to the current directory/project root. Completion invalidates affected cache entries; success closes immediately, while failure remains visible until dismissed.

### Git/worktree context + Git status

Git context is resolved via `gix` (see `classify::git::resolve_git_context` in `src/classify/git.rs`).

For per-path Git status in the UI, `classify_path_git_status` shells out to `git`:

- `git check-ignore -q -- <path>` (ignored)
- `git ls-files --error-unmatch -- <path>` (tracked)

The subprocess output is suppressed and calls are timeout-limited (2 seconds) (see `src/classify/git.rs`).

## 2) Build & Commands

### Local

- Build: `cargo build`
- Run TUI: `cargo run --quiet` (optionally `cargo run --quiet -- /path/to/workspace`)
- Run plain text mode: set `[ui].mode = "plain"` in `config.toml`
- Run tests: `cargo test --all-targets`

### CI / Release

- CI (`.github/workflows/ci.yml`) runs `cargo build --verbose` and `cargo test --verbose`.
- Release (`.github/workflows/release.yml`) triggers on tag `v*`, runs:
  - `cargo test --all-targets --target <target>`
  - `cargo build --locked --release --target <target>`
  - Packages `artix` into a `.tar.gz` and uploads to GitHub Releases.

## 3) Code Style

- Rust edition is **2024** (see `Cargo.toml`).
- No repo-level `rustfmt.toml` is present; formatting follows Rust defaults.
- Public APIs across modules frequently use `Result<T, String>` for error propagation into CLI/TUI layers (e.g. `scan`, `ui`, `delete`).
- The UI uses an explicit “fast placeholder then stream updates” pattern; when changing list behavior, preserve the `BrowserList` invariants for filtered cursor clamping, wrapping, directory reset, and same-directory refresh selection.

## 4) Testing

Tests are written with Rust’s built-in test harness:

- Integration tests live under `tests/` and include both sync `#[test]` and async `#[tokio::test]` cases.
- `tempfile` is used for filesystem fixtures.
- Some tests invoke the system `git` binary (e.g. `tests/git_context_test.rs` creates a worktree); ensure `git` is available in PATH when running tests.
- Some regression tests manipulate environment variables (e.g. `HOME`) and restore them afterwards (see `tests/delete_trash_regression_test.rs`).

## 5) Security

This project can delete directories. Key safety-related behaviors are implemented in code:

- **Permanent delete requires explicit confirmation** at the API level (`DeleteMode::Permanent { confirmed: bool }` in `src/delete.rs` rejects `confirmed=false`).
- **UI delete flow adds stronger confirmation** for `Tracked`/`Unknown` Git status (see `delete_flow::delete_intent_for` in `src/delete_flow.rs`).
- **TUI delete keybindings:** `d` opens delete confirmation, `t` moves to trash, and `y` requests permanent delete; tracked/unknown permanent delete still requires the extra `y` confirmation.
- **Trash delete** uses the `trash` crate and can fall back to a built-in macOS `~/.Trash` move; built-in trash uses `HOME` (see `src/delete.rs`).
- **TUI clean keybinding:** idle `x` is reserved for current-directory clean, not permanent delete. Clean is only available when the current directory itself is a detected project root with an executable clean plan.

Git status classification in the UI depends on executing `git` from PATH (see `src/classify/git.rs`). Subprocess output is suppressed and calls are timeout-limited, but agents should be aware that PATH influences which `git` executable is used.

Clean command execution also depends on tools from PATH or local wrappers (`cargo`, `mvn`/`mvnw`, `gradle`/`gradlew`, package managers). Output is captured for UI summaries; avoid adding silent clean paths.

## 6) Configuration

Configuration is primarily through `config.toml` loaded by `src/config.rs`.

- **Default config path:** `~/.config/artix/config.toml`.
- **Compatibility lookup order:** `~/.config/artix/config.toml`, then `~/.artix/config.toml`.
- **Supported user-facing fields:** `version`, `[ui].mode`, `[ui].icons`, `[performance].fs_concurrency`, `[performance].git_concurrency`, `[performance].tui_entry_concurrency`, `[scan.tui_size_budget].max_entries`, `[scan.tui_size_budget].timeout_ms`, `[delete].trash_backend`.

Built-in candidate descriptors are defined in `src/candidate.rs`; the move to `config.toml` did not introduce an external rules file.
