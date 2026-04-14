use std::fs;
use std::path::{Path, PathBuf};

use crate::rules::Rule;

#[derive(Debug, Clone)]
pub struct DiscoveredCandidate {
    pub path: PathBuf,
    pub rule: Rule,
}

pub fn discover_candidates(roots: &[PathBuf], rules: &[Rule]) -> Vec<DiscoveredCandidate> {
    let mut discovered = Vec::new();

    for root in roots {
        walk(root, rules, &mut discovered);
    }

    discovered
}

fn walk(path: &Path, rules: &[Rule], discovered: &mut Vec<DiscoveredCandidate>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let Some(dir_name) = entry_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if let Some(rule) = rules.iter().find(|rule| rule.dir_name == dir_name) {
            discovered.push(DiscoveredCandidate {
                path: entry_path.clone(),
                rule: rule.clone(),
            });
        }

        walk(&entry_path, rules, discovered);
    }
}
