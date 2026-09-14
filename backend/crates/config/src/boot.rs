//! Boot-time configuration validation (§6.12).
//!
//! Fails fast — before serving — with a clear message naming the missing
//! dependency, instead of failing later mid-job.

use crate::app_config::AppConfig;
use std::path::{Path, PathBuf};

/// The result of a successful boot validation.
#[derive(Debug, Clone)]
pub struct BootReport {
    /// Number of configured media roots that exist and are readable.
    pub readable_roots: usize,
    /// Total number of configured media roots.
    pub total_roots: usize,
}

/// Validate the configuration on startup (§6.12).
///
/// Returns `Ok` with a report, or `Err` with a list of human-readable problems
/// (each naming the offending dependency).
pub fn validate(config: &AppConfig) -> Result<BootReport, Vec<String>> {
    let mut problems: Vec<String> = Vec::new();

    // 1. At least one readable media root.
    let readable = config
        .media_roots
        .iter()
        .filter(|r| r.is_dir() && std::fs::read_dir(r).is_ok())
        .count();
    if readable == 0 {
        let list = if config.media_roots.is_empty() {
            "none configured".to_string()
        } else {
            config
                .media_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        problems.push(format!(
            "no readable media root — set MEDIA_ROOT/MEDIA_ROOTS (tried: {list})"
        ));
    }

    // 2. DB path writable.
    if let Err(e) = crate::db::check_writable(&config.db_path) {
        problems.push(format!(
            "database path not writable ({}): {e}",
            config.db_path.display()
        ));
    }

    // 3. ffmpeg / ffprobe present and executable.
    for (bin, var) in [
        (&config.ffmpeg_bin, "FFMPEG_BIN"),
        (&config.ffprobe_bin, "FFPROBE_BIN"),
    ] {
        if find_program(bin).is_none() {
            problems.push(format!("required binary not found: {bin} (set {var})"));
        }
    }

    // 4. Python interpreter + the bundled llm-subtrans scripts.
    if find_program(&config.python_bin).is_none() {
        problems.push(format!(
            "python interpreter not found: {} (set PYTHON_BIN)",
            config.python_bin
        ));
    }
    let scripts = config.llm_subtrans_home.join("scripts");
    if !scripts.is_dir() {
        problems.push(format!(
            "llm-subtrans scripts directory not found: {} (set LLM_SUBTRANS_HOME)",
            scripts.display()
        ));
    }

    if problems.is_empty() {
        Ok(BootReport {
            readable_roots: readable,
            total_roots: config.media_roots.len(),
        })
    } else {
        Err(problems)
    }
}

/// Locate a program by name (searching `PATH`) or by absolute/relative path.
///
/// Returns the resolved path when found. On Windows the `PATHEXT` extensions
/// (`.exe`, `.bat`, ...) are tried for bare names, so a stubbed `ffmpeg.bat`
/// on `PATH` resolves correctly.
pub fn find_program(bin: &str) -> Option<PathBuf> {
    let p = Path::new(bin);
    // Absolute, or contains a path separator: check directly.
    if p.is_absolute() || bin.contains(std::path::MAIN_SEPARATOR) || bin.contains('/') {
        return if is_executable_file(p) {
            Some(p.to_path_buf())
        } else {
            None
        };
    }
    // Otherwise search PATH.
    let path_var = std::env::var("PATH").ok()?;
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(sep) {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(bin);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
        if cfg!(windows) {
            if let Ok(pathext) = std::env::var("PATHEXT") {
                for ext in pathext.split(';') {
                    let ext = ext.trim();
                    if ext.is_empty() {
                        continue;
                    }
                    let c = Path::new(dir).join(format!("{bin}{ext}"));
                    if is_executable_file(&c) {
                        return Some(c);
                    }
                }
            }
        }
    }
    None
}

fn is_executable_file(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_config::OverwriteBehavior;

    fn make_exec(path: &Path) {
        std::fs::write(path, b"#!/bin/sh\necho ok\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perm = std::fs::metadata(path).unwrap().permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(path, perm).unwrap();
        }
    }

    fn base_config() -> AppConfig {
        AppConfig {
            media_roots: vec![],
            db_path: PathBuf::from("x.db"),
            data_dir: PathBuf::from("data"),
            app_secret: "test".into(),
            auth_token: String::new(),
            host: "127.0.0.1".into(),
            port: 8080,
            llm_subtrans_home: PathBuf::from("/opt/llm-subtrans"),
            python_bin: "python3".into(),
            ffmpeg_bin: "ffmpeg".into(),
            ffprobe_bin: "ffprobe".into(),
            concurrency: 2,
            output_pattern: "{name}.{lang}.srt".into(),
            overwrite_behavior: OverwriteBehavior::Suffix,
            default_target_language: "English".into(),
            ffprobe_timeout_secs: 30,
            ffmpeg_timeout_secs: 300,
            web_root: PathBuf::from("frontend/dist"),
            log_level: "info".into(),
        }
    }

    /// A fully-valid temp layout: readable root, writable db, fake binaries.
    fn valid_layout() -> (tempfile::TempDir, AppConfig) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("media");
        std::fs::create_dir_all(&root).unwrap();

        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let ffmpeg = bin.join("ffmpeg");
        let ffprobe = bin.join("ffprobe");
        let python = bin.join("python3");
        make_exec(&ffmpeg);
        make_exec(&ffprobe);
        make_exec(&python);

        let home = dir.path().join("llm-subtrans");
        std::fs::create_dir_all(home.join("scripts")).unwrap();

        let db = dir.path().join("data").join("t.db");

        let mut cfg = base_config();
        cfg.media_roots = vec![root];
        cfg.db_path = db;
        cfg.ffmpeg_bin = ffmpeg.to_string_lossy().to_string();
        cfg.ffprobe_bin = ffprobe.to_string_lossy().to_string();
        cfg.python_bin = python.to_string_lossy().to_string();
        cfg.llm_subtrans_home = home;
        (dir, cfg)
    }

    #[test]
    fn valid_config_passes() {
        let (_dir, cfg) = valid_layout();
        let report = validate(&cfg).unwrap();
        assert_eq!(report.readable_roots, 1);
        assert_eq!(report.total_roots, 1);
    }

    #[test]
    fn missing_ffmpeg_is_named() {
        let (_dir, mut cfg) = valid_layout();
        cfg.ffmpeg_bin = "definitely-not-a-real-binary-xyz".into();
        let err = validate(&cfg).unwrap_err();
        assert!(
            err.iter()
                .any(|m| m.contains("definitely-not-a-real-binary-xyz")),
            "expected ffmpeg to be named, got: {err:?}"
        );
    }

    #[test]
    fn no_media_root_is_named() {
        let (_dir, mut cfg) = valid_layout();
        cfg.media_roots = vec![PathBuf::from("/does/not/exist/root")];
        let err = validate(&cfg).unwrap_err();
        assert!(
            err.iter().any(|m| m.contains("media root")),
            "expected a media-root problem, got: {err:?}"
        );
    }

    #[test]
    fn missing_llm_subtrans_is_named() {
        let (_dir, mut cfg) = valid_layout();
        cfg.llm_subtrans_home = PathBuf::from("/no/such/llm-subtrans");
        let err = validate(&cfg).unwrap_err();
        assert!(
            err.iter().any(|m| m.contains("llm-subtrans")),
            "expected an llm-subtrans problem, got: {err:?}"
        );
    }

    #[test]
    fn find_program_resolves_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("tool");
        make_exec(&exe);
        let found = find_program(exe.to_string_lossy().as_ref()).unwrap();
        assert_eq!(found, exe);
    }

    #[test]
    fn find_program_missing_returns_none() {
        assert!(find_program("no-such-program-xyz-123").is_none());
    }
}
