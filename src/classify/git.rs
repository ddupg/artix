use std::fs;
use std::path::Path;

use crate::model::GitStatus;
use crate::rules::Rule;

pub fn classify_git_status(candidate: &Path, project_root: &Path, rule: &Rule) -> GitStatus {
    let gitignore_path = project_root.join(".gitignore");
    let Ok(contents) = fs::read_to_string(gitignore_path) else {
        return GitStatus::Unknown;
    };

    let Some(relative_path) = candidate.strip_prefix(project_root).ok() else {
        return GitStatus::Unknown;
    };
    let relative = relative_path.to_string_lossy().replace('\\', "/");
    let dir_rule = format!("{}/", rule.dir_name);

    if contents.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && (trimmed == dir_rule || trimmed == relative || trimmed == format!("{relative}/"))
    }) {
        GitStatus::Ignored
    } else {
        GitStatus::Unknown
    }
}
