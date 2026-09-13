//! Library browsing: root listing, directory listing, and filename search
//! (FR-1, FR-3, FR-4). All paths go through [`PathGuard`].

use crate::model::MediaError;
use crate::path_guard::PathGuard;
use serde::Serialize;
use std::path::Path;

/// Container extensions treated as video files (FR-4). MKV is the priority case.
pub const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "avi", "m2ts", "ts", "mov", "wmv", "flv", "webm", "mpg", "mpeg",
];

/// True if the filename has a known video extension.
pub fn is_video(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// A configured media root.
#[derive(Serialize, Clone, Debug)]
pub struct RootInfo {
    pub index: usize,
    pub name: String,
    pub path: String,
}

/// A folder or file entry in a listing.
#[derive(Serialize, Clone, Debug)]
pub struct Entry {
    pub name: String,
    /// Path relative to the root, using `/` separators.
    pub rel_path: String,
    pub is_dir: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub is_video: bool,
    /// Cached ffprobe metadata (FR-2); present only when the file has been
    /// probed (see `MediaCache`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle_count: Option<u32>,
}

/// Result of listing a directory.
#[derive(Serialize, Debug)]
pub struct TreeResult {
    pub folders: Vec<Entry>,
    pub files: Vec<Entry>,
}

/// List the configured media roots.
pub fn list_roots(guard: &PathGuard) -> Vec<RootInfo> {
    guard
        .roots()
        .iter()
        .enumerate()
        .map(|(i, p)| RootInfo {
            index: i,
            name: p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("root")
                .to_string(),
            path: display_path(p),
        })
        .collect()
}

/// A path for display: forward slashes, and the Windows verbatim prefix
/// (`\\?\`) stripped (restoring `\\server\share` for UNC paths).
pub fn display_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    // Strip the verbatim prefix *before* normalizing separators.
    let s = if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
        // `\\?\UNC\server\share` -> `\\server\share`
        format!("\\\\{stripped}")
    } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        s.to_string()
    };
    s.replace('\\', "/")
}

/// List the folders and (video) files directly under `rel` in `root_index`.
///
/// With `show_all` false, only folders and video files are returned (FR-1).
pub fn list_dir(
    guard: &PathGuard,
    root_index: usize,
    rel: &str,
    show_all: bool,
) -> Result<TreeResult, MediaError> {
    let abs = guard.resolve(root_index, rel)?;
    if !abs.is_dir() {
        return Err(MediaError::NotADirectory(abs.display().to_string()));
    }
    let rd = std::fs::read_dir(&abs).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            MediaError::PermissionDenied(abs.display().to_string())
        } else {
            MediaError::Io(e)
        }
    })?;

    let mut folders = Vec::new();
    let mut files = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let rel_path = join_rel(rel, &name);
        if file_type.is_dir() {
            folders.push(Entry {
                name,
                rel_path,
                is_dir: true,
                size: None,
                is_video: false,
                duration_s: None,
                container: None,
                audio_count: None,
                subtitle_count: None,
            });
        } else if file_type.is_file() {
            let video = is_video(&name);
            if show_all || video {
                let size = entry.metadata().ok().map(|m| m.len());
                files.push(Entry {
                    name,
                    rel_path,
                    is_dir: false,
                    size,
                    is_video: video,
                    duration_s: None,
                    container: None,
                    audio_count: None,
                    subtitle_count: None,
                });
            }
        }
    }

    folders.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(TreeResult { folders, files })
}

/// Recursively search for files whose name contains `query` (case-insensitive)
/// under `root_index` (FR-3).
pub fn search(
    guard: &PathGuard,
    root_index: usize,
    query: &str,
    show_all: bool,
) -> Result<Vec<Entry>, MediaError> {
    let abs = guard.resolve(root_index, "")?;
    if !abs.is_dir() {
        return Err(MediaError::NotADirectory(abs.display().to_string()));
    }
    let q = query.to_ascii_lowercase();
    let mut results = Vec::new();

    for entry in walkdir::WalkDir::new(&abs).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let video = is_video(&name);
        if !show_all && !video {
            continue;
        }
        if !name.to_ascii_lowercase().contains(&q) {
            continue;
        }
        let rel = entry
            .path()
            .strip_prefix(&abs)
            .unwrap_or(entry.path());
        let size = entry.metadata().ok().map(|m| m.len());
        results.push(Entry {
            name,
            rel_path: display_path(rel),
            is_dir: false,
            size,
            is_video: video,
            duration_s: None,
            container: None,
            audio_count: None,
            subtitle_count: None,
        });
    }

    results.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(results)
}

/// Join a parent relative path and a child name using `/` separators.
fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup() -> (tempfile::TempDir, PathGuard) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("Show/Season 1")).unwrap();
        fs::write(dir.path().join("Show/Season 1/E01.mkv"), b"v").unwrap();
        fs::write(dir.path().join("Show/Season 1/E02.mkv"), b"v").unwrap();
        fs::write(dir.path().join("Show/notes.txt"), b"junk").unwrap();
        let guard = PathGuard::new(&[dir.path().to_path_buf()]).unwrap();
        (dir, guard)
    }

    #[test]
    fn is_video_detects_extensions() {
        assert!(is_video("movie.mkv"));
        assert!(is_video("movie.MP4"));
        assert!(is_video("movie.ts"));
        assert!(!is_video("notes.txt"));
        assert!(!is_video("noext"));
    }

    #[test]
    fn list_dir_hides_junk_by_default() {
        let (_dir, guard) = setup();
        let tree = list_dir(&guard, 0, "Show/Season 1", false).unwrap();
        assert_eq!(tree.files.len(), 2, "expected 2 video files, got {:?}", tree.files);
        assert!(tree.files.iter().all(|f| f.is_video));
        // notes.txt is in the parent, not here; folders empty.
        assert!(tree.folders.is_empty());
    }

    #[test]
    fn list_dir_show_all_includes_junk() {
        let (_dir, guard) = setup();
        let tree = list_dir(&guard, 0, "Show", true).unwrap();
        assert!(tree.folders.iter().any(|f| f.name == "Season 1"));
    }

    #[test]
    fn search_finds_by_name() {
        let (_dir, guard) = setup();
        let results = search(&guard, 0, "e01", false).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].name.contains("E01"));
    }

    #[test]
    fn search_case_insensitive() {
        let (_dir, guard) = setup();
        let results = search(&guard, 0, "e02", false).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn join_rel_handles_root() {
        assert_eq!(join_rel("", "a.mkv"), "a.mkv");
        assert_eq!(join_rel("Show", "a.mkv"), "Show/a.mkv");
        assert_eq!(join_rel("Show/", "a.mkv"), "Show/a.mkv");
    }

    #[test]
    fn display_path_keeps_unix_paths() {
        assert_eq!(display_path(Path::new("/a/b/c")), "/a/b/c");
    }

    #[cfg(windows)]
    #[test]
    fn display_path_strips_verbatim_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\media\show")),
            "C:/media/show"
        );
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            "//server/share"
        );
    }
}
