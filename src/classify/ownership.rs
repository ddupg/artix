use std::path::{Path, PathBuf};

pub fn infer_project_roots(markers: &[PathBuf]) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    for marker in markers {
        let root = marker
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| marker.clone());

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
