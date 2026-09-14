//! Test helpers: fake ffprobe/ffmpeg scripts for the subprocess tests
//! (§6.3/§6.4). Each helper writes a platform-appropriate script into a temp
//! dir and returns its path.

use std::path::{Path, PathBuf};

fn write_script(dir: &Path, name: &str, windows: &str, unix: &str) -> PathBuf {
    if cfg!(windows) {
        let p = dir.join(format!("{name}.bat"));
        std::fs::write(&p, windows).unwrap();
        p
    } else {
        let p = dir.join(name);
        std::fs::write(&p, unix).unwrap();
        p
    }
}

/// A script that prints `stderr_line` to stderr and exits with `code`,
/// ignoring all arguments.
pub fn failing_tool(dir: &Path, name: &str, stderr_line: &str, code: i32) -> PathBuf {
    write_script(
        dir,
        name,
        &format!("@echo off\r\necho {stderr_line} 1>&2\r\nexit /b {code}\r\n"),
        &format!("#!/bin/sh\necho '{stderr_line}' >&2\nexit {code}\n"),
    )
}

/// A script that sleeps ~5s (longer than the test timeouts) and exits 0.
pub fn sleeper_tool(dir: &Path, name: &str) -> PathBuf {
    write_script(
        dir,
        name,
        "@echo off\r\nping 127.0.0.1 -n 6 >nul\r\nexit /b 0\r\n",
        "#!/bin/sh\nsleep 5\n",
    )
}

/// A script that writes `content` to its **last** argument, prints
/// `stderr_line` to stderr, then exits with `code`. Simulates a tool that
/// leaves a partial file behind before failing.
pub fn writer_tool(dir: &Path, name: &str, content: &str, stderr_line: &str, code: i32) -> PathBuf {
    let windows_lines: Vec<String> = content
        .lines()
        .map(|l| format!("echo {}", escape_bat(l)))
        .collect();
    // No delayed expansion: each bat line is parsed at execution, so %last%
    // works and `!` in the text stays literal.
    let windows = format!(
        "@echo off\r\nset \"last=\"\r\nfor %%a in (%*) do set \"last=%%a\"\r\n(\r\n{}\r\n) > \"%last%\"\r\necho {stderr_line} 1>&2\r\nexit /b {code}\r\n",
        windows_lines.join("\r\n")
    );
    let unix_lines: Vec<String> = content
        .lines()
        .map(|l| format!("'{}'", l.replace('\'', "'\\''")))
        .collect();
    let unix = format!(
        "#!/bin/sh\nlast=\"\"\nfor a in \"$@\"; do last=\"$a\"; done\nprintf '%s\\n' {} > \"$last\"\necho '{stderr_line}' >&2\nexit {code}\n",
        unix_lines.join(" ")
    );
    write_script(dir, name, &windows, &unix)
}

fn escape_bat(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars() {
        match c {
            '&' | '|' | '<' | '>' => {
                out.push('^');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out
}
