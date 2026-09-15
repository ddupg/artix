use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::config::{Config, DeleteConfig, TrashBackend};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteMode {
    Trash,
    Permanent { confirmed: bool },
}

pub fn delete_directories(paths: &[PathBuf], mode: DeleteMode) -> Result<(), String> {
    delete_directories_with_config(paths, mode, &Config::default().delete)
}

pub fn delete_directories_with_config(
    paths: &[PathBuf],
    mode: DeleteMode,
    delete_config: &DeleteConfig,
) -> Result<(), String> {
    match mode {
        DeleteMode::Trash => {
            for path in paths {
                move_to_trash(path, delete_config)?;
            }
            Ok(())
        }
        DeleteMode::Permanent { confirmed } => {
            if !confirmed {
                return Err("permanent delete requires explicit confirmation".into());
            }

            for path in paths {
                delete_path_permanently(path)?;
            }
            Ok(())
        }
    }
}

fn delete_path_permanently(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
    if metadata.file_type().is_dir() {
        match fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(err) if is_too_many_open_files(&err) => remove_dir_all_bounded(path),
            Err(err) => Err(err.to_string()),
        }
    } else {
        fs::remove_file(path).map_err(|err| err.to_string())
    }
}

#[cfg(unix)]
fn is_too_many_open_files(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::EMFILE)
}

#[cfg(not(unix))]
fn is_too_many_open_files(_err: &io::Error) -> bool {
    false
}

fn remove_dir_all_bounded(path: &Path) -> Result<(), String> {
    for entry in WalkDir::new(path)
        .follow_links(false)
        .follow_root_links(false)
        .contents_first(true)
        .max_open(1)
    {
        let entry = entry.map_err(|err| err.to_string())?;
        let result = if entry.file_type().is_dir() {
            fs::remove_dir(entry.path())
        } else {
            fs::remove_file(entry.path())
        };

        if let Err(err) = result
            && err.kind() != io::ErrorKind::NotFound
        {
            return Err(err.to_string());
        }
    }

    Ok(())
}

fn move_to_trash(path: &Path, delete_config: &DeleteConfig) -> Result<(), String> {
    if matches!(delete_config.trash_backend, TrashBackend::Builtin) {
        return move_to_builtin_trash(path);
    }

    trash::delete(path).or_else(|trash_err| {
        #[cfg(target_os = "macos")]
        {
            move_to_builtin_trash(path).map_err(|builtin_err| {
                format!("{trash_err}; builtin trash fallback failed: {builtin_err}")
            })
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(trash_err.to_string())
        }
    })
}

fn move_to_builtin_trash(path: &Path) -> Result<(), String> {
    let trash_dir = user_trash_dir()?;
    fs::create_dir_all(&trash_dir).map_err(|err| err.to_string())?;

    let file_name = path
        .file_name()
        .ok_or_else(|| format!("missing file name for {}", path.display()))?;
    let destination = unique_trash_destination(&trash_dir, file_name);

    fs::rename(path, &destination).map_err(|err| err.to_string())
}

fn user_trash_dir() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or_else(|| "HOME is not set".to_string())?;
    Ok(PathBuf::from(home).join(".Trash"))
}

fn unique_trash_destination(trash_dir: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let candidate = trash_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("trash-item");
    let extension = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str());

    let mut counter = 1usize;
    loop {
        let suffix = match extension {
            Some(extension) => format!("{stem}-{counter}.{extension}"),
            None => format!("{stem}-{counter}"),
        };
        let candidate = trash_dir.join(suffix);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}
