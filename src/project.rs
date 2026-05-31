use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectMarker {
    pub file_name: &'static str,
    pub kind: ProjectKind,
}

const PROJECT_MARKERS: &[ProjectMarker] = &[
    ProjectMarker {
        file_name: "Cargo.toml",
        kind: ProjectKind::Rust,
    },
    ProjectMarker {
        file_name: "pom.xml",
        kind: ProjectKind::Maven,
    },
    ProjectMarker {
        file_name: "build.gradle",
        kind: ProjectKind::Gradle,
    },
    ProjectMarker {
        file_name: "build.gradle.kts",
        kind: ProjectKind::Gradle,
    },
    ProjectMarker {
        file_name: "package.json",
        kind: ProjectKind::Node,
    },
    ProjectMarker {
        file_name: "pyproject.toml",
        kind: ProjectKind::Python,
    },
];

pub fn detect_project_kinds(path: &Path) -> Vec<ProjectKind> {
    let mut kinds = Vec::new();

    for marker in PROJECT_MARKERS {
        if path.join(marker.file_name).is_file() && !kinds.contains(&marker.kind) {
            kinds.push(marker.kind);
        }
    }

    kinds
}

pub fn is_project_marker_file_name(file_name: &str) -> bool {
    PROJECT_MARKERS
        .iter()
        .any(|marker| marker.file_name == file_name)
}

pub fn project_root_from_marker(marker: &Path) -> PathBuf {
    marker
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| marker.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{ProjectKind, detect_project_kinds, is_project_marker_file_name};

    #[test]
    fn marker_registry_covers_scan_and_clean_project_kinds() {
        assert!(is_project_marker_file_name("Cargo.toml"));
        assert!(is_project_marker_file_name("package.json"));
        assert!(is_project_marker_file_name("pyproject.toml"));
        assert!(is_project_marker_file_name("pom.xml"));
        assert!(is_project_marker_file_name("build.gradle.kts"));
        assert!(!is_project_marker_file_name("README.md"));
    }

    #[test]
    fn detect_project_kinds_deduplicates_gradle_markers() {
        let dir = tempdir().expect("tempdir");
        fs::write(dir.path().join("build.gradle"), "plugins {}\n").expect("write gradle");
        fs::write(dir.path().join("build.gradle.kts"), "plugins {}\n").expect("write gradle kts");

        assert_eq!(detect_project_kinds(dir.path()), vec![ProjectKind::Gradle]);
    }
}
