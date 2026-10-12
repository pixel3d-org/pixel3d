//! `cargo xtask bench-scale`: complexity scaling check.
//!
//! Every crate with a `benches/scale.rs` registers criterion groups named
//! `scale_<kernel>`, each running one kernel at n, 4n and 16n elements. This
//! command runs those benches, reads criterion's estimates and fits the
//! log-log slope k of time over n. Kernels in the check are declared O(n) or
//! O(n log n); k above [`MAX_SLOPE`] fails (n log n measures about 1.05 to
//! 1.15 over this range).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::{cargo, metadata, root};

/// Largest accepted log-log slope.
pub const MAX_SLOPE: f64 = 1.25;

/// Least-squares slope of ln(time) over ln(n). `None` with fewer than two
/// distinct sizes or non-positive values.
///
/// Complexity: O(points).
pub fn slope(points: &[(f64, f64)]) -> Option<f64> {
    if points.len() < 2 || points.iter().any(|&(n, t)| n <= 0.0 || t <= 0.0) {
        return None;
    }
    let logs: Vec<(f64, f64)> = points.iter().map(|&(n, t)| (n.ln(), t.ln())).collect();
    let len = logs.len() as f64;
    let (mx, my) = logs.iter().fold((0.0, 0.0), |(a, b), &(x, y)| (a + x / len, b + y / len));
    let (cov, var) = logs.iter().fold((0.0, 0.0), |(c, v), &(x, y)| (c + (x - mx) * (y - my), v + (x - mx) * (x - mx)));
    (var > 0.0).then(|| cov / var)
}

/// Median time in nanoseconds from a criterion `estimates.json`.
pub fn median_ns(estimates: &Value) -> Option<f64> {
    estimates["median"]["point_estimate"].as_f64()
}

/// One kernel's measurements: (n, nanoseconds), sorted by n.
#[derive(Debug, Clone, PartialEq)]
pub struct Kernel {
    pub name: String,
    pub points: Vec<(f64, f64)>,
}

/// Reads every `scale_*` group under criterion's output directory.
///
/// Complexity: O(groups x sizes) file reads.
fn collect(criterion_dir: &Path, filter: Option<&str>) -> Result<Vec<Kernel>, String> {
    let mut kernels = Vec::new();
    let Ok(entries) = std::fs::read_dir(criterion_dir) else {
        return Ok(kernels);
    };
    let mut groups: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    groups.sort();
    for group in groups {
        let Some(name) = group.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("scale_")) else { continue };
        if filter.is_some_and(|f| !name.contains(f)) {
            continue;
        }
        let mut points = Vec::new();
        for size in std::fs::read_dir(&group).map_err(|e| format!("{}: {e}", group.display()))?.filter_map(Result::ok) {
            let Some(n) = size.file_name().to_str().and_then(|s| s.parse::<f64>().ok()) else { continue };
            let file = size.path().join("new").join("estimates.json");
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let json: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            let t = median_ns(&json).ok_or_else(|| format!("{}: no median estimate", file.display()))?;
            points.push((n, t));
        }
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        kernels.push(Kernel { name: name.to_owned(), points });
    }
    Ok(kernels)
}

/// Crates that have a `benches/scale.rs`, as package names.
fn scale_crates(root: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(root.join("crates"))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.path().join("benches").join("scale.rs").is_file())
        .filter_map(|e| e.file_name().to_str().map(|n| format!("p3d-{n}")))
        .collect();
    out.sort();
    out
}

pub fn run(args: &[&str]) -> Result<(), String> {
    let large = args.contains(&"--large");
    let filter = args.iter().position(|a| *a == "--kernel").and_then(|i| args.get(i + 1)).copied();
    let meta = metadata()?;
    let target = meta["target_directory"].as_str().map(PathBuf::from).unwrap_or_else(|| root().join("target"));
    let criterion_dir = target.join("criterion");

    // Old results (for example from a large run) must not mix in.
    if let Ok(entries) = std::fs::read_dir(&criterion_dir) {
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("scale_") && filter.is_none_or(|f| name.contains(f)) {
                std::fs::remove_dir_all(e.path()).map_err(|err| format!("{name}: {err}"))?;
            }
        }
    }

    for pkg in scale_crates(&root()) {
        let mut c = cargo();
        c.args(["bench", "-p", &pkg, "--bench", "scale", "--"]);
        if let Some(f) = filter {
            c.arg(f);
        }
        c.args(["--warm-up-time", "0.5", "--measurement-time", "2", "--sample-size", "20"]);
        if large {
            c.env("P3D_BENCH_SCALE", "large");
        }
        eprintln!("$ cargo bench -p {pkg} --bench scale{}", if large { " (large sizes)" } else { "" });
        let status = c.status().map_err(|e| format!("cargo bench: {e}"))?;
        if !status.success() {
            return Err(format!("cargo bench -p {pkg} --bench scale failed with {status}"));
        }
    }

    let kernels = collect(&criterion_dir, filter)?;
    if kernels.is_empty() {
        return Err("no scale_* benchmark results found".into());
    }
    println!("\n{:<32} {:>12} {:>12} {:>12} {:>7}", "kernel", "n", "4n", "16n", "slope");
    let mut failures = Vec::new();
    for k in &kernels {
        let times: Vec<String> = k.points.iter().map(|&(n, t)| format!("{}:{}", n, format_ns(t))).collect();
        let s = slope(&k.points);
        let cell = |i: usize| times.get(i).cloned().unwrap_or_else(|| "-".into());
        let shown = s.map_or_else(|| "-".to_owned(), |s| format!("{s:.2}"));
        println!("{:<32} {:>12} {:>12} {:>12} {:>7}", k.name, cell(0), cell(1), cell(2), shown);
        match s {
            Some(s) if s <= MAX_SLOPE => {}
            Some(s) => failures.push(format!("{}: slope {s:.2} > {MAX_SLOPE}", k.name)),
            None => failures.push(format!("{}: needs at least two sizes", k.name)),
        }
    }
    if failures.is_empty() {
        println!("\nbench-scale: OK, {} kernels scale at most n^{MAX_SLOPE}.", kernels.len());
        Ok(())
    } else {
        Err(format!("scaling check failed:\n  {}", failures.join("\n  ")))
    }
}

fn format_ns(t: f64) -> String {
    match t {
        t if t >= 1e9 => format!("{:.2}s", t / 1e9),
        t if t >= 1e6 => format!("{:.2}ms", t / 1e6),
        t if t >= 1e3 => format!("{:.1}us", t / 1e3),
        t => format!("{t:.0}ns"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slope_of_power_laws() {
        let pts = |k: f64| [1e3, 4e3, 16e3].map(|n: f64| (n, 5.0 * n.powf(k)));
        assert!((slope(&pts(1.0)).unwrap() - 1.0).abs() < 1e-9);
        assert!((slope(&pts(2.0)).unwrap() - 2.0).abs() < 1e-9);
        let nlogn = [1e3, 4e3, 16e3].map(|n: f64| (n, n * n.ln()));
        let k = slope(&nlogn).unwrap();
        assert!(k > 1.05 && k < 1.2, "{k}");
        assert!(slope(&[(1e3, 1.0)]).is_none());
        assert!(slope(&[(1e3, 1.0), (1e3, 2.0)]).is_none());
        assert!(slope(&[(1e3, 0.0), (4e3, 2.0)]).is_none());
    }

    #[test]
    fn reads_criterion_layout() {
        let dir = std::env::temp_dir().join(format!("p3d-bench-scale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (n, t) in [(1000, 10.0), (4000, 40.0), (16000, 160.0)] {
            let d = dir.join("scale_demo").join(n.to_string()).join("new");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("estimates.json"), format!(r#"{{"median": {{"point_estimate": {t}}}}}"#)).unwrap();
        }
        std::fs::create_dir_all(dir.join("other_group")).unwrap();
        let ks = collect(&dir, None).unwrap();
        assert_eq!(ks.len(), 1);
        assert_eq!(ks[0].name, "demo");
        assert_eq!(ks[0].points, vec![(1000.0, 10.0), (4000.0, 40.0), (16000.0, 160.0)]);
        assert!((slope(&ks[0].points).unwrap() - 1.0).abs() < 1e-9);
        assert!(collect(&dir, Some("nothing")).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
