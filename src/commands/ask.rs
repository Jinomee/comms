//! Ask command: `hcom ask <claude|codex> <question>`
//!
//! One-shot consult: start the target agent headless in the current
//! directory, wait for its final answer, print it to stdout, exit. Read-only
//! unless `--write` is passed. Meant to be run as a background task by the
//! asking agent, so the answer arrives as that task's output.
//!
//! The child is a plain `claude -p` / `codex exec` process — not an hcom
//! instance — so it never registers, gets hooks, or receives messages.

use std::collections::HashSet;
use std::io::{IsTerminal, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::router::GlobalFlags;
use crate::tool::Tool;

/// Env var carrying how many asks deep the current process is.
pub const ASK_DEPTH_ENV: &str = "HCOM_ASK_DEPTH";
/// Nested asks allowed below a top-level ask (an asked agent may not ask again).
const MAX_ASK_DEPTH: u32 = 1;
/// Files up to this size are inlined into the prompt; larger ones are referenced by path.
const INLINE_FILE_LIMIT: u64 = 16 * 1024;

const CLAUDE_READ_ONLY_TOOLS: &[&str] = &[
    "Read",
    "Grep",
    "Glob",
    "LS",
    "Bash(git log:*)",
    "Bash(git diff:*)",
    "Bash(git show:*)",
    "Bash(git status:*)",
    "Bash(git blame:*)",
];
const CLAUDE_EDIT_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// Per-session vars a host Claude Code exports to its shell. Leaked into a
/// child `claude -p`, they make it reuse the caller's session id (sharing its
/// transcript) or talk to the caller's messaging socket. Auth/proxy vars are
/// deliberately not listed: the child needs them.
const CLAUDE_SESSION_SCOPED_ENV: &[&str] = &[
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_REMOTE_SESSION_ID",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_TEE_SDK_STDOUT",
    "CLAUDE_CODE_SYNC_SESSION_REFS",
    "CLAUDE_CODE_DIAGNOSTICS_FILE",
    "CLAUDE_AFTER_LAST_COMPACT",
    "CLAUDE_PID",
];

/// Parsed arguments for `hcom ask`.
#[derive(clap::Parser, Debug)]
#[command(
    name = "ask",
    about = "Ask another agent a one-shot question (read-only)"
)]
pub struct AskArgs {
    /// Agent to ask: claude or codex
    pub agent: String,
    /// The question (use "-" or omit to read it from stdin)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub question: Vec<String>,
    /// File to point the agent at (repeatable). Small files are inlined.
    #[arg(long = "file", short = 'f')]
    pub files: Vec<String>,
    /// Allow the asked agent to edit files (default: read-only)
    #[arg(long)]
    pub write: bool,
    /// Resume a previous ask's session (id printed after each answer)
    #[arg(long = "continue", value_name = "SESSION")]
    pub resume: Option<String>,
    /// Model to use
    #[arg(long)]
    pub model: Option<String>,
    /// Give up after this many seconds
    #[arg(long, default_value_t = 600)]
    pub timeout: u64,
    /// Print {"agent","answer","session"} as JSON instead of plain text
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AskAgent {
    Claude,
    Codex,
}

impl AskAgent {
    fn parse(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "claude" => Ok(Self::Claude),
            "codex" => Ok(Self::Codex),
            other => bail!("ask supports 'claude' or 'codex', not '{other}'"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn tool(self) -> Tool {
        match self {
            Self::Claude => Tool::Claude,
            Self::Codex => Tool::Codex,
        }
    }
}

/// A fully-specified child invocation.
#[derive(Debug, PartialEq, Eq)]
struct Invocation {
    program: String,
    args: Vec<String>,
}

pub fn run(argv: &[String], _flags: &GlobalFlags) -> Result<i32> {
    let mut filtered = vec!["ask".to_string()];
    filtered.extend(argv.iter().skip_while(|a| *a != "ask").skip(1).cloned());

    use clap::Parser;
    let args = match AskArgs::try_parse_from(&filtered) {
        Ok(a) => a,
        Err(e) => {
            e.print().ok();
            return Ok(if e.use_stderr() { 1 } else { 0 });
        }
    };

    match execute(&args) {
        Ok(code) => Ok(code),
        Err(e) => {
            eprintln!("Error: {e:#}");
            Ok(1)
        }
    }
}

fn execute(args: &AskArgs) -> Result<i32> {
    let agent = AskAgent::parse(&args.agent)?;

    let depth = current_depth();
    if depth > MAX_ASK_DEPTH {
        bail!("nested ask refused: this agent was itself started by an ask");
    }

    let question = read_question(&args.question)?;
    let cwd = std::env::current_dir().context("cannot read current directory")?;
    let prompt = build_prompt(agent, &asker_name(), &question, &args.files, args.write);

    let program = crate::terminal::which_bin(agent.tool().spec().name)
        .with_context(|| format!("'{}' not found on PATH", agent.name()))?;

    let output_file = tempfile::NamedTempFile::new()?;
    let invocation = build_invocation(agent, program, args, output_file.path());

    let mut cmd = Command::new(&invocation.program);
    cmd.args(&invocation.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for var in env_strip_set(agent) {
        cmd.env_remove(var);
    }
    cmd.env(ASK_DEPTH_ENV, (depth + 1).to_string());

    let run = run_with_timeout(cmd, &prompt, Duration::from_secs(args.timeout))?;

    let parsed = match agent {
        AskAgent::Claude => parse_claude_output(&run.stdout),
        AskAgent::Codex => parse_codex_output(&run.stdout, output_file.path()),
    };

    if run.timed_out {
        bail!("{} did not answer within {}s", agent.name(), args.timeout);
    }
    let answer = match parsed {
        Some(answer) if run.success && !answer.is_error => answer,
        other => {
            let detail = other
                .map(|a| a.text)
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| tail(&run.stderr, 20));
            bail!("{} failed: {}", agent.name(), detail.trim());
        }
    };

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "agent": agent.name(),
                "answer": answer.text,
                "session": answer.session,
            })
        );
    } else {
        println!("{}", answer.text.trim_end());
        if let Some(session) = &answer.session {
            eprintln!(
                "\n[follow up: hcom ask {} --continue {} \"...\"]",
                agent.name(),
                session
            );
        }
    }
    Ok(0)
}

fn current_depth() -> u32 {
    std::env::var(ASK_DEPTH_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn read_question(words: &[String]) -> Result<String> {
    let joined = words.join(" ");
    let from_stdin = joined.trim().is_empty() || joined.trim() == "-";
    if !from_stdin {
        return Ok(joined);
    }
    if std::io::stdin().is_terminal() {
        bail!("no question given (pass it as an argument or pipe it on stdin)");
    }
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        bail!("no question given (stdin was empty)");
    }
    Ok(buf)
}

/// Who is asking, for the prompt preamble.
fn asker_name() -> String {
    if let Ok(name) = std::env::var("HCOM_INSTANCE_NAME")
        && !name.is_empty()
    {
        return name;
    }
    let ctx = crate::shared::context::HcomContext::from_os();
    if ctx.is_inside_ai_tool() {
        format!("another {} agent", ctx.tool)
    } else {
        "a human".to_string()
    }
}

fn build_prompt(
    agent: AskAgent,
    asker: &str,
    question: &str,
    files: &[String],
    write: bool,
) -> String {
    let mut prompt = format!(
        "You are {name}, consulted for a second opinion by {asker} working in this \
         repository. Answer the question directly and concisely. Give your own \
         independent judgment; disagree when you think the asker is wrong. You can \
         read the code to check your answer.\n",
        name = agent.name(),
    );
    if write {
        prompt.push_str("You may edit files if the question asks you to.\n");
    } else {
        prompt.push_str("This is a read-only consult: do not modify any files.\n");
    }

    for file in files {
        let path = std::path::Path::new(file);
        let size = std::fs::metadata(path).map(|m| m.len()).ok();
        match size {
            Some(len) if len <= INLINE_FILE_LIMIT => {
                let content = std::fs::read_to_string(path).unwrap_or_default();
                prompt.push_str(&format!("\n<file path=\"{file}\">\n{content}\n</file>\n"));
            }
            Some(_) => prompt.push_str(&format!("\nRelevant file (read it): {file}\n")),
            None => prompt.push_str(&format!("\nRelevant file (not found by asker): {file}\n")),
        }
    }

    prompt.push_str(&format!("\nQuestion:\n{question}\n"));
    prompt
}

fn build_invocation(
    agent: AskAgent,
    program: String,
    args: &AskArgs,
    output_file: &std::path::Path,
) -> Invocation {
    let mut a: Vec<String> = Vec::new();
    match agent {
        AskAgent::Claude => {
            a.extend(["-p", "--output-format", "json"].map(String::from));
            if let Some(session) = &args.resume {
                a.extend(["--resume".into(), session.clone()]);
            }
            if let Some(model) = &args.model {
                a.extend(["--model".into(), model.clone()]);
            }
            if args.write {
                a.extend(["--permission-mode", "acceptEdits"].map(String::from));
            } else {
                a.push("--allowedTools".into());
                a.push(CLAUDE_READ_ONLY_TOOLS.join(","));
                a.push("--disallowedTools".into());
                a.push(CLAUDE_EDIT_TOOLS.join(","));
            }
            // Prompt is written to stdin.
        }
        AskAgent::Codex => {
            let sandbox = if args.write {
                "workspace-write"
            } else {
                "read-only"
            };
            a.push("exec".into());
            if let Some(session) = &args.resume {
                // `codex exec resume` rejects --sandbox; pass it as config instead.
                a.extend(["resume".into(), session.clone()]);
                a.extend(["-c".into(), format!("sandbox_mode=\"{sandbox}\"")]);
            } else {
                a.extend(["--sandbox".into(), sandbox.into()]);
            }
            a.extend(["--skip-git-repo-check", "--json"].map(String::from));
            a.push("--output-last-message".into());
            a.push(output_file.to_string_lossy().into_owned());
            if let Some(model) = &args.model {
                a.extend(["--model".into(), model.clone()]);
            }
            a.push("-".into()); // read prompt from stdin
        }
    }
    Invocation { program, args: a }
}

/// Env vars that would make the child think it is (or belongs to) the caller.
fn env_strip_set(agent: AskAgent) -> HashSet<String> {
    let mut strip = crate::launcher::run_here_env_strip_set();
    for var in agent.tool().spec().instance_state_env {
        strip.insert((*var).to_string());
    }
    for var in [
        "HCOM",
        "HCOM_INSTANCE_NAME",
        "HCOM_TOOL",
        "HCOM_TAG",
        "HCOM_NOTES",
    ]
    .into_iter()
    .chain(CLAUDE_SESSION_SCOPED_ENV.iter().copied())
    {
        strip.insert(var.to_string());
    }
    strip
}

struct RunOutput {
    stdout: String,
    stderr: String,
    success: bool,
    timed_out: bool,
}

fn run_with_timeout(mut cmd: Command, stdin: &str, timeout: Duration) -> Result<RunOutput> {
    let mut child = cmd.spawn().context("failed to start agent")?;

    let mut child_stdin = child.stdin.take().expect("stdin piped");
    let input = stdin.to_string();
    let writer = std::thread::spawn(move || {
        let _ = child_stdin.write_all(input.as_bytes());
        // Dropping closes stdin so the agent sees EOF.
    });

    let mut out = child.stdout.take().expect("stdout piped");
    let mut err = child.stderr.take().expect("stderr piped");
    let out_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_reader = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    let _ = writer.join();
    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    Ok(RunOutput {
        stdout,
        stderr,
        success: status.is_some_and(|s| s.success()),
        timed_out,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct Answer {
    text: String,
    session: Option<String>,
    is_error: bool,
}

/// `claude -p --output-format json` prints one object with `result` and `session_id`.
fn parse_claude_output(stdout: &str) -> Option<Answer> {
    let value: serde_json::Value = stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str(line.trim()).ok())?;
    let text = value.get("result")?.as_str()?.to_string();
    Some(Answer {
        text,
        session: value
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(String::from),
        is_error: value
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// `codex exec --json` streams JSONL events (session id in `thread.started`);
/// the final message lands in the `--output-last-message` file.
fn parse_codex_output(stdout: &str, last_message_file: &std::path::Path) -> Option<Answer> {
    let session = stdout.lines().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        if v.get("type")?.as_str()? == "thread.started" {
            v.get("thread_id")?.as_str().map(String::from)
        } else {
            None
        }
    });
    let text = std::fs::read_to_string(last_message_file).ok()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(Answer {
        text,
        session,
        is_error: false,
    })
}

fn tail(s: &str, lines: usize) -> String {
    let all: Vec<&str> = s.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(argv: &[&str]) -> AskArgs {
        AskArgs::try_parse_from(std::iter::once("ask").chain(argv.iter().copied())).unwrap()
    }

    #[test]
    fn question_words_are_collected_after_flags() {
        let a = parse(&["codex", "--file", "x.rs", "is", "this", "racy?"]);
        assert_eq!(a.agent, "codex");
        assert_eq!(a.files, vec!["x.rs"]);
        assert_eq!(a.question.join(" "), "is this racy?");
        assert!(!a.write);
    }

    #[test]
    fn claude_read_only_restricts_tools() {
        let a = parse(&["claude", "q"]);
        let inv = build_invocation(
            AskAgent::Claude,
            "claude".into(),
            &a,
            std::path::Path::new("/tmp/o"),
        );
        assert_eq!(&inv.args[..3], &["-p", "--output-format", "json"]);
        let allowed = inv.args.iter().position(|x| x == "--allowedTools").unwrap();
        assert!(inv.args[allowed + 1].contains("Read"));
        assert!(!inv.args[allowed + 1].contains("Edit"));
        let disallowed = inv
            .args
            .iter()
            .position(|x| x == "--disallowedTools")
            .unwrap();
        assert!(inv.args[disallowed + 1].contains("Write"));
        assert!(!inv.args.contains(&"acceptEdits".to_string()));
    }

    #[test]
    fn claude_write_and_resume() {
        let a = parse(&["claude", "--write", "--continue", "sess-1", "q"]);
        let inv = build_invocation(
            AskAgent::Claude,
            "claude".into(),
            &a,
            std::path::Path::new("/tmp/o"),
        );
        assert!(inv.args.windows(2).any(|w| w == ["--resume", "sess-1"]));
        assert!(
            inv.args
                .windows(2)
                .any(|w| w == ["--permission-mode", "acceptEdits"])
        );
        assert!(!inv.args.contains(&"--allowedTools".to_string()));
    }

    #[test]
    fn codex_read_only_uses_sandbox_flag() {
        let a = parse(&["codex", "q"]);
        let inv = build_invocation(
            AskAgent::Codex,
            "codex".into(),
            &a,
            std::path::Path::new("/tmp/o"),
        );
        assert_eq!(inv.args[0], "exec");
        assert!(inv.args.windows(2).any(|w| w == ["--sandbox", "read-only"]));
        assert!(
            inv.args
                .windows(2)
                .any(|w| w == ["--output-last-message", "/tmp/o"])
        );
        assert_eq!(inv.args.last().unwrap(), "-");
    }

    #[test]
    fn codex_resume_passes_sandbox_as_config() {
        let a = parse(&["codex", "--continue", "t-9", "q"]);
        let inv = build_invocation(
            AskAgent::Codex,
            "codex".into(),
            &a,
            std::path::Path::new("/tmp/o"),
        );
        assert_eq!(&inv.args[..3], &["exec", "resume", "t-9"]);
        assert!(
            inv.args
                .windows(2)
                .any(|w| w == ["-c", "sandbox_mode=\"read-only\""])
        );
        assert!(!inv.args.contains(&"--sandbox".to_string()));
    }

    #[test]
    fn unknown_agent_is_rejected() {
        assert!(AskAgent::parse("gemini").is_err());
        assert_eq!(AskAgent::parse("Codex").unwrap(), AskAgent::Codex);
    }

    #[test]
    fn prompt_inlines_small_files_and_states_mode() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("small.txt");
        std::fs::write(&small, "hello world").unwrap();
        let small = small.to_string_lossy().into_owned();

        let p = build_prompt(
            AskAgent::Codex,
            "luna",
            "why?",
            std::slice::from_ref(&small),
            false,
        );
        assert!(p.contains("by luna"));
        assert!(p.contains("read-only consult"));
        assert!(p.contains(&format!("<file path=\"{small}\">\nhello world")));
        assert!(p.trim_end().ends_with("why?"));

        let p = build_prompt(AskAgent::Codex, "luna", "why?", &["/nope".into()], true);
        assert!(p.contains("may edit files"));
        assert!(p.contains("not found by asker): /nope"));
    }

    #[test]
    fn parses_claude_json_result() {
        let out = r#"{"type":"result","is_error":false,"result":"Looks fine.","session_id":"abc"}"#;
        assert_eq!(
            parse_claude_output(out),
            Some(Answer {
                text: "Looks fine.".into(),
                session: Some("abc".into()),
                is_error: false,
            })
        );
        assert_eq!(parse_claude_output("not json"), None);
    }

    #[test]
    fn parses_codex_events_and_last_message() {
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), "Race on line 40.\n").unwrap();
        let events =
            "{\"type\":\"thread.started\",\"thread_id\":\"t-1\"}\n{\"type\":\"turn.completed\"}\n";
        let a = parse_codex_output(events, file.path()).unwrap();
        assert_eq!(a.text, "Race on line 40.\n");
        assert_eq!(a.session.as_deref(), Some("t-1"));

        std::fs::write(file.path(), "  \n").unwrap();
        assert_eq!(parse_codex_output(events, file.path()), None);
    }

    #[test]
    fn strip_set_clears_caller_identity() {
        let strip = env_strip_set(AskAgent::Claude);
        assert!(strip.contains("HCOM_INSTANCE_NAME"));
        assert!(strip.contains("HCOM_PROCESS_ID"));
        assert!(strip.contains("CLAUDECODE"));
        assert!(strip.contains("CLAUDE_CODE_SESSION_ID"));
        assert!(!strip.contains("ANTHROPIC_BASE_URL"));
    }
}
