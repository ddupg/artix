use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::Deserialize;
use serde_json::Value;

const OUTPUT_LIMIT_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProjectKind {
    Rust,
    Maven,
    Gradle,
    Node,
    Python,
}

impl ProjectKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Maven => "Maven",
            Self::Gradle => "Gradle",
            Self::Node => "Node",
            Self::Python => "Python",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageSummary {
    pub name: String,
    pub bytes: u64,
    pub percent: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProfile {
    pub root: PathBuf,
    pub kinds: Vec<ProjectKind>,
    pub languages: Vec<LanguageSummary>,
    pub clean_plan: CleanPlan,
}

impl ProjectProfile {
    pub fn kind_label(&self) -> String {
        self.kinds
            .iter()
            .map(ProjectKind::label)
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn language_label(&self) -> String {
        self.languages
            .iter()
            .take(3)
            .map(|language| format!("{} {}%", language.name, language.percent))
            .collect::<Vec<_>>()
            .join(", ")
    }

    pub fn can_clean(&self) -> bool {
        !self.clean_plan.commands.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanPlan {
    pub project_root: PathBuf,
    pub commands: Vec<CleanCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanCommand {
    pub kind: ProjectKind,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

impl CleanCommand {
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanRunSummary {
    pub project_root: PathBuf,
    pub results: Vec<CleanCommandResult>,
}

impl CleanRunSummary {
    pub fn success(&self) -> bool {
        self.results.iter().all(|result| result.success)
    }

    pub fn message(&self) -> String {
        let total = self.results.len();
        let failed = self.results.iter().filter(|result| !result.success).count();
        if failed == 0 {
            return format!("clean completed: {total} command(s)");
        }

        let first_failure = self
            .results
            .iter()
            .find(|result| !result.success)
            .expect("failed count came from results");
        let mut message = format!(
            "clean failed: {} ({} failed of {})",
            first_failure.command.display(),
            failed,
            total
        );
        if !first_failure.stderr.trim().is_empty() {
            message.push_str(": ");
            message.push_str(first_failure.stderr.trim());
        }
        message
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanCommandResult {
    pub command: CleanCommand,
    pub success: bool,
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Deserialize)]
struct PackageJson {
    #[serde(default, rename = "packageManager")]
    package_manager: Option<String>,
    #[serde(default)]
    scripts: BTreeMap<String, Value>,
}

pub fn detect_project_profile(path: &Path) -> Result<Option<ProjectProfile>, String> {
    if !path.is_dir() {
        return Ok(None);
    }

    let kinds = detect_project_kinds(path);
    if kinds.is_empty() {
        return Ok(None);
    }

    let languages = detect_languages(path)?;
    if languages.is_empty() {
        return Ok(None);
    }

    let clean_plan = plan_clean(path)?;

    Ok(Some(ProjectProfile {
        root: path.to_path_buf(),
        kinds,
        languages,
        clean_plan,
    }))
}

pub fn plan_clean(path: &Path) -> Result<CleanPlan, String> {
    let mut commands = Vec::new();

    if path.join("Cargo.toml").is_file() {
        commands.push(command(ProjectKind::Rust, "cargo", ["clean"], path));
    }

    if path.join("pom.xml").is_file() {
        if path.join("mvnw").is_file() {
            commands.push(command(ProjectKind::Maven, "./mvnw", ["clean"], path));
        } else {
            commands.push(command(ProjectKind::Maven, "mvn", ["clean"], path));
        }
    }

    if path.join("build.gradle").is_file() || path.join("build.gradle.kts").is_file() {
        if path.join("gradlew").is_file() {
            commands.push(command(ProjectKind::Gradle, "./gradlew", ["clean"], path));
        } else {
            commands.push(command(ProjectKind::Gradle, "gradle", ["clean"], path));
        }
    }

    if let Some(command) = node_clean_command(path) {
        commands.push(command);
    }

    Ok(CleanPlan {
        project_root: path.to_path_buf(),
        commands,
    })
}

pub async fn execute_clean_plan(plan: CleanPlan) -> CleanRunSummary {
    let mut results = Vec::with_capacity(plan.commands.len());
    for command in plan.commands {
        let result = execute_clean_command(command).await;
        results.push(result);
    }

    CleanRunSummary {
        project_root: plan.project_root,
        results,
    }
}

async fn execute_clean_command(command: CleanCommand) -> CleanCommandResult {
    let mut cmd = tokio::process::Command::new(&command.program);
    cmd.current_dir(&command.cwd)
        .args(&command.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match cmd.output().await {
        Ok(output) => CleanCommandResult {
            command,
            success: output.status.success(),
            status_code: output.status.code(),
            stdout: truncate_output(&output.stdout),
            stderr: truncate_output(&output.stderr),
        },
        Err(err) => CleanCommandResult {
            command,
            success: false,
            status_code: None,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

fn command<const N: usize>(
    kind: ProjectKind,
    program: &str,
    args: [&str; N],
    cwd: &Path,
) -> CleanCommand {
    CleanCommand {
        kind,
        program: program.to_string(),
        args: args.into_iter().map(str::to_string).collect(),
        cwd: cwd.to_path_buf(),
    }
}

fn detect_project_kinds(path: &Path) -> Vec<ProjectKind> {
    let mut kinds = Vec::new();
    if path.join("Cargo.toml").is_file() {
        kinds.push(ProjectKind::Rust);
    }
    if path.join("pom.xml").is_file() {
        kinds.push(ProjectKind::Maven);
    }
    if path.join("build.gradle").is_file() || path.join("build.gradle.kts").is_file() {
        kinds.push(ProjectKind::Gradle);
    }
    if path.join("package.json").is_file() {
        kinds.push(ProjectKind::Node);
    }
    if path.join("pyproject.toml").is_file() {
        kinds.push(ProjectKind::Python);
    }
    kinds
}

fn detect_languages(path: &Path) -> Result<Vec<LanguageSummary>, String> {
    let source = gengo::Directory::new(path, 8192).map_err(|err| err.to_string())?;
    let gengo = gengo::Builder::new(source)
        .build()
        .map_err(|err| err.to_string())?;
    let analysis = gengo.analyze().map_err(|err| err.to_string())?;
    let summary = analysis.summary();
    let total = summary.total() as u64;
    if total == 0 {
        return Ok(Vec::new());
    }

    let mut summaries = summary
        .iter()
        .map(|(language, bytes)| {
            let bytes = *bytes as u64;
            LanguageSummary {
                name: language.name().to_string(),
                bytes,
                percent: ((bytes.saturating_mul(100) + total / 2) / total) as u8,
            }
        })
        .collect::<Vec<_>>();
    summaries.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(summaries)
}

fn node_clean_command(path: &Path) -> Option<CleanCommand> {
    if !path.join("package.json").is_file() {
        return None;
    }

    let package = read_package_json(path).ok()?;
    if !package.scripts.contains_key("clean") {
        return None;
    }

    let package_manager = detect_package_manager(path, &package);
    Some(CleanCommand {
        kind: ProjectKind::Node,
        program: package_manager,
        args: vec!["run".to_string(), "clean".to_string()],
        cwd: path.to_path_buf(),
    })
}

fn detect_package_manager(path: &Path, package: &PackageJson) -> String {
    if let Some(manager) = package
        .package_manager
        .as_deref()
        .and_then(|value| value.split('@').next())
        .filter(|value| !value.is_empty())
    {
        return manager.to_string();
    }

    if path.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if path.join("yarn.lock").is_file() {
        "yarn"
    } else if path.join("bun.lockb").is_file() || path.join("bun.lock").is_file() {
        "bun"
    } else {
        "npm"
    }
    .to_string()
}

fn read_package_json(path: &Path) -> Result<PackageJson, String> {
    let package_path = path.join("package.json");
    let contents = fs::read_to_string(&package_path)
        .map_err(|err| format!("failed to read {}: {err}", package_path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|err| format!("failed to parse {}: {err}", package_path.display()))
}

fn truncate_output(bytes: &[u8]) -> String {
    let bytes = if bytes.len() > OUTPUT_LIMIT_BYTES {
        &bytes[..OUTPUT_LIMIT_BYTES]
    } else {
        bytes
    };
    String::from_utf8_lossy(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        CleanCommand, CleanPlan, ProjectKind, detect_project_profile, execute_clean_plan,
        plan_clean,
    };
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn current_directory_must_be_project_root() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        let nested = root.join("src");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").unwrap();
        fs::write(nested.join("main.rs"), "fn main() {}\n").unwrap();

        assert!(detect_project_profile(&nested).unwrap().is_none());
    }

    #[test]
    fn current_project_root_detects_language_summary() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let profile = detect_project_profile(dir.path())
            .unwrap()
            .expect("project profile");

        assert_eq!(profile.kinds, vec![ProjectKind::Rust]);
        assert!(
            profile
                .languages
                .iter()
                .any(|language| language.name == "Rust")
        );
        assert!(profile.can_clean());
    }

    #[test]
    fn rust_project_plans_cargo_clean() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();

        let plan = plan_clean(dir.path()).unwrap();

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].kind, ProjectKind::Rust);
        assert_eq!(plan.commands[0].display(), "cargo clean");
    }

    #[test]
    fn java_project_prefers_wrappers() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("pom.xml"), "<project />").unwrap();
        fs::write(dir.path().join("mvnw"), "#!/bin/sh\n").unwrap();
        fs::write(dir.path().join("build.gradle.kts"), "plugins {}\n").unwrap();
        fs::write(dir.path().join("gradlew"), "#!/bin/sh\n").unwrap();

        let plan = plan_clean(dir.path()).unwrap();
        let commands = plan
            .commands
            .iter()
            .map(|command| command.display())
            .collect::<Vec<_>>();

        assert_eq!(commands, vec!["./mvnw clean", "./gradlew clean"]);
    }

    #[test]
    fn node_requires_clean_script_and_detects_package_manager() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"clean":"rimraf dist"}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();

        let plan = plan_clean(dir.path()).unwrap();

        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].kind, ProjectKind::Node);
        assert_eq!(plan.commands[0].display(), "pnpm run clean");
    }

    #[test]
    fn node_without_clean_script_has_no_command() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"build":"vite"}}"#,
        )
        .unwrap();

        let plan = plan_clean(dir.path()).unwrap();

        assert!(plan.commands.is_empty());
    }

    #[test]
    fn invalid_package_json_does_not_block_other_clean_commands() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("package.json"), "{").unwrap();

        let plan = plan_clean(dir.path()).unwrap();
        let commands = plan
            .commands
            .iter()
            .map(|command| command.display())
            .collect::<Vec<_>>();

        assert_eq!(commands, vec!["cargo clean"]);
    }

    #[test]
    fn python_project_has_no_v1_clean_command() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname = \"api\"\n",
        )
        .unwrap();

        let plan = plan_clean(dir.path()).unwrap();

        assert!(plan.commands.is_empty());
    }

    #[test]
    fn multi_language_commands_have_stable_order() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        fs::write(dir.path().join("pom.xml"), "<project />").unwrap();
        fs::write(dir.path().join("build.gradle"), "plugins {}\n").unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"clean":"rimraf dist"},"packageManager":"yarn@4.0.0"}"#,
        )
        .unwrap();

        let plan = plan_clean(dir.path()).unwrap();
        let commands = plan
            .commands
            .iter()
            .map(|command| command.display())
            .collect::<Vec<_>>();

        assert_eq!(
            commands,
            vec!["cargo clean", "mvn clean", "gradle clean", "yarn run clean"]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executor_runs_command_in_project_root() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        let bin = dir.path().join("bin");
        let log = dir.path().join("cwd.log");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let script = bin.join("fake-clean");
        fs::write(
            &script,
            format!("#!/bin/sh\npwd > {}\necho ok\n", log.display()),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let summary = execute_clean_plan(CleanPlan {
            project_root: project.clone(),
            commands: vec![CleanCommand {
                kind: ProjectKind::Rust,
                program: script.display().to_string(),
                args: vec!["clean".to_string()],
                cwd: project.clone(),
            }],
        })
        .await;

        assert!(summary.success());
        let logged = PathBuf::from(fs::read_to_string(log).unwrap().trim());
        assert_eq!(
            logged.canonicalize().unwrap(),
            project.canonicalize().unwrap()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn executor_runs_remaining_commands_after_failure() {
        let dir = tempdir().unwrap();
        let project = dir.path().join("project");
        fs::create_dir_all(&project).unwrap();

        let summary = execute_clean_plan(CleanPlan {
            project_root: project.clone(),
            commands: vec![
                CleanCommand {
                    kind: ProjectKind::Maven,
                    program: "sh".to_string(),
                    args: vec!["-c".to_string(), "exit 7".to_string()],
                    cwd: project.clone(),
                },
                CleanCommand {
                    kind: ProjectKind::Gradle,
                    program: "sh".to_string(),
                    args: vec!["-c".to_string(), "printf ok".to_string()],
                    cwd: project.clone(),
                },
            ],
        })
        .await;

        assert!(!summary.success());
        assert_eq!(summary.results.len(), 2);
        assert_eq!(summary.results[0].status_code, Some(7));
        assert_eq!(summary.results[1].stdout, "ok");
    }
}
