use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use artix::model::Project;
use artix::model::{GitStatus, RiskLevel};
use artix::scan::scan_workspace;
use artix::ui::build_overview_rows;
use tokio::time::{Duration, timeout};

#[tokio::test]
async fn scan_workspace_finds_target_and_classifies_minimally() {
    let project = make_temp_project();

    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(project.join(".gitignore"), "target/\n").expect("write .gitignore");

    let target_file = project.join("target/debug/app");
    fs::create_dir_all(target_file.parent().expect("target dir")).expect("create target dir");
    fs::write(&target_file, "binary").expect("write target/debug/app");

    let report = scan_workspace(std::slice::from_ref(&project)).await;

    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.rule_id == "rust.target")
        .expect("rust.target candidate");
    let project_summary = report
        .projects
        .iter()
        .find(|summary| summary.root == project)
        .expect("project summary");

    assert_eq!(candidate.project_root, project);
    assert_eq!(candidate.risk_level, RiskLevel::Low);
    assert!(matches!(
        candidate.git_status,
        GitStatus::Ignored | GitStatus::Unknown
    ));
    assert_eq!(project_summary.language_hint.as_deref(), Some("rust"));

    fs::remove_dir_all(&project).expect("cleanup temp project");
}

#[tokio::test]
async fn scan_workspace_keeps_discovery_and_project_summary_in_sync() {
    let project = make_temp_project();

    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(
        project.join(".gitignore"),
        "target/\npackages/app/target/\n",
    )
    .expect("write .gitignore");

    let root_target = project.join("target/debug/app");
    fs::create_dir_all(root_target.parent().expect("root target dir"))
        .expect("create root target dir");
    fs::write(&root_target, "binary").expect("write target/debug/app");

    let nested_target = project.join("packages/app/target/debug/tool");
    fs::create_dir_all(nested_target.parent().expect("nested target dir"))
        .expect("create nested target dir");
    fs::write(&nested_target, "binary").expect("write packages/app/target/debug/tool");

    let report = scan_workspace(std::slice::from_ref(&project)).await;

    assert_eq!(report.candidates.len(), 2);
    assert_eq!(report.projects.len(), 1);

    let candidate_ids = report
        .candidates
        .iter()
        .map(|candidate| candidate.rule_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(candidate_ids, BTreeSet::from(["rust.target"]));
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.project_root == project)
    );

    let project_summary = &report.projects[0];
    assert_eq!(project_summary.root, project);
    assert_eq!(project_summary.language_hint.as_deref(), Some("rust"));
    assert_eq!(project_summary.candidate_count, 2);
    assert_eq!(
        project_summary.reclaimable_bytes,
        report
            .candidates
            .iter()
            .map(|candidate| candidate.size_bytes)
            .sum()
    );

    let rows = build_overview_rows(report.projects.clone());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].project_name, project_summary.name);
    assert_eq!(rows[0].candidate_count, project_summary.candidate_count);
    assert_eq!(rows[0].reclaimable_bytes, project_summary.reclaimable_bytes);

    fs::remove_dir_all(&project).expect("cleanup temp project");
}

#[tokio::test]
async fn scan_workspace_assigns_nested_node_modules_to_nearest_node_project() {
    let workspace = make_temp_project();
    let app = workspace.join("apps/web");

    fs::create_dir_all(&app).expect("create nested app");
    fs::write(app.join("package.json"), "{ \"name\": \"web\" }\n").expect("write package.json");
    fs::write(workspace.join(".gitignore"), "apps/web/node_modules/\n").expect("write .gitignore");

    let node_modules_pkg = app.join("node_modules/react");
    fs::create_dir_all(&node_modules_pkg).expect("create node_modules/react");
    fs::write(node_modules_pkg.join("index.js"), "module.exports = {};\n")
        .expect("write react entrypoint");

    let report = scan_workspace(std::slice::from_ref(&workspace)).await;

    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.rule_id == "node.node_modules")
        .expect("node.node_modules candidate");
    let project_summary = report
        .projects
        .iter()
        .find(|summary| summary.root == app)
        .expect("nested node project summary");

    assert_eq!(candidate.project_root, app);
    assert_eq!(project_summary.language_hint.as_deref(), Some("node"));
    assert_eq!(project_summary.candidate_count, 1);

    fs::remove_dir_all(&workspace).expect("cleanup temp project");
}

#[tokio::test]
async fn scan_workspace_assigns_nested_python_venv_to_nearest_python_project() {
    let workspace = make_temp_project();
    let app = workspace.join("services/api");

    fs::create_dir_all(&app).expect("create nested python app");
    fs::write(
        app.join("pyproject.toml"),
        "[project]\nname = \"api\"\nversion = \"0.1.0\"\n",
    )
    .expect("write pyproject.toml");
    fs::write(workspace.join(".gitignore"), "services/api/.venv/\n").expect("write .gitignore");

    let venv_bin = app.join(".venv/bin");
    fs::create_dir_all(&venv_bin).expect("create .venv/bin");
    fs::write(venv_bin.join("python"), "#!/usr/bin/env python3\n").expect("write python shim");

    let report = scan_workspace(std::slice::from_ref(&workspace)).await;

    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.rule_id == "python.venv")
        .expect("python.venv candidate");
    let project_summary = report
        .projects
        .iter()
        .find(|summary| summary.root == app)
        .expect("nested python project summary");

    assert_eq!(candidate.project_root, app);
    assert_eq!(project_summary.language_hint.as_deref(), Some("python"));
    assert_eq!(project_summary.candidate_count, 1);

    fs::remove_dir_all(&workspace).expect("cleanup temp project");
}

#[cfg(unix)]
#[tokio::test]
async fn scan_workspace_size_does_not_follow_symlink_cycles() {
    use std::os::unix::fs::symlink;

    let project = make_temp_project();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(project.join(".gitignore"), "target/\n").expect("write .gitignore");

    let target_dir = project.join("target");
    let target_file = target_dir.join("debug/app");
    fs::create_dir_all(target_file.parent().expect("target debug dir"))
        .expect("create target/debug");
    fs::write(&target_file, "binary").expect("write target/debug/app");

    // Create a cycle: target/loop -> .. (project root).
    symlink("..", target_dir.join("loop")).expect("create symlink cycle");

    let report = timeout(
        Duration::from_secs(2),
        scan_workspace(std::slice::from_ref(&project)),
    )
    .await
    .expect("scan_workspace should not hang on symlink cycles");

    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.rule_id == "rust.target")
        .expect("rust.target candidate");

    // "binary" is 6 bytes, and we only count the symlink itself (".." -> len 2).
    assert_eq!(candidate.size_bytes, 8);

    fs::remove_dir_all(&project).expect("cleanup temp project");
}

#[cfg(unix)]
#[tokio::test]
async fn scan_workspace_size_does_not_follow_symlink_escape() {
    use std::os::unix::fs::symlink;

    let project = make_temp_project();
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(project.join(".gitignore"), "target/\n").expect("write .gitignore");

    let target_dir = project.join("target");
    let target_file = target_dir.join("debug/app");
    fs::create_dir_all(target_file.parent().expect("target debug dir"))
        .expect("create target/debug");
    fs::write(&target_file, "binary").expect("write target/debug/app");

    // Create an "escape" directory with a large file.
    let escape_dir = project.join("escape");
    fs::create_dir_all(&escape_dir).expect("create escape dir");
    fs::write(escape_dir.join("big.bin"), vec![0u8; 1024 * 1024]).expect("write big file");

    // target/escape_link -> ../escape; this must not pull escape/ into target size.
    let link_target = "../escape";
    symlink(link_target, target_dir.join("escape_link")).expect("create symlink escape");

    let report = timeout(
        Duration::from_secs(2),
        scan_workspace(std::slice::from_ref(&project)),
    )
    .await
    .expect("scan_workspace should finish");

    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.rule_id == "rust.target")
        .expect("rust.target candidate");

    // 6 bytes for "binary" + symlink metadata length for "../escape".
    assert_eq!(candidate.size_bytes, 6 + link_target.len() as u64);

    fs::remove_dir_all(&project).expect("cleanup temp project");
}

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

#[test]
fn cli_without_args_scans_current_directory() {
    let project = make_temp_project();
    let expected_name = project
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temp project name")
        .to_string();

    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write Cargo.toml");
    fs::write(project.join(".gitignore"), "target/\n").expect("write .gitignore");

    let target_file = project.join("target/debug/app");
    fs::create_dir_all(target_file.parent().expect("target dir")).expect("create target dir");
    fs::write(&target_file, "binary").expect("write target/debug/app");

    let output = Command::new(env!("CARGO_BIN_EXE_artix"))
        .current_dir(&project)
        .output()
        .expect("run artix without args");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(!stdout.trim().is_empty());
    assert!(stdout.contains('\t'));
    assert!(stdout.lines().any(|line| line.starts_with(&expected_name)));

    fs::remove_dir_all(&project).expect("cleanup temp project");
}

fn make_temp_project() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time after epoch")
        .as_nanos();
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("artix-scan-pipeline-{pid}-{counter}-{unique}"));
    fs::create_dir_all(&path).expect("create temp project root");
    path
}
