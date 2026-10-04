//! Init command: `comms init`
//!
//! Creates the per-project data dir (`<git root>/.comms/data`) and gitignores
//! `.comms/`. With COMMS_DIR unset, every comms call inside the project (the
//! human's and the agents') then resolves to it; see
//! `paths::find_project_comms_dir`.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::router::GlobalFlags;

const PROJECT_DIR: &str = ".comms";

/// Git root containing `start`, else `start` itself.
fn project_root(start: &Path) -> PathBuf {
    start
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(start)
        .to_path_buf()
}

/// Whether `.gitignore` content already ignores the project dir.
fn already_ignored(gitignore: &str) -> bool {
    gitignore.lines().map(str::trim).any(|line| {
        matches!(
            line,
            ".comms" | ".comms/" | "/.comms" | "/.comms/" | ".comms/*" | "/.comms/*"
        )
    })
}

/// Create the data dir under `root` and gitignore `.comms/`.
/// Returns the data dir and whether `.gitignore` was changed.
fn init(root: &Path) -> std::io::Result<(PathBuf, bool)> {
    let data = crate::paths::PROJECT_DATA_DIR
        .iter()
        .fold(root.to_path_buf(), |p, c| p.join(c));
    std::fs::create_dir_all(&data)?;
    let mut ignored = false;
    if root.join(".git").exists() {
        let path = root.join(".gitignore");
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !already_ignored(&existing) {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            if !existing.is_empty() && !existing.ends_with('\n') {
                writeln!(file)?;
            }
            writeln!(file, "{PROJECT_DIR}/")?;
            ignored = true;
        }
    }
    Ok((data, ignored))
}

pub fn run(_argv: &[String], _flags: &GlobalFlags) -> Result<i32> {
    let cwd = std::env::current_dir()?;
    let root = project_root(&cwd);
    let (data, ignored) = init(&root)?;
    println!("comms is set up for {}", root.display());
    println!("  data: {}", data.display());
    if ignored {
        println!("  added {PROJECT_DIR}/ to .gitignore");
    }
    println!("Agents launched from anywhere in this project now share it.");
    if std::env::var_os("COMMS_DIR").is_some() {
        println!(
            "Note: COMMS_DIR is set in this shell and takes precedence; unset it to use the project dir."
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_data_dir_and_gitignores_once() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target").unwrap();
        let sub = root.join("src");
        std::fs::create_dir(&sub).unwrap();

        assert_eq!(project_root(&sub), root);
        let (data, ignored) = init(root).unwrap();
        assert!(data.is_dir());
        assert_eq!(crate::paths::find_project_comms_dir(&sub), Some(data));
        assert!(ignored);
        assert_eq!(
            std::fs::read_to_string(root.join(".gitignore")).unwrap(),
            "target\n.comms/\n"
        );

        let (_, ignored) = init(root).unwrap();
        assert!(!ignored);
    }

    #[test]
    fn init_outside_git_skips_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, ignored) = init(tmp.path()).unwrap();
        assert!(!ignored);
        assert!(!tmp.path().join(".gitignore").exists());
    }

    #[test]
    fn ignore_detection() {
        assert!(already_ignored("x\n.comms/\n"));
        assert!(already_ignored("/.comms"));
        assert!(!already_ignored(".commsx\n# .comms"));
    }
}
