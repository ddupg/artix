use crate::git_storage::{GitGcResult, GitStorageAnalysis, GitStorageTarget};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum GitStorageState {
    #[default]
    Idle,
    Confirming {
        analysis: GitStorageAnalysis,
    },
    Running {
        target: GitStorageTarget,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStorageRequest {
    target: GitStorageTarget,
}

impl GitStorageRequest {
    pub fn into_target(self) -> GitStorageTarget {
        self.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitStorageFlow {
    state: GitStorageState,
}

impl GitStorageFlow {
    pub fn state(&self) -> &GitStorageState {
        &self.state
    }

    pub fn request(&mut self, analysis: GitStorageAnalysis) {
        if matches!(self.state, GitStorageState::Idle) {
            self.state = GitStorageState::Confirming { analysis };
        }
    }

    pub fn confirm(&mut self) -> Option<GitStorageRequest> {
        let GitStorageState::Confirming { analysis } = self.state.clone() else {
            return None;
        };
        let target = analysis.target;
        self.state = GitStorageState::Running {
            target: target.clone(),
        };
        Some(GitStorageRequest { target })
    }

    pub fn finish(&mut self, result: GitGcResult) -> Option<GitStorageTarget> {
        let GitStorageState::Running { target } = &self.state else {
            return None;
        };
        if result.target.common_dir != target.common_dir {
            return None;
        }

        let target = result.target.clone();
        if result.success {
            self.state = GitStorageState::Idle;
        } else {
            self.state = GitStorageState::Failed {
                message: result.message(),
            };
        }
        Some(target)
    }

    pub fn cancel(&mut self) {
        if matches!(
            self.state,
            GitStorageState::Confirming { .. } | GitStorageState::Failed { .. }
        ) {
            self.state = GitStorageState::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::git_storage::{GitGcResult, GitStorageAnalysis, GitStorageTarget};
    use crate::model::{GitContext, SizeStatus};

    use super::{GitStorageFlow, GitStorageState};

    #[test]
    fn successful_gc_returns_to_idle() {
        let analysis = analysis();
        let mut flow = GitStorageFlow::default();
        flow.request(analysis.clone());
        let request = flow.confirm().expect("request");
        let target = request.into_target();

        let refresh_target = flow
            .finish(GitGcResult {
                target: target.clone(),
                success: true,
                status_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
            .expect("matching completion");

        assert_eq!(refresh_target, target);
        assert_eq!(flow.state(), &GitStorageState::Idle);
    }

    #[test]
    fn failed_gc_stays_visible() {
        let mut flow = GitStorageFlow::default();
        flow.request(analysis());
        let target = flow.confirm().expect("request").into_target();
        flow.finish(GitGcResult {
            target,
            success: false,
            status_code: Some(1),
            stdout: String::new(),
            stderr: "gc refused".to_string(),
        });

        assert!(matches!(
            flow.state(),
            GitStorageState::Failed { message } if message.contains("gc refused")
        ));
    }

    fn analysis() -> GitStorageAnalysis {
        let target = GitStorageTarget {
            repo_root: PathBuf::from("/repo"),
            common_dir: PathBuf::from("/repo/.git"),
            git_context: GitContext::default(),
        };
        GitStorageAnalysis {
            target,
            total_size_bytes: 100,
            total_size_status: SizeStatus::Complete,
            loose_object_count: 1,
            loose_object_size_bytes: 10,
            packed_object_count: 2,
            pack_count: 1,
            pack_size_bytes: 80,
            prune_packable_count: 0,
            garbage_count: 1,
            garbage_size_bytes: 10,
            lfs_size_bytes: 0,
            lfs_size_status: SizeStatus::Complete,
        }
    }
}
