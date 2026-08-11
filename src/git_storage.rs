use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::AppContext;
use crate::model::{BrowserEntry, EntryKind, GitContext, GitStatus, RiskLevel, SizeStatus};
use crate::scan::size::measure_size;

const OUTPUT_LIMIT_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStorageTarget {
    pub repo_root: PathBuf,
    pub common_dir: PathBuf,
    pub git_context: GitContext,
}

impl GitStorageTarget {
    pub fn for_repo_root(path: &Path, context: &GitContext) -> Option<Self> {
        let is_worktree_root = context.worktree_root.as_deref() == Some(path);
        let is_bare_root =
            context.worktree_root.is_none() && context.repo_root.as_deref() == Some(path);
        if !is_worktree_root && !is_bare_root {
            return None;
        }

        Some(Self {
            repo_root: context.repo_root.clone()?,
            common_dir: context.common_dir.clone()?,
            git_context: context.clone(),
        })
    }

    pub fn placeholder_entry(&self) -> BrowserEntry {
        BrowserEntry {
            path: self.common_dir.clone(),
            name: "Git storage".to_string(),
            size_bytes: 0,
            reclaimable_bytes: 0,
            size_status: SizeStatus::Incomplete,
            entry_kind: EntryKind::GitStorage,
            git_status: GitStatus::Unknown,
            git_context: self.git_context.clone(),
            risk_level: RiskLevel::Hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStorageAnalysis {
    pub target: GitStorageTarget,
    pub total_size_bytes: u64,
    pub total_size_status: SizeStatus,
    pub loose_object_count: u64,
    pub loose_object_size_bytes: u64,
    pub packed_object_count: u64,
    pub pack_count: u64,
    pub pack_size_bytes: u64,
    pub prune_packable_count: u64,
    pub garbage_count: u64,
    pub garbage_size_bytes: u64,
    pub lfs_size_bytes: u64,
    pub lfs_size_status: SizeStatus,
}

impl GitStorageAnalysis {
    pub fn browser_entry(&self) -> BrowserEntry {
        BrowserEntry {
            path: self.target.common_dir.clone(),
            name: "Git storage".to_string(),
            size_bytes: self.total_size_bytes,
            reclaimable_bytes: self.garbage_size_bytes,
            size_status: self.total_size_status,
            entry_kind: EntryKind::GitStorage,
            git_status: GitStatus::Unknown,
            git_context: self.target.git_context.clone(),
            risk_level: RiskLevel::Hidden,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitGcResult {
    pub target: GitStorageTarget,
    pub success: bool,
    pub status_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl GitGcResult {
    pub fn message(&self) -> String {
        if self.success {
            return "git gc completed".to_string();
        }

        let detail = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        if detail.is_empty() {
            "git gc failed".to_string()
        } else {
            format!("git gc failed: {detail}")
        }
    }
}

pub async fn analyze_git_storage(
    target: GitStorageTarget,
    ctx: &AppContext,
) -> Result<GitStorageAnalysis, String> {
    let total = measure_size(&target.common_dir, ctx).await;
    let lfs_path = target.common_dir.join("lfs");
    let lfs = if lfs_path.exists() {
        measure_size(&lfs_path, ctx).await
    } else {
        crate::scan::size::SizeMeasurement::complete(0)
    };

    let output = run_git(&target.repo_root, ["count-objects", "-v"], ctx).await?;
    if !output.status.success() {
        return Err(command_failure("git count-objects -v", &output));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|err| format!("git count-objects returned non-UTF-8 output: {err}"))?;
    let counts = parse_count_objects(&stdout)?;

    Ok(GitStorageAnalysis {
        target,
        total_size_bytes: total.bytes(),
        total_size_status: total.status(),
        loose_object_count: counts.count,
        loose_object_size_bytes: kib_to_bytes(counts.size_kib),
        packed_object_count: counts.in_pack,
        pack_count: counts.packs,
        pack_size_bytes: kib_to_bytes(counts.size_pack_kib),
        prune_packable_count: counts.prune_packable,
        garbage_count: counts.garbage,
        garbage_size_bytes: kib_to_bytes(counts.size_garbage_kib),
        lfs_size_bytes: lfs.bytes(),
        lfs_size_status: lfs.status(),
    })
}

pub async fn execute_git_gc(target: GitStorageTarget, ctx: &AppContext) -> GitGcResult {
    match run_git(&target.repo_root, ["gc"], ctx).await {
        Ok(output) => GitGcResult {
            target,
            success: output.status.success(),
            status_code: output.status.code(),
            stdout: truncate_output(&output.stdout),
            stderr: truncate_output(&output.stderr),
        },
        Err(err) => GitGcResult {
            target,
            success: false,
            status_code: None,
            stdout: String::new(),
            stderr: err,
        },
    }
}

async fn run_git<const N: usize>(
    repo_root: &Path,
    args: [&str; N],
    ctx: &AppContext,
) -> Result<std::process::Output, String> {
    let sem = ctx.git_semaphore();
    let _permit = sem.acquire().await.expect("semaphore must not be closed");
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| format!("failed to run git: {err}"))
}

fn command_failure(command: &str, output: &std::process::Output) -> String {
    let stderr = truncate_output(&output.stderr);
    if stderr.trim().is_empty() {
        format!("{command} failed with status {:?}", output.status.code())
    } else {
        format!("{command} failed: {}", stderr.trim())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CountObjects {
    count: u64,
    size_kib: u64,
    in_pack: u64,
    packs: u64,
    size_pack_kib: u64,
    prune_packable: u64,
    garbage: u64,
    size_garbage_kib: u64,
}

fn parse_count_objects(output: &str) -> Result<CountObjects, String> {
    let value = |key: &str| -> Result<u64, String> {
        output
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| *name == key)
            .ok_or_else(|| format!("git count-objects output is missing {key}"))?
            .1
            .trim()
            .parse::<u64>()
            .map_err(|err| format!("invalid {key} value from git count-objects: {err}"))
    };

    Ok(CountObjects {
        count: value("count")?,
        size_kib: value("size")?,
        in_pack: value("in-pack")?,
        packs: value("packs")?,
        size_pack_kib: value("size-pack")?,
        prune_packable: value("prune-packable")?,
        garbage: value("garbage")?,
        size_garbage_kib: value("size-garbage")?,
    })
}

fn kib_to_bytes(kib: u64) -> u64 {
    kib.saturating_mul(1024)
}

fn truncate_output(bytes: &[u8]) -> String {
    let end = bytes.len().min(OUTPUT_LIMIT_BYTES);
    let mut text = String::from_utf8_lossy(&bytes[..end]).into_owned();
    if bytes.len() > OUTPUT_LIMIT_BYTES {
        text.push_str("\n... output truncated ...");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{CountObjects, kib_to_bytes, parse_count_objects};

    #[test]
    fn parses_machine_readable_count_objects_output() {
        let output = "count: 12\nsize: 34\nin-pack: 56\npacks: 2\nsize-pack: 78\nprune-packable: 9\ngarbage: 3\nsize-garbage: 10\nalternate: /tmp/objects\n";

        assert_eq!(
            parse_count_objects(output).expect("valid output"),
            CountObjects {
                count: 12,
                size_kib: 34,
                in_pack: 56,
                packs: 2,
                size_pack_kib: 78,
                prune_packable: 9,
                garbage: 3,
                size_garbage_kib: 10,
            }
        );
        assert_eq!(kib_to_bytes(10), 10 * 1024);
    }

    #[test]
    fn rejects_incomplete_count_objects_output() {
        let error = parse_count_objects("count: 1\n").expect_err("missing fields");
        assert!(error.contains("missing size"));
    }
}
