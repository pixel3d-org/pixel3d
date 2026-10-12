//! pixel3d desktop application.

use std::process::ExitCode;

const USAGE: &str = "\
usage: pixel3d [options]

options:
  -V, --version  print the version
  -h, --help     print this help
";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("-V" | "--version") => {
            println!("pixel3d {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("-h" | "--help") | None => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("error: unknown option `{other}`\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
