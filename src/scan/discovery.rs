use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::project::is_project_marker_file_name;
use crate::rules::Rule;

#[derive(Debug, Clone)]
pub(super) struct DiscoveredCandidate {
    pub(super) path: PathBuf,
    pub(super) project_root: PathBuf,
    pub(super) rule: Rule,
}

pub(super) fn discover_workspace(roots: &[PathBuf], rules: &[Rule]) -> Vec<DiscoveredCandidate> {
    let mut candidate_rules = BTreeMap::<PathBuf, Rule>::new();
    let mut project_roots = BTreeSet::<PathBuf>::new();

    for root in roots {
        discover_root(root, rules, &mut candidate_rules, &mut project_roots);
    }

    let candidate_paths = candidate_rules.keys().cloned().collect::<BTreeSet<_>>();
    candidate_rules
        .into_iter()
        .filter(|(path, _)| {
            !path
                .ancestors()
                .skip(1)
                .any(|ancestor| candidate_paths.contains(ancestor))
        })
        .map(|(path, rule)| DiscoveredCandidate {
            project_root: nearest_ancestor(&path, &project_roots)
                .or_else(|| nearest_ancestor(&path, roots))
                .unwrap_or_else(|| path.clone()),
            path,
            rule,
        })
        .collect()
}

fn discover_root(
    root: &Path,
    rules: &[Rule],
    candidate_rules: &mut BTreeMap<PathBuf, Rule>,
    project_roots: &mut BTreeSet<PathBuf>,
) {
    let mut entries = WalkDir::new(root)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter();

    while let Some(entry) = entries.next() {
        let Ok(entry) = entry else {
            continue;
        };

        if entry.depth() == 0 {
            continue;
        }

        let file_type = entry.file_type();
        let file_name = entry.file_name().to_str();

        if file_type.is_dir() {
            if file_name == Some(".git") {
                entries.skip_current_dir();
                continue;
            }

            if let Some(rule) =
                file_name.and_then(|name| rules.iter().find(|rule| rule.dir_name == name).cloned())
            {
                candidate_rules.insert(entry.path().to_path_buf(), rule);
                entries.skip_current_dir();
            }

            continue;
        }

        if file_type.is_file()
            && file_name.is_some_and(is_project_marker_file_name)
            && let Some(project_root) = entry.path().parent()
        {
            project_roots.insert(project_root.to_path_buf());
        }
    }
}

fn nearest_ancestor<'a>(
    path: &Path,
    roots: impl IntoIterator<Item = &'a PathBuf>,
) -> Option<PathBuf> {
    roots
        .into_iter()
        .filter(|root| path.starts_with(root.as_path()))
        .max_by_key(|root| root.components().count())
        .cloned()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::discover_workspace;
    use crate::rules::default_rules;

    #[test]
    fn discovers_hidden_ignored_candidates_and_assigns_the_nearest_project() {
        let workspace = tempdir().expect("workspace");
        let app = workspace.path().join("services/api");
        fs::create_dir_all(app.join(".venv/bin")).expect("create virtualenv");
        fs::write(workspace.path().join(".gitignore"), "services/api/.venv/\n")
            .expect("write gitignore");
        fs::write(app.join("pyproject.toml"), "[project]\nname = \"api\"\n")
            .expect("write project marker");

        let candidates = discover_workspace(&[workspace.path().to_path_buf()], &default_rules());

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, app.join(".venv"));
        assert_eq!(candidates[0].project_root, app);
        assert_eq!(candidates[0].rule.id, "python.venv");
    }

    #[test]
    fn treats_candidates_and_git_metadata_as_traversal_boundaries() {
        let workspace = tempdir().expect("workspace");
        let dependency = workspace.path().join("node_modules/dep");
        fs::write(workspace.path().join("package.json"), "{}\n").expect("write marker");
        fs::create_dir_all(dependency.join("node_modules/nested"))
            .expect("create nested dependency");
        fs::write(dependency.join("package.json"), "{}\n").expect("write dependency marker");
        fs::create_dir_all(workspace.path().join(".git/target/debug"))
            .expect("create git metadata candidate");

        let candidates = discover_workspace(
            &[workspace.path().to_path_buf(), dependency],
            &default_rules(),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, workspace.path().join("node_modules"));
        assert_eq!(candidates[0].project_root, workspace.path());
    }

    #[test]
    fn deduplicates_candidates_from_overlapping_roots() {
        let workspace = tempdir().expect("workspace");
        let app = workspace.path().join("app");
        fs::create_dir_all(app.join("target/debug")).expect("create target");
        fs::write(app.join("Cargo.toml"), "[package]\nname = \"app\"\n").expect("write marker");

        let candidates = discover_workspace(
            &[workspace.path().to_path_buf(), app.clone()],
            &default_rules(),
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].path, app.join("target"));
        assert_eq!(candidates[0].project_root, app);
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let workspace = tempdir().expect("workspace");
        let outside = tempdir().expect("outside");
        fs::create_dir_all(outside.path().join("target/debug")).expect("create target");
        symlink(outside.path(), workspace.path().join("linked")).expect("link outside workspace");

        let candidates = discover_workspace(&[workspace.path().to_path_buf()], &default_rules());

        assert!(candidates.is_empty());
    }
}
