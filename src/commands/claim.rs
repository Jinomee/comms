//! Claim commands: `hcom claim`, `hcom release`, `hcom claims`.
//!
//! See `crate::claims` for semantics. Patterns are resolved against the
//! current directory; the holder is the caller's hcom identity, or the human
//! (`bigboss`) when run from a plain terminal.

use std::path::{Path, PathBuf};

use crate::claims::{self, Claim, ClaimOutcome};
use crate::db::HcomDb;
use crate::shared::identity::{CommandContext, SenderKind};

#[derive(clap::Parser, Debug)]
#[command(name = "claim", about = "Claim files so other agents don't edit them")]
pub struct ClaimArgs {
    /// Paths, directories, or globs (relative to the current directory)
    #[arg(required = true)]
    pub patterns: Vec<String>,
    /// What you're doing (shown to anyone blocked by the claim)
    #[arg(long, short = 'n', default_value = "")]
    pub note: String,
    /// How long the claim lasts without edits: 90s, 30m, 2h (default 30m)
    #[arg(long, value_parser = parse_duration)]
    pub ttl: Option<i64>,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Parser, Debug)]
#[command(name = "release", about = "Release your claims")]
pub struct ReleaseArgs {
    /// Patterns to release (as claimed)
    pub patterns: Vec<String>,
    /// Release all of your claims
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Parser, Debug)]
#[command(name = "claims", about = "List live claims")]
pub struct ClaimsArgs {
    /// Exit 1 if any of these paths is claimed by someone else
    #[arg(long, num_args = 1..)]
    pub check: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

fn parse_duration(s: &str) -> Result<i64, String> {
    let s = s.trim();
    let (num, unit) = s
        .find(|c: char| !c.is_ascii_digit())
        .map_or((s, ""), |i| s.split_at(i));
    let n: i64 = num.parse().map_err(|_| format!("invalid duration '{s}'"))?;
    let secs = match unit {
        "" | "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return Err(format!("invalid duration '{s}' (use 90s, 30m, 2h)")),
    };
    if secs <= 0 {
        return Err("duration must be positive".into());
    }
    Ok(secs)
}

/// Who holds claims made from this shell.
fn holder(ctx: Option<&CommandContext>) -> Result<String, String> {
    if let Some(id) = ctx.and_then(|c| c.identity.as_ref())
        && matches!(id.kind, SenderKind::Instance)
        && id.instance_data.is_some()
    {
        return Ok(id.name.clone());
    }
    if crate::shared::platform::is_inside_ai_tool() {
        let hcom = crate::runtime_env::build_hcom_command();
        return Err(format!(
            "hcom identity not found; run '{hcom} start' first (or pass --name <you>)"
        ));
    }
    Ok(crate::shared::constants::SENDER.to_string())
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn claim_json(c: &Claim, now: i64) -> serde_json::Value {
    serde_json::json!({
        "id": c.id,
        "holder": c.holder,
        "pattern": c.pattern,
        "note": c.note,
        "expires_in": (c.expires_at - now).max(0),
    })
}

fn describe(c: &Claim, base: &Path, now: i64) -> String {
    let note = if c.note.is_empty() {
        String::new()
    } else {
        format!(" (\"{}\")", c.note)
    };
    format!(
        "{} by {}{}, {} left",
        claims::display_pattern(&c.pattern, base),
        c.holder,
        note,
        claims::format_remaining(c.expires_at, now)
    )
}

pub fn cmd_claim(db: &HcomDb, args: &ClaimArgs, ctx: Option<&CommandContext>) -> i32 {
    let holder = match holder(ctx) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    let base = cwd();
    let shown_base = claims::display_base(&base);
    let now = crate::shared::time::now_epoch_i64();
    let patterns: Vec<String> = args
        .patterns
        .iter()
        .map(|p| claims::normalize_pattern(p, &base))
        .collect();
    let ttl = args.ttl.unwrap_or(claims::DEFAULT_TTL_SECS);
    let outcomes = match claims::claim(db, &holder, &patterns, &args.note, ttl, now) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    let conflicted = outcomes
        .iter()
        .any(|o| matches!(o, ClaimOutcome::Conflict { .. }));
    if args.json {
        let items: Vec<_> = outcomes
            .iter()
            .map(|o| match o {
                ClaimOutcome::Claimed(c) => {
                    serde_json::json!({"result": "claimed", "claim": claim_json(c, now)})
                }
                ClaimOutcome::Renewed(c) => {
                    serde_json::json!({"result": "renewed", "claim": claim_json(c, now)})
                }
                ClaimOutcome::Conflict { pattern, existing } => serde_json::json!({
                    "result": "conflict", "pattern": pattern, "held": claim_json(existing, now)
                }),
            })
            .collect();
        println!("{}", serde_json::Value::Array(items));
    } else {
        let hcom = crate::runtime_env::build_hcom_command();
        for o in &outcomes {
            match o {
                ClaimOutcome::Claimed(c) => println!("Claimed {}", describe(c, &shown_base, now)),
                ClaimOutcome::Renewed(c) => println!("Renewed {}", describe(c, &shown_base, now)),
                ClaimOutcome::Conflict { pattern, existing } => {
                    println!(
                        "Not claimed: {} overlaps {}. Ask them: {hcom} send @{} -- ...",
                        claims::display_pattern(pattern, &shown_base),
                        describe(existing, &shown_base, now),
                        existing.holder
                    );
                }
            }
        }
    }
    if conflicted { 1 } else { 0 }
}

pub fn cmd_release(db: &HcomDb, args: &ReleaseArgs, ctx: Option<&CommandContext>) -> i32 {
    if args.patterns.is_empty() && !args.all {
        eprintln!("Error: name the patterns to release, or pass --all");
        return 1;
    }
    let holder = match holder(ctx) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    let base = cwd();
    let now = crate::shared::time::now_epoch_i64();
    let patterns: Vec<String> = if args.all {
        Vec::new()
    } else {
        args.patterns
            .iter()
            .map(|p| claims::normalize_pattern(p, &base))
            .collect()
    };
    let released = match claims::release(db, &holder, &patterns, now) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    if args.json {
        let items: Vec<_> = released.iter().map(|c| claim_json(c, now)).collect();
        println!("{}", serde_json::json!({"released": items}));
    } else if released.is_empty() {
        println!("Nothing to release");
    } else {
        for c in &released {
            println!(
                "Released {}",
                claims::display_pattern(&c.pattern, &claims::display_base(&base))
            );
        }
    }
    0
}

pub fn cmd_claims(db: &HcomDb, args: &ClaimsArgs, ctx: Option<&CommandContext>) -> i32 {
    let base = cwd();
    let shown_base = claims::display_base(&base);
    let now = crate::shared::time::now_epoch_i64();

    if !args.check.is_empty() {
        let me = holder(ctx).unwrap_or_default();
        let paths: Vec<PathBuf> = args
            .check
            .iter()
            .map(|p| claims::normalize_path(Path::new(p), &base))
            .collect();
        let mut blocked = Vec::new();
        for path in &paths {
            match claims::blocking_claim(db, &me, std::slice::from_ref(path), now) {
                Ok(Some((c, p))) => blocked.push((p, c)),
                Ok(None) => {}
                Err(e) => {
                    eprintln!("Error: {e}");
                    return 1;
                }
            }
        }
        if args.json {
            let items: Vec<_> = blocked
                .iter()
                .map(|(p, c)| serde_json::json!({"path": p, "held": claim_json(c, now)}))
                .collect();
            println!("{}", serde_json::json!({"blocked": items}));
        } else if blocked.is_empty() {
            println!("Free to edit");
        } else {
            for (p, c) in &blocked {
                println!(
                    "{}: {}",
                    claims::display_pattern(&p.to_string_lossy(), &shown_base),
                    describe(c, &shown_base, now)
                );
            }
        }
        return if blocked.is_empty() { 0 } else { 1 };
    }

    let live = match claims::active(db, now) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    if args.json {
        let items: Vec<_> = live.iter().map(|c| claim_json(c, now)).collect();
        println!("{}", serde_json::Value::Array(items));
    } else if live.is_empty() {
        println!("No claims");
    } else {
        for c in &live {
            println!("{}", describe(c, &shown_base, now));
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90"), Ok(90));
        assert_eq!(parse_duration("90s"), Ok(90));
        assert_eq!(parse_duration("30m"), Ok(1800));
        assert_eq!(parse_duration("2h"), Ok(7200));
        assert!(parse_duration("0m").is_err());
        assert!(parse_duration("5d").is_err());
        assert!(parse_duration("abc").is_err());
    }
}
