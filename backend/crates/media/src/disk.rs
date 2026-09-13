//! Defensive disk-space checks (§6.10). Subtitles are tiny, but we verify there
//! is room before writing extraction output.

use crate::model::MediaError;
use std::path::{Path, PathBuf};

/// Available free space (bytes) on the filesystem containing `path`.
pub fn available_space(path: &Path) -> std::io::Result<u64> {
    let target: PathBuf = if path.exists() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|p| p.to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(".").to_path_buf())
    };
    fs2::available_space(&target)
}

/// Ensure at least `min_bytes` are free on the filesystem containing `path`.
pub fn ensure_space(path: &Path, min_bytes: u64) -> Result<(), MediaError> {
    let avail = available_space(path).map_err(MediaError::Io)?;
    if avail < min_bytes {
        Err(MediaError::DiskSpace(format!(
            "only {} bytes free, need at least {}",
            avail, min_bytes
        )))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_space_is_nonnegative() {
        let avail = available_space(Path::new(".")).unwrap();
        assert!(avail > 0);
    }

    #[test]
    fn ensure_space_passes_for_tiny_requirement() {
        assert!(ensure_space(Path::new("."), 1).is_ok());
    }
}
