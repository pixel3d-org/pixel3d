//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust. External tools (`cargo`, `git`) are invoked through
//! `std::process::Command`.

mod bench_scale;
mod changed;
mod ci;
mod emdash;
mod layers;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci       everything CI runs: fmt, clippy, test, layers, emdash, deny, release build, bench-scale
  ci-fast  run before every commit: fmt, clippy, tests of changed crates, layers, emdash
  bench-scale [--large] [--kernel <name>]
           run the scale_* benches at n, 4n, 16n and fail if a kernel grows faster than n^1.25
  layers   enforce the crate layer table (docs/architecture.md)
  emdash   fail if any tracked or untracked file contains an em dash (U+2014) or en dash (U+2013)
  help     print this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let result = match args.first().map(String::as_str) {
        Some("bench-scale") => bench_scale::run(&rest),
        Some("ci") => ci::ci(),
        Some("ci-fast") => ci::ci_fast(),
        Some("layers") => cmd_layers(),
        Some("emdash") => emdash::run(&root()),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
fn root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap_or(manifest).to_path_buf()
}

/// A `cargo` command running in the workspace root.
fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

/// Full `cargo metadata` (with the resolved graph, all features, all targets).
fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--all-features"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    layers::report(&layers::from_metadata(&metadata()?)?, true)
}
