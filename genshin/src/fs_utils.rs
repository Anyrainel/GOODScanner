use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use anyhow::{Context, Result};

/// Remove a file when present, while preserving any real filesystem failure.
///
/// Cache refreshers share this helper so a locked or unreadable cache cannot
/// be mistaken for a successful refresh.
pub(crate) fn remove_file_if_exists(path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to remove cached file: {}", path.display()))
        },
    }
}
