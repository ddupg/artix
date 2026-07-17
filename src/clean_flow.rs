use std::path::PathBuf;

use crate::clean::{CleanPlan, CleanRunSummary};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanState {
    Idle,
    Confirming { plan: CleanPlan },
    Running { plan: CleanPlan },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanRequest {
    plan: CleanPlan,
}

impl CleanRequest {
    pub fn into_plan(self) -> CleanPlan {
        self.plan
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanFlow {
    state: CleanState,
}

impl Default for CleanFlow {
    fn default() -> Self {
        Self {
            state: CleanState::Idle,
        }
    }
}

impl CleanFlow {
    pub fn state(&self) -> &CleanState {
        &self.state
    }

    pub fn request(&mut self, plan: CleanPlan) {
        if !matches!(self.state, CleanState::Idle) || plan.commands.is_empty() {
            return;
        }

        self.state = CleanState::Confirming { plan };
    }

    pub fn confirm(&mut self) -> Option<CleanRequest> {
        let CleanState::Confirming { plan } = self.state.clone() else {
            return None;
        };

        self.state = CleanState::Running { plan: plan.clone() };
        Some(CleanRequest { plan })
    }

    pub fn finish(&mut self, summary: CleanRunSummary) -> Option<PathBuf> {
        let CleanState::Running { plan } = &self.state else {
            return None;
        };
        if summary.project_root != plan.project_root {
            return None;
        }

        let project_root = summary.project_root.clone();
        if summary.success() {
            self.state = CleanState::Idle;
        } else {
            self.state = CleanState::Failed {
                message: summary.message(),
            };
        }

        Some(project_root)
    }

    pub fn cancel(&mut self) {
        if matches!(
            self.state,
            CleanState::Confirming { .. } | CleanState::Failed { .. }
        ) {
            self.state = CleanState::Idle;
        }
    }

    pub fn cancel_confirmation(&mut self) {
        if matches!(self.state, CleanState::Confirming { .. }) {
            self.state = CleanState::Idle;
        }
    }
}
