use crate::config::{Config, DeleteConfig};
use crate::delete::{DeleteMode, delete_directories_with_config};
use crate::model::{BrowserEntry, GitStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteState {
    Idle,
    Confirming {
        entry: BrowserEntry,
        requires_extra_confirmation: bool,
    },
    AwaitingExtraConfirmation {
        entry: BrowserEntry,
    },
    Running {
        entry: BrowserEntry,
        mode: DeleteMode,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRequest {
    entry: BrowserEntry,
    mode: DeleteMode,
}

impl DeleteRequest {
    pub fn into_parts(self) -> (BrowserEntry, DeleteMode) {
        (self.entry, self.mode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteFlow {
    state: DeleteState,
}

impl Default for DeleteFlow {
    fn default() -> Self {
        Self {
            state: DeleteState::Idle,
        }
    }
}

impl DeleteFlow {
    pub fn state(&self) -> &DeleteState {
        &self.state
    }

    pub fn request(&mut self, entry: BrowserEntry) {
        if !matches!(self.state, DeleteState::Idle) {
            return;
        }

        let requires_extra_confirmation =
            matches!(entry.git_status, GitStatus::Tracked | GitStatus::Unknown);
        self.state = DeleteState::Confirming {
            entry,
            requires_extra_confirmation,
        };
    }

    pub fn choose_trash(&mut self) -> Option<DeleteRequest> {
        let DeleteState::Confirming { entry, .. } = self.state.clone() else {
            return None;
        };

        Some(self.start(entry, DeleteMode::Trash))
    }

    pub fn confirm_permanent(&mut self) -> Option<DeleteRequest> {
        match self.state.clone() {
            DeleteState::Confirming {
                entry,
                requires_extra_confirmation: true,
            } => {
                self.state = DeleteState::AwaitingExtraConfirmation { entry };
                None
            }
            DeleteState::Confirming {
                entry,
                requires_extra_confirmation: false,
            }
            | DeleteState::AwaitingExtraConfirmation { entry } => {
                Some(self.start(entry, DeleteMode::Permanent { confirmed: true }))
            }
            _ => None,
        }
    }

    pub fn finish(&mut self, result: Result<String, String>) -> bool {
        if !matches!(self.state, DeleteState::Running { .. }) {
            return false;
        }

        match result {
            Ok(_) => {
                self.state = DeleteState::Idle;
                true
            }
            Err(message) => {
                self.state = DeleteState::Failed { message };
                false
            }
        }
    }

    pub fn cancel(&mut self) {
        if matches!(
            self.state,
            DeleteState::Confirming { .. }
                | DeleteState::AwaitingExtraConfirmation { .. }
                | DeleteState::Failed { .. }
        ) {
            self.state = DeleteState::Idle;
        }
    }

    fn start(&mut self, entry: BrowserEntry, mode: DeleteMode) -> DeleteRequest {
        self.state = DeleteState::Running {
            entry: entry.clone(),
            mode: mode.clone(),
        };
        DeleteRequest { entry, mode }
    }
}

pub fn execute_delete(entry: &BrowserEntry, mode: DeleteMode) -> Result<String, String> {
    execute_delete_with_config(entry, mode, &Config::default().delete)
}

pub fn execute_delete_with_config(
    entry: &BrowserEntry,
    mode: DeleteMode,
    delete_config: &DeleteConfig,
) -> Result<String, String> {
    delete_directories_with_config(
        std::slice::from_ref(&entry.path),
        mode.clone(),
        delete_config,
    )?;

    let mode_label = match mode {
        DeleteMode::Trash => "moved to trash",
        DeleteMode::Permanent { .. } => "deleted permanently",
    };

    Ok(format!("{} {}", entry.path.display(), mode_label))
}
