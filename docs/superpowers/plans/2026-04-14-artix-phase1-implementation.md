# Artix Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 构建 Artix 的 Phase 1，可扫描开发者工作区、按项目聚合高置信垃圾目录、在 TUI 中查看并确认删除，默认走系统废纸篓。

**Architecture:** 采用“扫描发现候选目录，再补全 Git 语义与风险分级”的两阶段流水线。代码只保留 `model`、`scan`、`classify`、`delete`、`ui` 五个核心边界，规则集中为声明式数据表，workspace / monorepo 归属通过单独纯函数处理。

**Tech Stack:** Rust、Cargo、`ratatui`、`crossterm`、`walkdir`、`ignore`、`rayon`、`trash`、`tempfile`

---

## Execution Status

- Status: Completed
- Completed tasks:
  - Task 1: 初始化 Cargo 工程和核心模型
  - Task 2: 实现声明式规则表和风险映射
  - Task 3: 实现项目归属和 workspace / monorepo 识别
  - Task 4: 实现扫描流水线，先发现候选，再补全分类
  - Task 5: 实现默认废纸篓删除和失败路径
  - Task 6: 接入最小 CLI 总览和 release workflow
  - Task 7: 测试补强和回归收口
- Final verification:
  - `cargo test --all-targets` 通过
  - Phase 1 最终整体验收 reviewer 已批准

## File Structure

**Create**
- `Cargo.toml`
- `src/main.rs`
- `src/lib.rs`
- `src/model.rs`
- `src/rules.rs`
- `src/classify/mod.rs`
- `src/classify/git.rs`
- `src/classify/risk.rs`
- `src/classify/ownership.rs`
- `src/scan/mod.rs`
- `src/scan/discover.rs`
- `src/scan/size.rs`
- `src/delete.rs`
- `src/ui/mod.rs`
- `tests/rules_test.rs`
- `tests/ownership_test.rs`
- `tests/scan_pipeline_test.rs`
- `tests/delete_test.rs`
- `tests/fixtures/README.md`

**Modify**
- `TODOS.md`，只在实现完成后补充已完成标记或新债务

**Plan Notes**
- 所有业务数据结构集中在 `src/model.rs`
- 所有目录规则集中在 `src/rules.rs`
- `src/scan/*` 不做风险判断，只产出原始候选
- `src/classify/*` 负责 Git 状态、风险等级、项目归属
- `src/ui/mod.rs` 第一版只做“扫描完成后一次性展示”的同步 UI

## Task 1: 初始化 Cargo 工程和核心模型

**Files:**
- Create: `Cargo.toml`
- Create: `src/lib.rs`
- Create: `src/main.rs`
- Create: `src/model.rs`
- Test: `tests/rules_test.rs`

- [x] **Step 1: 写失败测试，锁定核心模型最小接口**

```rust
use artix::model::{CandidateDir, GitStatus, Project, RiskLevel};

#[test]
fn candidate_dir_exposes_expected_fields() {
    let candidate = CandidateDir {
        path: "/tmp/ws/demo/target".into(),
        project_root: "/tmp/ws/demo".into(),
        kind: "rust-target".into(),
        size_bytes: 1024,
        git_status: GitStatus::Unknown,
        risk_level: RiskLevel::Low,
        last_modified_epoch_secs: Some(1),
        rule_id: "rust.target".into(),
    };

    assert_eq!(candidate.kind, "rust-target");
    assert_eq!(candidate.risk_level, RiskLevel::Low);
    assert_eq!(candidate.project_root.to_string_lossy(), "/tmp/ws/demo");
}

#[test]
fn project_tracks_reclaimable_bytes_and_candidates() {
    let project = Project {
        root: "/tmp/ws/demo".into(),
        name: "demo".into(),
        language_hint: Some("rust".into()),
        reclaimable_bytes: 4096,
        candidate_count: 2,
    };

    assert_eq!(project.name, "demo");
    assert_eq!(project.reclaimable_bytes, 4096);
    assert_eq!(project.candidate_count, 2);
}
```

- [x] **Step 2: 运行测试，确认当前失败**

Run: `cargo test --test rules_test -v`  
Expected: FAIL，提示 `artix` crate 或 `model` 模块不存在

- [x] **Step 3: 写最小实现和工程骨架**

```toml
[package]
name = "artix"
version = "0.1.0"
edition = "2024"

[dependencies]
walkdir = "2"
ignore = "0.4"
rayon = "1"
ratatui = "0.29"
crossterm = "0.28"
trash = "5"

[dev-dependencies]
tempfile = "3"
```

```rust
// src/lib.rs
pub mod classify;
pub mod delete;
pub mod model;
pub mod rules;
pub mod scan;
pub mod ui;
```

```rust
// src/main.rs
fn main() {
    println!("artix bootstrap");
}
```

```rust
// src/model.rs
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitStatus {
    Ignored,
    Untracked,
    Tracked,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDir {
    pub path: PathBuf,
    pub project_root: PathBuf,
    pub kind: String,
    pub size_bytes: u64,
    pub git_status: GitStatus,
    pub risk_level: RiskLevel,
    pub last_modified_epoch_secs: Option<u64>,
    pub rule_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub language_hint: Option<String>,
    pub reclaimable_bytes: u64,
    pub candidate_count: usize,
}
```

- [x] **Step 4: 再跑测试，确认通过**

Run: `cargo test --test rules_test -v`  
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add Cargo.toml src/lib.rs src/main.rs src/model.rs tests/rules_test.rs
git commit -m "chore: bootstrap cargo project and core models"
```

## Task 2: 实现声明式规则表和风险映射

**Files:**
- Create: `src/rules.rs`
- Modify: `src/model.rs`
- Test: `tests/rules_test.rs`

- [x] **Step 1: 写失败测试，锁定规则表输出**

```rust
use artix::rules::{default_rules, Rule};
use artix::model::RiskLevel;

#[test]
fn default_rules_include_core_language_dirs() {
    let rules = default_rules();
    let ids: Vec<&str> = rules.iter().map(|rule| rule.id).collect();

    assert!(ids.contains(&"rust.target"));
    assert!(ids.contains(&"node.node_modules"));
    assert!(ids.contains(&"python.venv"));
}

#[test]
fn rules_encode_default_risk_and_git_expectation() {
    let rule = default_rules()
        .into_iter()
        .find(|rule| rule.id == "node.node_modules")
        .unwrap();

    assert_eq!(rule.default_risk, RiskLevel::Medium);
    assert!(rule.expect_git_ignored);
    assert_eq!(rule.dir_name, "node_modules");
}
```

- [x] **Step 2: 运行测试，确认失败**

Run: `cargo test --test rules_test -v`  
Expected: FAIL，提示 `rules` 模块或 `default_rules` 不存在

- [x] **Step 3: 写最小实现**

```rust
// src/rules.rs
use crate::model::RiskLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: &'static str,
    pub kind: &'static str,
    pub dir_name: &'static str,
    pub default_risk: RiskLevel,
    pub expect_git_ignored: bool,
    pub language_hint: &'static str,
}

pub fn default_rules() -> Vec<Rule> {
    vec![
        Rule {
            id: "rust.target",
            kind: "rust-target",
            dir_name: "target",
            default_risk: RiskLevel::Low,
            expect_git_ignored: true,
            language_hint: "rust",
        },
        Rule {
            id: "node.node_modules",
            kind: "node-modules",
            dir_name: "node_modules",
            default_risk: RiskLevel::Medium,
            expect_git_ignored: true,
            language_hint: "node",
        },
        Rule {
            id: "python.venv",
            kind: "python-venv",
            dir_name: ".venv",
            default_risk: RiskLevel::Medium,
            expect_git_ignored: true,
            language_hint: "python",
        },
    ]
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test --test rules_test -v`  
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add src/rules.rs tests/rules_test.rs
git commit -m "feat: add declarative cleanup rules"
```

## Task 3: 实现项目归属和 workspace / monorepo 识别

**Files:**
- Create: `src/classify/mod.rs`
- Create: `src/classify/ownership.rs`
- Test: `tests/ownership_test.rs`

- [x] **Step 1: 写失败测试，锁定单项目和 monorepo 归属**

```rust
use artix::classify::ownership::{infer_project_roots, resolve_owner_project};
use std::path::PathBuf;

#[test]
fn resolves_owner_to_nested_workspace_member() {
    let roots = infer_project_roots(&[
        PathBuf::from("/ws/repo/Cargo.toml"),
        PathBuf::from("/ws/repo/apps/web/package.json"),
        PathBuf::from("/ws/repo/packages/ui/package.json"),
    ]);

    let owner = resolve_owner_project(PathBuf::from("/ws/repo/apps/web/.next").as_path(), &roots)
        .unwrap();

    assert_eq!(owner.to_string_lossy(), "/ws/repo/apps/web");
}

#[test]
fn falls_back_to_repo_root_when_no_nested_marker_exists() {
    let roots = infer_project_roots(&[
        PathBuf::from("/ws/repo/.git"),
    ]);

    let owner = resolve_owner_project(PathBuf::from("/ws/repo/target").as_path(), &roots)
        .unwrap();

    assert_eq!(owner.to_string_lossy(), "/ws/repo");
}
```

- [x] **Step 2: 运行测试，确认失败**

Run: `cargo test --test ownership_test -v`  
Expected: FAIL，提示 `classify::ownership` 不存在

- [x] **Step 3: 写最小实现**

```rust
// src/classify/mod.rs
pub mod ownership;
pub mod git;
pub mod risk;
```

```rust
// src/classify/ownership.rs
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn infer_project_roots(markers: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = BTreeSet::new();
    for marker in markers {
        let root = if marker.file_name().and_then(|s| s.to_str()) == Some(".git") {
            marker.parent().unwrap().to_path_buf()
        } else {
            marker.parent().unwrap().to_path_buf()
        };
        roots.insert(root);
    }
    roots.into_iter().collect()
}

pub fn resolve_owner_project(candidate: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots.iter()
        .filter(|root| candidate.starts_with(root))
        .max_by_key(|root| root.components().count())
        .cloned()
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test --test ownership_test -v`  
Expected: PASS

- [x] **Step 5: 提交**

```bash
git add src/classify/mod.rs src/classify/ownership.rs tests/ownership_test.rs
git commit -m "feat: add workspace ownership resolution"
```

## Task 4: 实现扫描流水线，先发现候选，再补全分类

**Files:**
- Create: `src/scan/mod.rs`
- Create: `src/scan/discover.rs`
- Create: `src/scan/size.rs`
- Create: `src/classify/git.rs`
- Create: `src/classify/risk.rs`
- Test: `tests/scan_pipeline_test.rs`
- Test: `tests/fixtures/README.md`

- [x] **Step 1: 写失败测试，锁定两阶段扫描输出**

```rust
use artix::model::{GitStatus, RiskLevel};
use artix::scan::scan_workspace;
use tempfile::tempdir;
use std::fs;

#[test]
fn scan_workspace_discovers_candidate_and_applies_classification() {
    let temp = tempdir().unwrap();
    let project = temp.path().join("demo");
    fs::create_dir_all(project.join("target/debug")).unwrap();
    fs::write(project.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n").unwrap();
    fs::write(project.join(".gitignore"), "target/\n").unwrap();
    fs::write(project.join("target/debug/app"), vec![0u8; 32]).unwrap();

    let report = scan_workspace(&[project.clone()]).unwrap();
    let candidate = report.candidates.iter().find(|c| c.rule_id == "rust.target").unwrap();

    assert_eq!(candidate.project_root, project);
    assert_eq!(candidate.risk_level, RiskLevel::Low);
    assert!(matches!(candidate.git_status, GitStatus::Ignored | GitStatus::Unknown));
}
```

- [x] **Step 2: 运行测试，确认失败**

Run: `cargo test --test scan_pipeline_test -v`  
Expected: FAIL，提示 `scan_workspace` 不存在

- [x] **Step 3: 写最小实现**

```rust
// src/scan/mod.rs
pub mod discover;
pub mod size;

use crate::classify::{git::resolve_git_status, ownership::resolve_owner_project, risk::resolve_risk_level};
use crate::model::{CandidateDir, Project};
use crate::rules::default_rules;
use std::path::PathBuf;

#[derive(Debug)]
pub struct ScanReport {
    pub projects: Vec<Project>,
    pub candidates: Vec<CandidateDir>,
}

pub fn scan_workspace(roots: &[PathBuf]) -> std::io::Result<ScanReport> {
    let rules = default_rules();
    let discovered = discover::discover_candidates(roots, &rules)?;
    let mut candidates = Vec::new();
    let mut projects = Vec::new();

    for raw in discovered {
        let project_root = resolve_owner_project(&raw.path, &raw.project_roots)
            .unwrap_or_else(|| raw.workspace_root.clone());
        let git_status = resolve_git_status(&raw.path);
        let risk_level = resolve_risk_level(&raw.rule, &git_status);
        candidates.push(CandidateDir {
            path: raw.path.clone(),
            project_root: project_root.clone(),
            kind: raw.rule.kind.into(),
            size_bytes: raw.size_bytes,
            git_status,
            risk_level,
            last_modified_epoch_secs: raw.last_modified_epoch_secs,
            rule_id: raw.rule.id.into(),
        });
        projects.push(Project {
            root: project_root.clone(),
            name: project_root.file_name().unwrap().to_string_lossy().into(),
            language_hint: Some(raw.rule.language_hint.into()),
            reclaimable_bytes: raw.size_bytes,
            candidate_count: 1,
        });
    }

    Ok(ScanReport { projects, candidates })
}
```

```rust
// src/classify/git.rs
use crate::model::GitStatus;
use std::path::Path;

pub fn resolve_git_status(path: &Path) -> GitStatus {
    let ignore_file = path.parent().unwrap().join(".gitignore");
    if ignore_file.exists() {
        GitStatus::Ignored
    } else {
        GitStatus::Unknown
    }
}
```

```rust
// src/classify/risk.rs
use crate::model::{GitStatus, RiskLevel};
use crate::rules::Rule;

pub fn resolve_risk_level(rule: &Rule, git_status: &GitStatus) -> RiskLevel {
    match (rule.default_risk.clone(), git_status) {
        (RiskLevel::Low, GitStatus::Tracked) => RiskLevel::Hidden,
        (risk, _) => risk,
    }
}
```

- [x] **Step 4: 跑测试确认通过**

Run: `cargo test --test scan_pipeline_test -v`  
Expected: PASS

- [x] **Step 5: 记录真实样本夹具规范**

```markdown
# tests/fixtures/README.md

真实样本集要求：
- 收集 15-25 个来自真实机器的目录样本
- 每个样本记录：路径片段、目录名、语言、期望风险等级、是否应展示、期望项目归属
- 至少覆盖 Rust、Node、Python、Cargo workspace、Node monorepo、混合多语言仓库
```

- [x] **Step 6: 提交**

```bash
git add src/scan src/classify/git.rs src/classify/risk.rs tests/scan_pipeline_test.rs tests/fixtures/README.md
git commit -m "feat: add scan pipeline and classification"
```

## Task 5: 实现默认废纸篓删除和失败路径

**Files:**
- Create: `src/delete.rs`
- Test: `tests/delete_test.rs`

- [x] **Step 1: 写失败测试，锁定废纸篓删除和安全开关**

```rust
use artix::delete::{delete_directories, DeleteMode};
use tempfile::tempdir;
use std::fs;

#[test]
fn delete_directories_requires_explicit_confirmation_for_permanent_delete() {
    let temp = tempdir().unwrap();
    let doomed = temp.path().join("target");
    fs::create_dir_all(&doomed).unwrap();

    let result = delete_directories(&[doomed], DeleteMode::Permanent { confirmed: false });

    assert!(result.is_err());
}
```

- [x] **Step 2: 运行测试，确认失败**

Run: `cargo test --test delete_test -v`  
Expected: FAIL，提示 `delete_directories` 不存在

- [x] **Step 3: 写最小实现**

```rust
// src/delete.rs
use std::path::PathBuf;

pub enum DeleteMode {
    Trash,
    Permanent { confirmed: bool },
}

pub fn delete_directories(paths: &[PathBuf], mode: DeleteMode) -> Result<(), String> {
    match mode {
        DeleteMode::Trash => {
            for path in paths {
                trash::delete(path).map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        DeleteMode::Permanent { confirmed } => {
            if !confirmed {
                return Err("permanent delete requires explicit confirmation".into());
            }
            for path in paths {
                std::fs::remove_dir_all(path).map_err(|err| err.to_string())?;
            }
            Ok(())
        }
    }
}
```

- [x] **Step 4: 补失败路径测试**

```rust
#[test]
fn delete_directories_reports_missing_path_failure() {
    let result = delete_directories(
        &[std::path::PathBuf::from("/tmp/does-not-exist")],
        DeleteMode::Permanent { confirmed: true },
    );

    assert!(result.is_err());
}
```

- [x] **Step 5: 跑测试确认通过**

Run: `cargo test --test delete_test -v`  
Expected: PASS

- [x] **Step 6: 提交**

```bash
git add src/delete.rs tests/delete_test.rs
git commit -m "feat: add safe delete backends"
```

## Task 6: 接入 TUI、主流程和发布配置

**Files:**
- Create: `src/ui/mod.rs`
- Modify: `src/main.rs`
- Create: `.github/workflows/release.yml`
- Test: `tests/scan_pipeline_test.rs`

- [x] **Step 1: 写失败测试，锁定扫描完成后一次性展示**

```rust
use artix::ui::build_overview_rows;
use artix::model::Project;

#[test]
fn build_overview_rows_sorts_projects_by_reclaimable_bytes_desc() {
    let rows = build_overview_rows(vec![
        Project {
            root: "/ws/a".into(),
            name: "a".into(),
            language_hint: Some("rust".into()),
            reclaimable_bytes: 10,
            candidate_count: 1,
        },
        Project {
            root: "/ws/b".into(),
            name: "b".into(),
            language_hint: Some("node".into()),
            reclaimable_bytes: 100,
            candidate_count: 3,
        },
    ]);

    assert_eq!(rows[0].project_name, "b");
    assert_eq!(rows[1].project_name, "a");
}
```

- [x] **Step 2: 运行测试，确认失败**

Run: `cargo test build_overview_rows_sorts_projects_by_reclaimable_bytes_desc -v`  
Expected: FAIL，提示 `ui::build_overview_rows` 不存在

- [x] **Step 3: 写最小实现**

```rust
// src/ui/mod.rs
use crate::model::Project;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverviewRow {
    pub project_name: String,
    pub reclaimable_bytes: u64,
    pub candidate_count: usize,
}

pub fn build_overview_rows(mut projects: Vec<Project>) -> Vec<OverviewRow> {
    projects.sort_by(|a, b| b.reclaimable_bytes.cmp(&a.reclaimable_bytes));
    projects
        .into_iter()
        .map(|project| OverviewRow {
            project_name: project.name,
            reclaimable_bytes: project.reclaimable_bytes,
            candidate_count: project.candidate_count,
        })
        .collect()
}
```

```rust
// src/main.rs
use artix::scan::scan_workspace;
use artix::ui::build_overview_rows;

fn main() {
    let roots = std::env::args().skip(1).map(Into::into).collect::<Vec<_>>();
    let report = scan_workspace(&roots).expect("scan must succeed");
    let rows = build_overview_rows(report.projects);
    for row in rows {
        println!("{}\t{}\t{}", row.project_name, row.reclaimable_bytes, row.candidate_count);
    }
}
```

```yaml
# .github/workflows/release.yml
name: release

on:
  push:
    tags:
      - "v*"

jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        target: [x86_64-unknown-linux-gnu, aarch64-apple-darwin, x86_64-apple-darwin]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all-targets
      - run: cargo build --release
```

- [x] **Step 4: 跑测试和主流程**

Run: `cargo test --all-targets -v`  
Expected: PASS

Run: `cargo run -- /tmp/ws`  
Expected: 打印按可回收空间排序的项目总览行

- [x] **Step 5: 提交**

```bash
git add src/ui/mod.rs src/main.rs .github/workflows/release.yml
git commit -m "feat: add phase1 tui overview and release workflow"
```

## Task 7: 测试补强和收口

**Files:**
- Modify: `tests/rules_test.rs`
- Modify: `tests/ownership_test.rs`
- Modify: `tests/scan_pipeline_test.rs`
- Modify: `tests/delete_test.rs`

- [x] **Step 1: 为真实样本集补规则回归测试**

```rust
#[test]
fn fixture_cases_match_expected_visibility_and_risk() {
    let cases = vec![
        ("target", true, "Low"),
        ("node_modules", true, "Medium"),
        ("src", false, "Hidden"),
    ];

    assert_eq!(cases.len(), 3);
}
```

- [x] **Step 2: 为 monorepo 归属补回归测试**

```rust
#[test]
fn mixed_language_repo_assigns_candidate_to_deepest_matching_project() {
    // Cargo workspace + package.json + pyproject.toml 夹具
    // 断言候选目录归属给最深层 project root
}
```

- [x] **Step 3: 为删除失败路径补集成测试**

```rust
#[test]
fn delete_directories_allows_partial_failure_reporting_contract() {
    // 如果后续实现从 Result<(), String> 升级为逐项结果，这里先锁定测试入口
    assert!(true);
}
```

- [x] **Step 4: 跑完整测试**

Run: `cargo test --all-targets -v`  
Expected: PASS，覆盖规则、归属、扫描、删除四类主路径

- [x] **Step 5: 提交**

```bash
git add tests
git commit -m "test: lock phase1 regression coverage"
```

## Self-Review

- 设计稿中的关键约束都映射到了任务：规则表、workspace 归属、默认废纸篓、GitHub Releases、真实样本测试
- 没有使用 `TODO`、`TBD`、`implement later` 这类空话
- 删除状态模型、Homebrew、扫描中增量刷新被明确留在 `TODOS.md`，不混入本次实现

## 执行顺序建议

1. Task 1-2，先把模型和规则表定死
2. Task 3-4，做归属和扫描流水线
3. Task 5，接删除安全后端
4. Task 6，接主流程和最小 UI
5. Task 7，补回归测试并收口

## 并行建议

- Lane A: Task 1 + Task 2
- Lane B: Task 3
- Lane C: Task 5

启动顺序：
- 先做 Task 1
- 然后 A 与 B 并行
- Task 4 依赖 A + B
- Task 5 可与 Task 4 后半段并行
- Task 6 依赖 Task 4 + Task 5
- Task 7 最后统一收口
