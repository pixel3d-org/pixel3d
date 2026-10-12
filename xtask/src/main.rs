//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust. External tools (`cargo`, `git`) are invoked through
//! `std::process::Command`.

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  help  print this help
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
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
