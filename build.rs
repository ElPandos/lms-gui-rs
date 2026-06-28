//! Build script — injects git hash, recent log, and build time into the crate
//! as compile-time env vars (`GIT_HASH`, `GIT_LOG`, `BUILD_TIME`) consumed by
//! the changelog template.

use std::process::Command;

fn main() {
    let hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string());

    let log = Command::new("git")
        .args(["log", "--oneline", "-20"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "No git history available".to_string());

    let build_time = Command::new("date")
        .args(["+%Y-%m-%d %H:%M:%S"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=GIT_HASH={}", hash.trim());
    println!("cargo:rustc-env=GIT_LOG={}", log.trim());
    println!("cargo:rustc-env=BUILD_TIME={}", build_time.trim());
}
