//! pixel3d headless command-line interface.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use p3d_engine::Session;
use p3d_mcp::Agent;
use p3d_ops::catalog::catalog_json;
use p3d_ops::{FileAccess, Registry};
use serde_json::{Value, json};

const USAGE: &str = "\
usage: pixel3d-cli <command> [options]

commands:
  mcp [--open <scene.p3d>] [--allow-dir <folder>]... | --connect <[host:]port>
      serve the agent tools over MCP on stdin and stdout; file tools (import)
      may use the current folder, the scene's folder and each --allow-dir.
      --connect drives a running desktop app instead (started with --control
      <port>, or File > Allow agents to connect, port 7879) so you watch the
      agent work; file tools then use the folders the app allows
  run <script.json> [--open <in.p3d>] [--save <out.p3d>] [--timings] [--allow-dir <folder>]...
      run a list of ops as one transaction and print the result; the script
      is {\"ops\": [{\"op\": \"prim.box\", \"args\": {...}}, ...]} or just the list
  convert <scene.p3d> --to json [--out <file.json>]
      write a scene as readable JSON (to stdout without --out)
  catalog
      print every op (metadata, parameter schema, defaults, examples) as JSON
  -V, --version  print the version
  -h, --help     print this help

Logs go to stderr; PIXEL3D_LOG sets the level (error, warn, info, debug, trace).
";

/// Command-line flags after the command and its positional argument.
struct Flags {
    positional: Option<String>,
    options: Vec<(String, Option<String>)>,
}

impl Flags {
    /// Parses `[positional] [--name value | --switch]...`; `switches` take no value.
    fn parse(args: &[String], switches: &[&str]) -> Result<Flags> {
        let mut f = Flags { positional: None, options: Vec::new() };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if switches.contains(&name) {
                    f.options.push((name.to_owned(), None));
                } else {
                    let v = it.next().with_context(|| format!("--{name} needs a value"))?;
                    f.options.push((name.to_owned(), Some(v.clone())));
                }
            } else if f.positional.is_none() {
                f.positional = Some(a.clone());
            } else {
                bail!("unexpected argument `{a}`");
            }
        }
        Ok(f)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.options.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.as_deref())
    }

    /// Every value of an option that may repeat.
    fn values(&self, name: &str) -> Vec<&str> {
        self.options.iter().filter(|(n, _)| n == name).filter_map(|(_, v)| v.as_deref()).collect()
    }

    fn has(&self, name: &str) -> bool {
        self.options.iter().any(|(n, _)| n == name)
    }

    /// Fails on options the command does not know.
    fn only(&self, known: &[&str]) -> Result<()> {
        if let Some((n, _)) = self.options.iter().find(|(n, _)| !known.contains(&n.as_str())) {
            bail!("unknown option --{n} (known: {})", known.iter().map(|k| format!("--{k}")).collect::<Vec<_>>().join(", "));
        }
        Ok(())
    }
}

fn init_logging() {
    let level = std::env::var("PIXEL3D_LOG").ok().and_then(|l| l.parse::<tracing::Level>().ok()).unwrap_or(tracing::Level::WARN);
    // stdout carries the MCP protocol: logs must go to stderr.
    let _ = tracing_subscriber::fmt().with_writer(std::io::stderr).with_max_level(level).with_target(false).try_init();
}

/// A session, optionally with a document loaded from a file. File ops may
/// use the current folder, the opened scene's folder and `--allow-dir` folders.
fn session(f: &Flags) -> Result<Session> {
    let mut s = Session::new();
    let mut roots = vec![std::env::current_dir().context("cannot read the current folder")?];
    if let Some(path) = f.value("open") {
        let doc = p3d_io::p3d::load(Path::new(path)).with_context(|| format!("cannot open {path}"))?;
        s.load(doc);
        roots.extend(Path::new(path).parent().filter(|p| !p.as_os_str().is_empty()).map(Path::to_path_buf));
    }
    roots.extend(f.values("allow-dir").into_iter().map(PathBuf::from));
    s.set_file_access(FileAccess::new(roots).context("an --allow-dir folder does not exist")?);
    Ok(s)
}

fn mcp(args: &[String]) -> Result<ExitCode> {
    let f = Flags::parse(args, &[])?;
    f.only(&["open", "connect", "allow-dir"])?;
    if let Some(p) = &f.positional {
        bail!("mcp takes no positional argument (got `{p}`); use --open <scene.p3d>");
    }
    if let Some(to) = f.value("connect") {
        if f.has("open") {
            bail!("--open and --connect do not go together: the app has its scene open (open files there)");
        }
        if f.has("allow-dir") {
            bail!("--allow-dir and --connect do not go together: the app decides which folders agents may use");
        }
        let addr = p3d_mcp::connect::parse_addr(to).map_err(anyhow::Error::msg)?;
        p3d_mcp::serve_connected_stdio(addr)?;
    } else {
        p3d_mcp::serve_stdio(session(&f)?)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn run(args: &[String]) -> Result<ExitCode> {
    let f = Flags::parse(args, &["timings"])?;
    f.only(&["open", "save", "timings", "allow-dir"])?;
    let script = f.positional.as_deref().context("run needs a script file")?;
    let text = std::fs::read_to_string(script).with_context(|| format!("cannot read {script}"))?;
    let value: Value = serde_json::from_str(&text).with_context(|| format!("{script} is not JSON"))?;
    let call = if value.is_array() { json!({ "ops": value }) } else { value };
    let mut agent = Agent::new(session(&f)?);
    let out = agent.call("run", call);
    print!("{}", out.text);
    if out.is_error {
        return Ok(ExitCode::FAILURE);
    }
    if f.has("timings") {
        for op in out.structured["ops"].as_array().into_iter().flatten() {
            println!("timing {} {} ms", op["label"].as_str().unwrap_or("?"), op["ms"]);
        }
    }
    if let Some(path) = f.value("save") {
        let report = p3d_io::p3d::save(agent.session().doc(), &PathBuf::from(path)).with_context(|| format!("cannot save {path}"))?;
        println!("saved {path} ({} bytes)", report.bytes);
    }
    Ok(ExitCode::SUCCESS)
}

fn convert(args: &[String]) -> Result<ExitCode> {
    let f = Flags::parse(args, &[])?;
    f.only(&["to", "out"])?;
    let input = f.positional.as_deref().context("convert needs a .p3d file")?;
    match f.value("to") {
        Some("json") => {}
        Some(other) => bail!("cannot convert to `{other}` (supported: json)"),
        None => bail!("convert needs --to json"),
    }
    let doc = p3d_io::p3d::load(Path::new(input)).with_context(|| format!("cannot open {input}"))?;
    let json = serde_json::to_string_pretty(&p3d_io::p3d::to_json(&doc)?)?;
    match f.value("out") {
        Some(out) => std::fs::write(out, json + "\n").with_context(|| format!("cannot write {out}"))?,
        None => println!("{json}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    init_logging();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or(&[]);
    let result = match args.first().map(String::as_str) {
        Some("mcp") => mcp(rest),
        Some("run") => run(rest),
        Some("convert") => convert(rest),
        Some("catalog") => serde_json::to_string_pretty(&catalog_json(&Registry::builtin())).map_err(Into::into).map(|s| {
            println!("{s}");
            ExitCode::SUCCESS
        }),
        Some("-V" | "--version") => {
            println!("pixel3d-cli {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => Err(anyhow::anyhow!("unknown command `{other}`\n\n{USAGE}")),
    };
    result.unwrap_or_else(|e| {
        eprintln!("error: {e:#}");
        ExitCode::FAILURE
    })
}
