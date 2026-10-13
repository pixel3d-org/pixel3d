//! Where file ops may read and write (project spec 3.8 and 17.19): paths
//! are canonicalized (so `..` and symbolic links are resolved) and must lie
//! inside one of the allowed roots.

use std::path::{Path, PathBuf};

/// Why a path is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SandboxError {
    #[error("file access is off: no folders are allowed")]
    NoRoots,
    #[error("`{0}` does not exist")]
    NotFound(String),
    #[error("`{path}` is outside the allowed folders ({roots})")]
    Outside { path: String, roots: String },
    #[error("`{path}`: {message}")]
    Io { path: String, message: String },
}

/// The allowed roots, canonical.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileAccess {
    roots: Vec<PathBuf>,
}

impl FileAccess {
    /// No access at all.
    pub fn none() -> Self {
        FileAccess::default()
    }

    /// Access inside these folders. Fails if one does not exist.
    ///
    /// Complexity: O(roots) file system calls.
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Result<Self, SandboxError> {
        let mut out = Vec::new();
        for r in roots {
            let c = r.canonicalize().map_err(|e| io(&r, &e))?;
            if !out.contains(&c) {
                out.push(c);
            }
        }
        Ok(FileAccess { roots: out })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// `path` resolved: relative paths start at the first root.
    fn resolve(&self, path: &str) -> Result<PathBuf, SandboxError> {
        let first = self.roots.first().ok_or(SandboxError::NoRoots)?;
        let p = Path::new(path);
        Ok(if p.is_absolute() { p.to_path_buf() } else { first.join(p) })
    }

    fn inside(&self, canonical: &Path, shown: &str) -> Result<(), SandboxError> {
        if self.roots.iter().any(|r| canonical.starts_with(r)) {
            Ok(())
        } else {
            Err(SandboxError::Outside {
                path: shown.to_owned(),
                roots: self.roots.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", "),
            })
        }
    }

    /// An existing file inside a root, canonical.
    ///
    /// Complexity: O(roots + path length).
    pub fn readable(&self, path: &str) -> Result<PathBuf, SandboxError> {
        let p = self.resolve(path)?;
        let c = match p.canonicalize() {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SandboxError::NotFound(path.to_owned())),
            Err(e) => return Err(io(&p, &e)),
        };
        self.inside(&c, path)?;
        Ok(c)
    }

    /// A file to create or replace inside a root: its folder must exist and
    /// lie inside a root, and an existing file there must not be a link out.
    ///
    /// Complexity: O(roots + path length).
    pub fn writable(&self, path: &str) -> Result<PathBuf, SandboxError> {
        let p = self.resolve(path)?;
        let name = p.file_name().ok_or_else(|| SandboxError::Io { path: path.to_owned(), message: "not a file name".into() })?;
        let dir = p.parent().unwrap_or(Path::new("."));
        let dir = match dir.canonicalize() {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(SandboxError::NotFound(dir.display().to_string())),
            Err(e) => return Err(io(dir, &e)),
        };
        self.inside(&dir, path)?;
        let target = dir.join(name);
        if let Ok(c) = target.canonicalize() {
            self.inside(&c, path)?;
        }
        Ok(target)
    }

    /// A file next to `base` (a canonical path from [`FileAccess::readable`]),
    /// such as a glTF buffer, checked like `readable`.
    ///
    /// Complexity: O(roots + path length).
    pub fn sibling(&self, base: &Path, relative: &str) -> Result<PathBuf, SandboxError> {
        let joined = base.parent().unwrap_or(Path::new("")).join(relative);
        self.readable(&joined.display().to_string())
    }
}

fn io(p: &Path, e: &std::io::Error) -> SandboxError {
    SandboxError::Io { path: p.display().to_string(), message: e.to_string() }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("p3d-sandbox-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("inside")).unwrap();
        std::fs::create_dir_all(d.join("outside")).unwrap();
        std::fs::write(d.join("inside/a.obj"), "v 0 0 0").unwrap();
        std::fs::write(d.join("outside/b.obj"), "v 0 0 0").unwrap();
        d
    }

    #[test]
    fn paths_stay_inside_the_roots() {
        let d = temp_dir("roots");
        let fa = FileAccess::new([d.join("inside")]).unwrap();
        assert!(fa.readable("a.obj").is_ok());
        assert!(fa.readable(&d.join("inside/a.obj").display().to_string()).is_ok());
        assert!(matches!(fa.readable("../outside/b.obj"), Err(SandboxError::Outside { .. })));
        assert!(matches!(fa.readable(&d.join("outside/b.obj").display().to_string()), Err(SandboxError::Outside { .. })));
        assert!(matches!(fa.readable("missing.obj"), Err(SandboxError::NotFound(_))));
        assert!(fa.writable("new.glb").is_ok());
        assert!(matches!(fa.writable("../outside/new.glb"), Err(SandboxError::Outside { .. })));
        assert!(matches!(fa.writable("nowhere/new.glb"), Err(SandboxError::NotFound(_))));
        let base = fa.readable("a.obj").unwrap();
        assert!(matches!(fa.sibling(&base, "../outside/b.obj"), Err(SandboxError::Outside { .. })));
        assert_eq!(FileAccess::none().readable("a.obj"), Err(SandboxError::NoRoots));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn links_out_of_a_root_are_refused() {
        let d = temp_dir("links");
        std::os::unix::fs::symlink(d.join("outside/b.obj"), d.join("inside/link.obj")).unwrap();
        std::os::unix::fs::symlink(d.join("outside"), d.join("inside/out")).unwrap();
        let fa = FileAccess::new([d.join("inside")]).unwrap();
        assert!(matches!(fa.readable("link.obj"), Err(SandboxError::Outside { .. })));
        assert!(matches!(fa.readable("out/b.obj"), Err(SandboxError::Outside { .. })));
        assert!(matches!(fa.writable("link.obj"), Err(SandboxError::Outside { .. })));
        assert!(matches!(fa.writable("out/new.glb"), Err(SandboxError::Outside { .. })));
        let _ = std::fs::remove_dir_all(&d);
    }
}
