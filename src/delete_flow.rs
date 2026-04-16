use crate::delete::{DeleteMode, delete_directories};
use crate::model::{BrowserEntry, GitStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteTargetKind {
    CleanupCandidate,
    TrackedOrUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteIntent {
    Confirm {
        target_kind: DeleteTargetKind,
        requires_extra_confirmation: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteState {
    Idle,
    Confirming {
        entry: BrowserEntry,
        target_kind: DeleteTargetKind,
        requires_extra_confirmation: bool,
        requested_mode: Option<DeleteMode>,
    },
    AwaitingExtraConfirmation {
        entry: BrowserEntry,
        mode: DeleteMode,
        target_kind: DeleteTargetKind,
    },
    Running {
        entry: BrowserEntry,
        mode: DeleteMode,
    },
    Failed {
        message: String,
    },
}

pub fn delete_intent_for(entry: &BrowserEntry) -> DeleteIntent {
    let tracked_or_unknown = matches!(entry.git_status, GitStatus::Tracked | GitStatus::Unknown);

    DeleteIntent::Confirm {
        target_kind: if tracked_or_unknown {
            DeleteTargetKind::TrackedOrUnknown
        } else {
            DeleteTargetKind::CleanupCandidate
        },
        requires_extra_confirmation: tracked_or_unknown,
    }
}

pub fn execute_delete(entry: &BrowserEntry, mode: DeleteMode) -> Result<String, String> {
    delete_directories(std::slice::from_ref(&entry.path), mode.clone())?;

    let mode_label = match mode {
        DeleteMode::Trash => "moved to trash",
        DeleteMode::Permanent { .. } => "deleted permanently",
    };

    Ok(format!("{} {}", entry.path.display(), mode_label))
}
