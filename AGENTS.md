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
- `rules`: Built-in candidate directory rules.
- `scan`: Workspace scanning and directory browsing.
- `classify`: Git/worktree context, ownership heuristics, and risk classification.
- `ui`: TUI state, rendering, and event loop.
- `delete` / `delete_flow`: Delete execution + confirmation state machine.

### Data model

Important types (see `src/model.rs`):

- `BrowserEntry`: The UI list item (dir or cleanup candidate) with size, Git status, and context.
- `CandidateDir`: A discovered “cleanup candidate” directory with owner project root, size, rule id, Git status, and risk level.
- `Project`: Aggregated per-project totals (name, reclaimable bytes, candidate count).
- `GitContext`: Repo/worktree roots plus branch/head metadata.

### Scanning pipeline (plain-text mode)

`scan::scan_workspace` (see `src/scan/mod.rs`) does roughly:

1. **Collect ownership markers**: recursively find `Cargo.toml`, `package.json`, `pyproject.toml` under the provided roots, then infer candidate “project roots” from those marker parents.
2. **Discover candidates**: recursively walk directories and match built-in rules by directory name (see `src/scan/discover.rs` and `src/rules.rs`).
3. **Enrich candidates** (async, concurrency-limited):
   - Determine `project_root` (nearest owner marker root, otherwise nearest CLI root).
   - Compute `size_bytes`.
   - Classify `git_status` and `risk_level`.
4. **Summarize projects**: aggregate candidates into `Project` rows for printing.

### TUI architecture

The TUI loop lives in `ui::run_tui` / `run_app` (see `src/ui/mod.rs`). A `BrowserApp` owns:

- `AppState`: current directory, entries, filter mode, selection index, and delete dialog state.
- An in-memory cache mapping `PathBuf -> Vec<BrowserEntry>`.
- A background request/response channel that streams directory load results and per-entry updates.

Directory loading is intentionally two-phase (see `BrowserApp::load_directory` and the background `BgRequest::LoadDirectory` worker in `src/ui/mod.rs`):

- **Quick placeholder listing**: returns entries with `size_bytes = 0` and `git_status = Unknown` so the UI becomes usable immediately.
- **Progressive enrichment**: per-entry tasks compute size + Git status and send `EntryUpdated` messages; the UI applies updates, resorts, and preserves the selected entry.

Deletion is also handled asynchronously:

- UI triggers delete confirmation state transitions (`delete_flow`).
- Actual deletion executes in a blocking task (`execute_delete`) and then the UI invalidates affected cache entries and refreshes the current directory.

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
- The UI uses an explicit “fast placeholder then stream updates” pattern; when changing list behavior, ensure selection is clamped/preserved (see `AppState::clamp_selection` and `BrowserApp::resort_visible_entries_preserving_selection` in `src/ui/mod.rs`).

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
- **Trash delete** uses the `trash` crate and can fall back to a built-in macOS `~/.Trash` move; built-in trash uses `HOME` (see `src/delete.rs`).

Git status classification in the UI depends on executing `git` from PATH (see `src/classify/git.rs`). Subprocess output is suppressed and calls are timeout-limited, but agents should be aware that PATH influences which `git` executable is used.

## 6) Configuration

Configuration is primarily through `config.toml` loaded by `src/config.rs`.

- **Default config path:** `~/.config/artix/config.toml`.
- **Compatibility lookup order:** `~/.config/artix/config.toml`, then `~/.artix/config.toml`.
- **Supported user-facing fields:** `version`, `[ui].mode`, `[ui].icons`, `[performance].fs_concurrency`, `[performance].git_concurrency`, `[performance].tui_entry_concurrency`, `[scan.tui_size_budget].max_entries`, `[scan.tui_size_budget].timeout_ms`, `[delete].trash_backend`.

Built-in candidate rules are still defined in `src/rules.rs`; the move to `config.toml` did not introduce an external rules file.
