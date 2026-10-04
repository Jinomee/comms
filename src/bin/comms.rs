//! `comms` — the user-facing entry point for this fork.
//!
//! The engine is the `hcom` binary built from this same crate. Agents keep
//! talking to it as `hcom` (hooks, bootstrap prompts, and permission
//! allowlists all use that name), so `comms` stays a thin pass-through:
//! it locates `hcom` next to itself (falling back to PATH) and hands over
//! argv unchanged.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

const ENGINE: &str = if cfg!(windows) { "hcom.exe" } else { "hcom" };

fn engine_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok())
        .and_then(|exe| exe.parent().map(|dir| dir.join(ENGINE)))
        .filter(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from(ENGINE))
}

fn main() -> ExitCode {
    let engine = engine_path();
    let mut cmd = Command::new(&engine);
    cmd.args(std::env::args_os().skip(1));

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
        // Windows exit codes are 32-bit; don't let e.g. 256 wrap to 0 (success).
        Ok(status) => match status.code() {
            Some(0) => ExitCode::SUCCESS,
            Some(code) => ExitCode::from(u8::try_from(code).ok().filter(|c| *c != 0).unwrap_or(1)),
            None => ExitCode::from(1),
        },
        Err(err) => {
            eprintln!("comms: failed to run {}: {err}", engine.display());
            ExitCode::from(127)
        }
    }
}
