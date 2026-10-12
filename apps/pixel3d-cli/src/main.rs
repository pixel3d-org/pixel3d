//! pixel3d headless command-line interface.

use std::process::ExitCode;

const USAGE: &str = "\
usage: pixel3d-cli <command>

commands:
  -V, --version  print the version
  -h, --help     print this help
";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("-V" | "--version") => {
            println!("pixel3d-cli {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown command `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
