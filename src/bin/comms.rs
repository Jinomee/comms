//! `comms` — the user-facing entry point for this fork.
//!
//! The engine is the `hcom` binary built from this same crate. Agents keep
//! talking to it as `hcom` (hooks, bootstrap prompts, and permission
//! allowlists all use that name), so `comms` stays a thin pass-through:
//! it locates `hcom` next to itself (falling back to PATH) and hands over
//! argv unchanged.
//!
//! The one command handled here is `comms init`, which creates the
//! per-project data dir (`<project>/.comms/hcom`) that the engine picks up
//! from anywhere inside the project.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const ENGINE: &str = if cfg!(windows) { "hcom.exe" } else { "hcom" };
const PROJECT_DIR: &str = ".comms";
const DATA_SUBDIR: &str = "hcom";

fn engine_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| exe.parent().map(|dir| dir.join(ENGINE)))
        .filter(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(ENGINE))
}

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

/// Create `<root>/.comms/hcom` and gitignore `.comms/`. Returns the data dir.
fn init(root: &Path) -> std::io::Result<(PathBuf, bool)> {
    let data = root.join(PROJECT_DIR).join(DATA_SUBDIR);
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

fn run_init() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("comms: cannot read current directory: {err}");
            return ExitCode::FAILURE;
        }
    };
    let root = project_root(&cwd);
    match init(&root) {
        Ok((data, ignored)) => {
            println!("comms is set up for {}", root.display());
            println!("  data: {}", data.display());
            if ignored {
                println!("  added {PROJECT_DIR}/ to .gitignore");
            }
            println!("Agents launched from anywhere in this project now share it.");
            if std::env::var_os("HCOM_DIR").is_some() {
                println!(
                    "Note: HCOM_DIR is set in this shell and takes precedence; unset it to use the project dir."
                );
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("comms: init failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "init") {
        return run_init();
    }

    let engine = engine_path();
    let mut cmd = Command::new(&engine);
    cmd.args(&args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // exec keeps the PTY, signals, and exit status identical to calling hcom directly.
        let err = cmd.exec();
        eprintln!("comms: failed to run {}: {err}", engine.display());
        ExitCode::from(127)
    }

    #[cfg(not(unix))]
    match cmd.status() {
        Ok(status) => match status.code() {
            Some(0) => ExitCode::SUCCESS,
            // Windows exit codes are 32-bit; don't let e.g. 256 wrap to 0 (success).
            Some(code) => ExitCode::from(u8::try_from(code).ok().filter(|c| *c != 0).unwrap_or(1)),
            None => ExitCode::from(1),
        },
        Err(err) => {
            eprintln!("comms: failed to run {}: {err}", engine.display());
            ExitCode::from(127)
        }
    }
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
        let (data, ignored) = init(tmp.path()).unwrap();
        assert!(data.ends_with(".comms/hcom"));
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
