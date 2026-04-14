use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteMode {
    Trash,
    Permanent { confirmed: bool },
}

pub fn delete_directories(paths: &[PathBuf], mode: DeleteMode) -> Result<(), String> {
    match mode {
        DeleteMode::Trash => {
            for path in paths {
                trash::delete(path).map_err(|err| err.to_string())?;
            }
            Ok(())
        }
        DeleteMode::Permanent { confirmed } => {
            if !confirmed {
                return Err("permanent delete requires explicit confirmation".into());
            }

            for path in paths {
                fs::remove_dir_all(path).map_err(|err| err.to_string())?;
            }
            Ok(())
        }
    }
}
