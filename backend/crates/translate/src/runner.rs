//! Spawns the llm-subtrans subprocess with the correct environment (D1) and
//! provides a best-effort process-tree kill for cancellation (FR-17).

use crate::command::CommandSpec;
use std::path::Path;
use tokio::process::Command;

/// Spawn a translation process described by `spec`.
///
/// - Runs `<python> <home>/<spec.script> <args...>`.
/// - Sets `cwd` and `PYTHONPATH` to the llm-subtrans home so its `scripts.*`
///   and top-level imports resolve.
/// - Applies `spec.env` (API keys) to the child environment.
/// - Pipes stdout/stderr for line streaming; stdin is null.
///
/// The caller takes ownership of the returned [`tokio::process::Child`] and is
/// responsible for reading its output and killing it on cancel.
pub async fn spawn_translation(
    home: &Path,
    python: &str,
    spec: &CommandSpec,
) -> anyhow::Result<tokio::process::Child> {
    let script = home.join(&spec.script);
    if !script.exists() {
        anyhow::bail!(
            "llm-subtrans script not found at {} — is LLM_SUBTRANS_HOME set correctly?",
            script.display()
        );
    }

    let mut cmd = Command::new(python);
    cmd.arg(&script)
        .args(&spec.args)
        .current_dir(home)
        .env("PYTHONPATH", home)
        .envs(&spec.env)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .stdin(std::process::Stdio::null());

    // Put the child in its own process group so cancellation can kill the whole
    // tree, not just the direct python process (Unix).
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    tracing::info!(script = %script.display(), "spawning llm-subtrans");
    let child = cmd.spawn().map_err(|e| {
        tracing::error!(script = %script.display(), error = %e, "failed to spawn llm-subtrans");
        e
    })?;
    Ok(child)
}

/// Best-effort kill of a child process and (on Unix) its entire process group.
///
/// This is what cancellation calls so the underlying llm-subtrans/ffmpeg process
/// is actually terminated, not just hidden in the UI (FR-17).
pub fn kill_tree(child: &mut tokio::process::Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        if let Some(pid) = pid {
            // Negative pid targets the whole process group.
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
    let _ = child.start_kill();
}
