//! Budget command: `hcom budget [reset | <limit>]`
//!
//! Shows the agent→agent turn budget (see `crate::turn_budget`). Changing it
//! is for the human only: agents can't lift their own limit.

use crate::db::HcomDb;
use crate::shared::identity::CommandContext;
use crate::turn_budget;

#[derive(clap::Parser, Debug)]
#[command(
    name = "budget",
    about = "Show or change the agent-to-agent turn budget"
)]
pub struct BudgetArgs {
    /// "reset" to clear all counts, or a number to set the limit (0 = off)
    pub action: Option<String>,
    #[arg(long)]
    pub json: bool,
}

pub fn cmd_budget(db: &HcomDb, args: &BudgetArgs, _ctx: Option<&CommandContext>) -> i32 {
    if let Some(action) = args.action.as_deref() {
        if crate::shared::platform::is_inside_ai_tool() {
            eprintln!(
                "Error: only the human can change the turn budget. Ask them, e.g. `{} send @{} -- ...`",
                crate::runtime_env::build_hcom_command(),
                crate::shared::constants::SENDER
            );
            return 1;
        }
        if action == "reset" {
            turn_budget::reset_all(db);
            println!("Turn counts reset");
            return 0;
        }
        match action.parse::<u32>() {
            Ok(n) => {
                if let Err(e) = turn_budget::set_limit(db, n) {
                    eprintln!("Error: {e}");
                    return 1;
                }
                if n == 0 {
                    println!("Turn budget off");
                } else {
                    println!("Turn budget set to {n} messages per agent pair");
                }
                return 0;
            }
            Err(_) => {
                eprintln!("Error: expected 'reset' or a number, got '{action}'");
                return 1;
            }
        }
    }

    let limit = turn_budget::limit(db);
    let pairs = turn_budget::pairs(db);
    if args.json {
        let items: Vec<_> = pairs
            .iter()
            .map(|(a, b, n)| serde_json::json!({"agents": [a, b], "count": n}))
            .collect();
        println!("{}", serde_json::json!({"limit": limit, "pairs": items}));
        return 0;
    }
    if limit == 0 {
        println!("Turn budget: off");
    } else {
        println!("Turn budget: {limit} messages per agent pair since the last human input");
    }
    if pairs.is_empty() {
        println!("No agent-to-agent messages counted");
    }
    for (a, b, n) in &pairs {
        let mark = if limit > 0 && *n >= limit {
            "  (exhausted)"
        } else {
            ""
        };
        if limit > 0 {
            println!("  {a} ↔ {b}: {n}/{limit}{mark}");
        } else {
            println!("  {a} ↔ {b}: {n}");
        }
    }
    0
}
