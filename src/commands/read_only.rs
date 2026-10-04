//! `--read-only` launch flag: start a live agent that can read, run hcom,
//! and talk, but not edit files.
//!
//! Kept out of the shared launch-flag parser on purpose (it is a fork-only
//! feature); `launch::parse_launch_argv` peels it off before that parser runs.
//!
//! - claude: edit tools are disallowed. Bash stays available because agents
//!   send messages with `hcom send` (shell edits are still possible; claims
//!   are the guard for that).
//! - codex: `--sandbox read-only`. hcom's own dir stays writable via
//!   `codex_preprocessing::ensure_hcom_writable`.

use anyhow::{Result, bail};

use crate::tool::Tool;

pub const FLAG: &str = "--read-only";

const CLAUDE_EDIT_TOOLS: &str = "Edit,Write,MultiEdit,NotebookEdit";

/// Remove `--read-only` from the hcom-flag section (before any `--`).
/// Returns whether it was present and the remaining args.
pub fn take_flag(args: &[String]) -> (bool, Vec<String>) {
    let mut found = false;
    let mut rest = Vec::with_capacity(args.len());
    let mut after_separator = false;
    for arg in args {
        if !after_separator && arg == "--" {
            after_separator = true;
        }
        if !after_separator && arg == FLAG {
            found = true;
            continue;
        }
        rest.push(arg.clone());
    }
    (found, rest)
}

/// Prepend the tool's read-only args. Errors for tools without support or
/// when the user's own args ask for write access.
pub fn apply(tool_name: &str, tool_args: Vec<String>) -> Result<Vec<String>> {
    let tool = tool_name.parse::<Tool>().ok();
    let flag_section = || tool_args.iter().take_while(|a| *a != "--");

    let mut prefix: Vec<String> = match tool {
        Some(Tool::Claude) => {
            if flag_section().any(|a| {
                a == "--permission-mode"
                    || a.starts_with("--permission-mode=")
                    || a == "--dangerously-skip-permissions"
            }) {
                bail!("{FLAG} can't be combined with a claude permission-mode override");
            }
            vec!["--disallowedTools".into(), CLAUDE_EDIT_TOOLS.into()]
        }
        Some(Tool::Codex) => {
            if flag_section().any(|a| {
                matches!(
                    a.as_str(),
                    "-s" | "--sandbox"
                        | "--full-auto"
                        | "--dangerously-bypass-approvals-and-sandbox"
                ) || a.starts_with("--sandbox=")
            }) {
                bail!("{FLAG} can't be combined with codex sandbox flags");
            }
            vec!["--sandbox".into(), "read-only".into()]
        }
        _ => bail!("{FLAG} is supported for claude and codex, not '{tool_name}'"),
    };

    prefix.extend(tool_args);
    Ok(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn take_flag_only_before_separator() {
        let (found, rest) = take_flag(&s(&["--tag", "x", "--read-only", "--", "--read-only"]));
        assert!(found);
        assert_eq!(rest, s(&["--tag", "x", "--", "--read-only"]));

        let (found, rest) = take_flag(&s(&["--model", "opus"]));
        assert!(!found);
        assert_eq!(rest, s(&["--model", "opus"]));
    }

    #[test]
    fn claude_disallows_edit_tools() {
        let args = apply("claude", s(&["--model", "opus"])).unwrap();
        assert_eq!(
            args,
            s(&["--disallowedTools", CLAUDE_EDIT_TOOLS, "--model", "opus"])
        );
    }

    #[test]
    fn codex_uses_read_only_sandbox() {
        let args = apply("codex", s(&[])).unwrap();
        assert_eq!(args, s(&["--sandbox", "read-only"]));
    }

    #[test]
    fn conflicting_write_flags_are_rejected() {
        assert!(apply("codex", s(&["--full-auto"])).is_err());
        assert!(apply("codex", s(&["--sandbox=workspace-write"])).is_err());
        assert!(apply("claude", s(&["--dangerously-skip-permissions"])).is_err());
        // Text after `--` is a prompt, not a flag.
        assert!(apply("codex", s(&["--", "--full-auto"])).is_ok());
    }

    #[test]
    fn unsupported_tool_is_rejected() {
        assert!(apply("gemini", s(&[])).is_err());
    }
}
