//! Path-traversal protection (§6.9).
//!
//! Every path the API touches is resolved through [`PathGuard`], which
//! canonicalizes (resolving `..` and symlinks) and verifies the result stays
//! inside one of the configured media roots.

use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardError {
    #[error("media root index {0} not found")]
    RootNotFound(usize),
    #[error("path escapes the media root (traversal blocked)")]
    Traversal,
    #[error("path does not exist: {0}")]
    NotFound(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
}

/// Guards a set of canonicalized media roots.
#[derive(Debug, Clone)]
pub struct PathGuard {
    /// Canonicalized roots (only roots that exist are kept).
    roots: Vec<PathBuf>,
}

impl PathGuard {
    /// Build a guard from raw root paths. Non-existent roots are dropped.
    pub fn new(roots: &[PathBuf]) -> std::io::Result<Self> {
        let mut canon = Vec::new();
        for r in roots {
            if r.exists() {
                canon.push(std::fs::canonicalize(r)?);
            }
        }
        Ok(Self { roots: canon })
    }

    /// The canonicalized roots (for display / boot validation).
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// True if at least one root exists and is readable.
    pub fn has_readable_root(&self) -> bool {
        self.roots
            .iter()
            .any(|r| r.is_dir() && std::fs::read_dir(r).is_ok())
    }

    /// Resolve `root_index` + `rel` (relative to that root) to a safe absolute
    /// path. `rel` may be empty (the root itself). Absolute or escaping paths
    /// are rejected.
    pub fn resolve(&self, root_index: usize, rel: &str) -> Result<PathBuf, GuardError> {
        let root = self
            .roots
            .get(root_index)
            .ok_or(GuardError::RootNotFound(root_index))?;

        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(GuardError::Traversal);
        }
        // Reject any `..` component outright (defense in depth).
        if rel_path.components().any(|c| matches!(c, Component::ParentDir)) {
            return Err(GuardError::Traversal);
        }

        let joined = root.join(rel_path);
        let canon = canonicalize_existing(&joined)?;

        // Both `canon` and `root` are canonical, so `starts_with` is safe.
        if !canon.starts_with(root) {
            return Err(GuardError::Traversal);
        }
        Ok(canon)
    }

    /// The inverse of [`resolve`]: find the media root that contains `abs` and
    /// return `(root_index, rel_path)` with `/` separators. Used to turn a
    /// stored absolute output path back into a guard-safe download reference.
    /// The most specific (longest) matching root wins.
    pub fn locate(&self, abs: &Path) -> Option<(usize, String)> {
        let mut best: Option<(usize, usize, &PathBuf)> = None;
        for (i, root) in self.roots.iter().enumerate() {
            if abs.starts_with(root) {
                let depth = root.components().count();
                if best.map(|(_, d, _)| depth > d).unwrap_or(true) {
                    best = Some((i, depth, root));
                }
            }
        }
        let (i, _, root) = best?;
        let rel = abs.strip_prefix(root).ok()?;
        Some((i, rel.to_string_lossy().replace('\\', "/")))
    }
}

/// Canonicalize `path`, tolerating a non-existent final component (e.g. a not-yet-
/// created output file) by canonicalizing the deepest existing ancestor and
/// re-appending the remainder.
fn canonicalize_existing(path: &Path) -> Result<PathBuf, GuardError> {
    if path.exists() {
        return std::fs::canonicalize(path).map_err(io_to_guard);
    }

    let mut pending: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    while !cur.exists() {
        match cur.file_name() {
            Some(name) => {
                pending.push(name.to_os_string());
            }
            None => return Err(GuardError::Traversal),
        }
        match cur.parent() {
            Some(p) => cur = p.to_path_buf(),
            None => return Err(GuardError::NotFound(path.display().to_string())),
        }
    }

    let base = std::fs::canonicalize(&cur).map_err(io_to_guard)?;
    let mut result = base;
    for component in pending.into_iter().rev() {
        result = result.join(component);
    }
    Ok(result)
}

impl From<GuardError> for crate::model::MediaError {
    fn from(e: GuardError) -> Self {
        use crate::model::MediaError;
        match e {
            GuardError::RootNotFound(i) => MediaError::RootNotFound(i),
            GuardError::Traversal => MediaError::Traversal,
            GuardError::NotFound(p) => MediaError::NotFound(p),
            GuardError::PermissionDenied(p) => MediaError::PermissionDenied(p),
        }
    }
}

fn io_to_guard(e: std::io::Error) -> GuardError {
    if e.kind() == std::io::ErrorKind::PermissionDenied {
        GuardError::PermissionDenied(e.to_string())
    } else if e.kind() == std::io::ErrorKind::NotFound {
        GuardError::NotFound(e.to_string())
    } else {
        GuardError::NotFound(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn make_root() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("show/season1")).unwrap();
        fs::write(dir.path().join("show/season1/ep1.mkv"), b"video").unwrap();
        dir
    }

    #[test]
    fn resolves_normal_path() {
        let dir = make_root();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        let p = guard.resolve(0, "show/season1/ep1.mkv").unwrap();
        assert!(p.ends_with("show/season1/ep1.mkv"));
        assert!(p.starts_with(&guard.roots()[0]));
    }

    #[test]
    fn locate_maps_absolute_path_back_to_root_and_rel() {
        let dir = make_root();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        let abs = guard.resolve(0, "show/season1/ep1.mkv").unwrap();
        let (idx, rel) = guard.locate(&abs).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(rel, "show/season1/ep1.mkv");
    }

    #[test]
    fn locate_rejects_path_outside_roots() {
        let dir = make_root();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        assert!(guard.locate(Path::new("/etc/passwd")).is_none());
    }

    #[test]
    fn rejects_parent_dir_traversal() {
        let dir = make_root();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        assert!(matches!(
            guard.resolve(0, "../etc/passwd"),
            Err(GuardError::Traversal)
        ));
        assert!(matches!(
            guard.resolve(0, "show/../../secret"),
            Err(GuardError::Traversal)
        ));
    }

    #[test]
    fn rejects_absolute_path() {
        let dir = make_root();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        assert!(matches!(
            guard.resolve(0, "/etc/passwd"),
            Err(GuardError::Traversal)
        ));
    }

    #[test]
    fn rejects_bad_root_index() {
        let dir = make_root();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        assert!(matches!(
            guard.resolve(5, "x"),
            Err(GuardError::RootNotFound(5))
        ));
    }

    #[test]
    fn symlink_escape_blocked() {
        let dir = make_root();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), b"top secret").unwrap();

        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        {
            // Windows symlink creation may require privileges; skip if it fails.
            if std::os::windows::fs::symlink_dir(outside.path(), &link).is_err() {
                return;
            }
        }

        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        // The symlink resolves outside the root -> must be rejected.
        assert!(matches!(
            guard.resolve(0, "link/secret.txt"),
            Err(GuardError::Traversal)
        ));
    }

    #[test]
    fn unicode_paths_work() {
        let dir = tempfile::tempdir().unwrap();
        let jp = dir.path().join("アニメ/シーズン1");
        fs::create_dir_all(&jp).unwrap();
        fs::write(jp.join("第1話.mkv"), b"v").unwrap();

        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        let p = guard.resolve(0, "アニメ/シーズン1/第1話.mkv").unwrap();
        assert!(p.exists());
    }
}
