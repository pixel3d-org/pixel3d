//! `cargo xtask ci` (everything CI runs) and `cargo xtask ci-fast` (the
//! pre-commit subset). Both stop at the first failing step.

use std::path::Path;
use std::process::Command;

use crate::changed::{self, Selection};
use crate::{cargo, emdash, layers, metadata, root};

type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);

/// Runs a command, echoing it first.
fn run(mut cmd: Command) -> Result<(), String> {
    let program = Path::new(cmd.get_program()).file_stem().unwrap_or(cmd.get_program()).to_string_lossy().into_owned();
    let what = format!("{program} {}", cmd.get_args().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" "));
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

fn cargo_step(args: &'static [&'static str]) -> Box<dyn Fn() -> Result<(), String>> {
    Box::new(move || {
        let mut c = cargo();
        c.args(args);
        run(c)
    })
}

fn layers_step() -> Result<(), String> {
    layers::report(&layers::from_metadata(&metadata()?)?, false)
}

fn emdash_step() -> Result<(), String> {
    emdash::run(&root())
}

fn deny_step() -> Result<(), String> {
    let probe = cargo().args(["deny", "--version"]).output();
    if !probe.is_ok_and(|o| o.status.success()) {
        return Err("cargo-deny is not installed; run `cargo install cargo-deny --locked`".into());
    }
    let mut c = cargo();
    c.args(["deny", "check"]);
    run(c)
}

/// Tests only the crates affected by the working-tree changes.
fn changed_tests_step() -> Result<(), String> {
    let out = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .current_dir(root())
        .output()
        .map_err(|e| format!("git status: {e}"))?;
    if !out.status.success() {
        return Err(format!("git status failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let paths = changed::parse_porcelain(&out.stdout);
    let mut c = cargo();
    c.arg("test");
    match changed::affected(&paths, &changed::members(&metadata()?)?) {
        Selection::All => {
            c.arg("--workspace");
        }
        Selection::Some(names) if names.is_empty() => {
            eprintln!("no crate changed; no tests to run");
            return Ok(());
        }
        Selection::Some(names) => {
            for n in names {
                c.args(["-p", &n]);
            }
        }
    }
    run(c)
}

fn run_steps(name: &str, steps: Vec<Step>) -> Result<(), String> {
    let mut done = Vec::new();
    for (step, f) in &steps {
        eprintln!("\n=== {name}: {step} ===");
        if let Err(e) = f() {
            println!("\n{name} summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {step}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("{name} failed at `{step}`"));
        }
        done.push(*step);
    }
    println!("\n{name} summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}

/// Everything CI runs, in the same order (small bench-scale sizes).
pub fn ci() -> Result<(), String> {
    run_steps(
        "ci",
        vec![
            ("fmt", cargo_step(&["fmt", "--all", "--", "--check"])),
            ("clippy", cargo_step(&["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"])),
            ("test", cargo_step(&["test", "--workspace", "--locked"])),
            ("layers", Box::new(layers_step)),
            ("emdash", Box::new(emdash_step)),
            ("deny", Box::new(deny_step)),
            ("release build", cargo_step(&["build", "--workspace", "--release", "--locked"])),
            ("bench-scale", Box::new(|| crate::bench_scale::run(&[]))),
        ],
    )
}

/// The pre-commit subset: fmt, clippy, tests of changed crates, layers, emdash.
pub fn ci_fast() -> Result<(), String> {
    run_steps(
        "ci-fast",
        vec![
            ("fmt", cargo_step(&["fmt", "--all", "--", "--check"])),
            ("clippy", cargo_step(&["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"])),
            ("test (changed crates)", Box::new(changed_tests_step)),
            ("layers", Box::new(layers_step)),
            ("emdash", Box::new(emdash_step)),
        ],
    )
}
