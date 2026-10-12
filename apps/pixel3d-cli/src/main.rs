//! pixel3d headless command-line interface.

use std::process::ExitCode;

use p3d_ops::Registry;
use p3d_ops::catalog::catalog_json;

const USAGE: &str = "\
usage: pixel3d-cli <command>

commands:
  catalog        print every op (metadata, parameter schema, defaults, examples) as JSON
  -V, --version  print the version
  -h, --help     print this help
";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("catalog") => match serde_json::to_string_pretty(&catalog_json(&Registry::builtin())) {
            Ok(s) => {
                println!("{s}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
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
