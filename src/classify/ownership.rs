use std::path::{Path, PathBuf};

use crate::project::project_root_from_marker;

pub fn infer_project_roots(markers: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    for marker in markers {
        let root = project_root_from_marker(marker);

        if !roots.iter().any(|existing| existing == &root) {
            roots.push(root);
        }
    }

    roots
}

pub fn resolve_owner_project(candidate: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    roots
        .iter()
        .filter(|root| candidate.starts_with(root.as_path()))
        .max_by_key(|root| root.components().count())
        .cloned()
}
