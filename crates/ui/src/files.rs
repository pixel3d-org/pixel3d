//! Which folders file ops (`io.import`, later export) may use in the
//! desktop app (project spec 3.8): the folder of the open `.p3d` file, and
//! every folder the user took a file from during this run, through a file
//! dialog or by dropping it onto the window. The user chose those, so they
//! are allowed; nothing else is. Agents connected through the control
//! server act on the same session, so they get exactly these folders and
//! never more (the control server also keeps their `file.*` commands
//! inside them, `remote.rs`).

use std::path::{Path, PathBuf};

use p3d_ops::FileAccess;

/// The project folder and the folders the user picked files from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileRoots {
    /// The folder of the open `.p3d` file.
    project: Option<PathBuf>,
    /// Folders the user picked files from, oldest first, without repeats.
    picked: Vec<PathBuf>,
}

/// The folder a file is in, canonical when it exists.
fn folder_of(file: &Path) -> Option<PathBuf> {
    let dir = file.parent().filter(|p| !p.as_os_str().is_empty()).map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    dir.canonicalize().ok()
}

impl FileRoots {
    /// The folder of the open project, if it was opened from or saved to a file.
    pub fn project(&self) -> Option<&Path> {
        self.project.as_deref()
    }

    /// Folders the user picked files from in this run.
    pub fn picked(&self) -> &[PathBuf] {
        &self.picked
    }

    /// Makes `file`'s folder the project folder (`None` for a new, unsaved
    /// document). Returns whether the folders changed.
    pub fn set_project_file(&mut self, file: Option<&Path>) -> bool {
        let dir = file.and_then(folder_of);
        let changed = dir != self.project;
        self.project = dir;
        changed
    }

    /// Allows the folder of a file the user picked (a dialog or a drop).
    /// Returns whether it was new.
    ///
    /// Complexity: O(picked folders).
    pub fn allow_picked_file(&mut self, file: &Path) -> bool {
        folder_of(file).is_some_and(|dir| self.push(dir))
    }

    /// Allows a folder the user named (`pixel3d --allow-dir`). Returns
    /// whether it was new; a folder that does not exist is not added.
    ///
    /// Complexity: O(picked folders).
    pub fn allow_folder(&mut self, dir: &Path) -> bool {
        dir.is_dir() && dir.canonicalize().is_ok_and(|d| self.push(d))
    }

    fn push(&mut self, dir: PathBuf) -> bool {
        if self.picked.contains(&dir) {
            return false;
        }
        self.picked.push(dir);
        true
    }

    /// Every allowed folder: the project folder first (relative paths start
    /// there), then the picked ones. Folders that are gone are left out.
    ///
    /// Complexity: O(folders) file system calls.
    pub fn roots(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::with_capacity(self.picked.len() + 1);
        for dir in self.project.iter().chain(&self.picked) {
            if dir.is_dir() && !out.contains(dir) {
                out.push(dir.clone());
            }
        }
        out
    }

    /// The folders as the session's file access.
    ///
    /// Complexity: O(folders) file system calls.
    pub fn access(&self) -> FileAccess {
        let roots = self.roots();
        // A folder that cannot be read any more drops out instead of
        // closing all the others.
        FileAccess::new(roots.clone()).unwrap_or_else(|_| FileAccess::new(roots.into_iter().filter(|r| r.canonicalize().is_ok())).unwrap_or_default())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{PixelApp, Services};
    use serde_json::json;

    /// A fresh temporary folder for one test.
    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("p3d-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.canonicalize().unwrap()
    }

    /// A one-triangle OBJ file.
    pub(crate) fn write_obj(path: &Path) {
        std::fs::write(path, "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n").unwrap();
    }

    #[test]
    fn roots_are_the_project_folder_then_picked_ones() {
        let (a, b) = (temp_dir("roots-a"), temp_dir("roots-b"));
        let mut r = FileRoots::default();
        assert!(r.roots().is_empty() && r.access().roots().is_empty(), "nothing is allowed at first");
        assert!(r.allow_picked_file(&b.join("x.obj")));
        assert!(!r.allow_picked_file(&b.join("y.obj")), "a folder counts once");
        assert!(!r.allow_folder(&b), "named or picked, once");
        assert!(!r.allow_folder(&b.join("missing")), "a folder that does not exist is not added");
        assert!(r.set_project_file(Some(&a.join("scene.p3d"))));
        assert_eq!(r.roots(), vec![a.clone(), b.clone()], "the project folder comes first");
        assert!(r.set_project_file(None));
        assert_eq!(r.roots(), vec![b.clone()], "a new document has no project folder; picked ones stay");
        std::fs::remove_dir_all(&b).unwrap();
        assert!(r.roots().is_empty(), "a folder that is gone drops out");
        let _ = std::fs::remove_dir_all(a);
    }

    #[test]
    fn the_app_allows_the_project_folder_and_picked_folders_only() {
        let (project, picked, other) = (temp_dir("files-project"), temp_dir("files-picked"), temp_dir("files-other"));
        for d in [&project, &picked, &other] {
            write_obj(&d.join("tri.obj"));
        }
        let pick = picked.join("scene.p3d");
        let services = Services { pick_save: Some(Box::new(move |_: &str| Some(pick.clone()))), ..Services::default() };
        let mut app = PixelApp::new(p3d_engine::Session::new(), services);
        let import = |app: &mut PixelApp, dir: &Path| app.run("io.import", json!({ "path": dir.join("tri.obj").display().to_string() }));
        assert!(import(&mut app, &project).unwrap_err().contains("file access is off"), "no folder before a file is opened or picked");
        // Saving to a file makes its folder the project folder.
        app.run("file.save", json!({ "path": project.join("scene.p3d").display().to_string() })).unwrap();
        import(&mut app, &project).unwrap();
        assert!(import(&mut app, &other).unwrap_err().contains("outside the allowed folders"));
        // A folder picked in a dialog stays allowed for the rest of the run.
        app.run("file.save_as", serde_json::Value::Null).unwrap();
        assert_eq!(app.files.picked(), std::slice::from_ref(&picked));
        import(&mut app, &picked).unwrap();
        app.run("file.new", serde_json::Value::Null).unwrap();
        import(&mut app, &picked).unwrap();
        assert!(import(&mut app, &project).is_err(), "the old project folder went with its document");
        for d in [project, picked, other] {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}
