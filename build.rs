//! Build script that captures the current git SHA at compile time.
//!
//! Emits `KOMANDAN_GIT_SHA` as a cargo rustc-env var. Never fails the build:
//! if git is unavailable or the directory is not a git repo, falls back to
//! the literal string `"unknown"`.

use std::process::Command;

fn main() {
    let sha = git_describe()
        .or_else(git_rev_parse)
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=KOMANDAN_GIT_SHA={sha}");
    println!("cargo:rerun-if-changed=.git/HEAD");
}

/// Runs `git describe --always --tags --dirty=-dirty --abbrev=10`.
fn git_describe() -> Option<String> {
    let out = Command::new("git")
        .args([
            "describe",
            "--always",
            "--tags",
            "--dirty=-dirty",
            "--abbrev=10",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Falls back to `git rev-parse --short=10 HEAD` when describe fails.
fn git_rev_parse() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "--short=10", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
