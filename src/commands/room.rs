//! Room commands: `comms room [list|join|leave|add|remove|members|show|delete]`.
//!
//! See `crate::rooms`. Rooms are opt-in; nothing here changes how agents
//! outside rooms, @mentions, or `--thread` behave.

use clap::Subcommand;

use crate::db::CommsDb;
use crate::rooms;
use crate::shared::identity::{CommandContext, SenderKind};

#[derive(clap::Parser, Debug)]
#[command(name = "room", about = "Named chatrooms for groups of agents")]
pub struct RoomArgs {
    #[command(subcommand)]
    pub action: Option<RoomAction>,
    #[arg(long, global = true)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum RoomAction {
    /// List rooms and their members (default)
    List,
    /// Join a room (creating it) and make it where your plain messages go
    Join { room: String },
    /// Leave a room (default: your current room)
    Leave { room: Option<String> },
    /// Put agents into a room
    Add {
        room: String,
        #[arg(required = true)]
        agents: Vec<String>,
    },
    /// Take agents out of a room
    Remove {
        room: String,
        #[arg(required = true)]
        agents: Vec<String>,
    },
    /// Who is in a room
    Members { room: String },
    /// Recent messages in a room
    Show {
        room: String,
        /// How many messages
        #[arg(long, default_value_t = 20)]
        last: usize,
    },
    /// Remove a room and all its members (messages are kept)
    Delete { room: String },
}

/// The calling agent's name, if this shell is a registered agent.
fn caller(ctx: Option<&CommandContext>) -> Option<String> {
    ctx.and_then(|c| c.identity.as_ref())
        .filter(|id| matches!(id.kind, SenderKind::Instance) && id.instance_data.is_some())
        .map(|id| id.name.clone())
}

fn require_caller(ctx: Option<&CommandContext>, what: &str) -> Result<String, String> {
    caller(ctx).ok_or_else(|| {
        let comms = crate::runtime_env::build_comms_command();
        format!(
            "only an agent can {what} a room itself; from a terminal use `{comms} room add|remove <room> <agent>...`"
        )
    })
}

/// Tell an agent its room membership changed (unless it made the change).
/// Its plain messages now go somewhere else, so it must know.
fn notify(db: &CommsDb, agent: &str, by: Option<&str>, what: &str) {
    if by == Some(agent) {
        return;
    }
    let by = by.unwrap_or(crate::shared::constants::SENDER);
    let route = match rooms::current(db, agent) {
        Some(room) => format!("Your plain messages now go to room '{room}'."),
        None => "Your plain messages now go to all agents.".to_string(),
    };
    let comms = crate::runtime_env::build_comms_command();
    let _ = db.send_system_message(
        "rooms",
        &format!(
            "@{agent} {by} {what}. {route} Use @name for one agent, `{comms} send --all` for everyone; `{comms} room` shows your rooms."
        ),
    );
}

fn known_agent(db: &CommsDb, name: &str) -> bool {
    matches!(db.get_instance_full(name), Ok(Some(_)))
}

fn exists_or_err(db: &CommsDb, room: &str) -> Result<(), String> {
    if rooms::exists(db, room) {
        Ok(())
    } else {
        Err(format!("no room named '{room}' (see `comms room list`)"))
    }
}

pub fn cmd_room(db: &CommsDb, args: &RoomArgs, ctx: Option<&CommandContext>) -> i32 {
    match run(db, args, ctx) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn run(db: &CommsDb, args: &RoomArgs, ctx: Option<&CommandContext>) -> Result<(), String> {
    let me = caller(ctx);
    match args.action.as_ref().unwrap_or(&RoomAction::List) {
        RoomAction::List => {
            let all = rooms::list(db).map_err(|e| e.to_string())?;
            let current = me.as_deref().and_then(|n| rooms::current(db, n));
            if args.json {
                let items: Vec<_> = all
                    .iter()
                    .map(|r| {
                        serde_json::json!({
                            "room": r,
                            "members": rooms::members(db, r),
                            "current": current.as_deref() == Some(r.as_str()),
                        })
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(items));
            } else if all.is_empty() {
                println!(
                    "No rooms. Create one with `comms room join <room>` or `comms room add <room> <agent>...`"
                );
            } else {
                for r in &all {
                    let members = rooms::members(db, r);
                    let mark = if current.as_deref() == Some(r.as_str()) {
                        " (your room)"
                    } else {
                        ""
                    };
                    let who = if members.is_empty() {
                        "empty".to_string()
                    } else {
                        members.join(", ")
                    };
                    println!("{r}{mark}: {who}");
                }
            }
        }
        RoomAction::Join { room } => {
            let name = require_caller(ctx, "join")?;
            rooms::join(db, room, &name).map_err(|e| e.to_string())?;
            println!("{}", rooms::launch_note(db, room));
        }
        RoomAction::Leave { room } => {
            let name = require_caller(ctx, "leave")?;
            let room = match room {
                Some(r) => r.clone(),
                None => rooms::current(db, &name).ok_or("you're not in a room".to_string())?,
            };
            let was = rooms::leave(db, &room, &name).map_err(|e| e.to_string())?;
            if !was {
                return Err(format!("you're not in room '{room}'"));
            }
            match rooms::current(db, &name) {
                Some(next) => println!("Left '{room}'. Plain messages now go to room '{next}'."),
                None => println!("Left '{room}'. Plain messages now go to all agents."),
            }
        }
        RoomAction::Add { room, agents } => {
            rooms::validate_name(room).map_err(|e| e.to_string())?;
            let unknown: Vec<&String> = agents.iter().filter(|a| !known_agent(db, a)).collect();
            if !unknown.is_empty() {
                return Err(format!(
                    "unknown agent(s): {} (see `comms list`)",
                    unknown
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            for agent in agents {
                let already = rooms::is_member(db, room, agent);
                rooms::join(db, room, agent).map_err(|e| e.to_string())?;
                if !already {
                    let others: Vec<String> = rooms::members(db, room)
                        .into_iter()
                        .filter(|m| m != agent)
                        .collect();
                    let with = if others.is_empty() {
                        String::new()
                    } else {
                        format!(" with {}", others.join(", "))
                    };
                    notify(
                        db,
                        agent,
                        me.as_deref(),
                        &format!("added you to room '{room}'{with}"),
                    );
                }
            }
            println!("Room '{room}': {}", rooms::members(db, room).join(", "));
        }
        RoomAction::Remove { room, agents } => {
            exists_or_err(db, room)?;
            for agent in agents {
                if rooms::leave(db, room, agent).map_err(|e| e.to_string())? {
                    notify(
                        db,
                        agent,
                        me.as_deref(),
                        &format!("removed you from room '{room}'"),
                    );
                }
            }
            let left = rooms::members(db, room);
            if left.is_empty() {
                println!("Room '{room}' is now empty");
            } else {
                println!("Room '{room}': {}", left.join(", "));
            }
        }
        RoomAction::Members { room } => {
            exists_or_err(db, room)?;
            let members = rooms::members(db, room);
            if args.json {
                println!("{}", serde_json::json!({"room": room, "members": members}));
            } else if members.is_empty() {
                println!("Room '{room}' is empty");
            } else {
                println!("{}", members.join("\n"));
            }
        }
        RoomAction::Show { room, last } => {
            exists_or_err(db, room)?;
            let messages = room_messages(db, room, *last).map_err(|e| e.to_string())?;
            if args.json {
                let items: Vec<_> = messages
                    .iter()
                    .map(|(id, ts, from, text)| {
                        serde_json::json!({"id": id, "timestamp": ts, "from": from, "text": text})
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(items));
            } else if messages.is_empty() {
                println!("No messages in '{room}' yet");
            } else {
                for (id, ts, from, text) in &messages {
                    let time = ts.get(11..16).unwrap_or(ts);
                    println!("#{id} {time} {from}: {text}");
                }
            }
        }
        RoomAction::Delete { room } => {
            exists_or_err(db, room)?;
            let former = rooms::members(db, room);
            let n = rooms::delete(db, room).map_err(|e| e.to_string())?;
            for agent in &former {
                notify(db, agent, me.as_deref(), &format!("deleted room '{room}'"));
            }
            println!("Deleted room '{room}' ({n} member(s) removed; messages kept)");
        }
    }
    Ok(())
}

/// (event id, timestamp, sender, text) of the last `limit` room messages, oldest first.
fn room_messages(
    db: &CommsDb,
    room: &str,
    limit: usize,
) -> anyhow::Result<Vec<(i64, String, String, String)>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, timestamp, json_extract(data, '$.from'), json_extract(data, '$.text')
         FROM events
         WHERE type = 'message' AND json_extract(data, '$.thread') = ?
         ORDER BY id DESC LIMIT ?",
    )?;
    let mut rows: Vec<(i64, String, String, String)> = stmt
        .query_map(rusqlite::params![room, limit as i64], |row| {
            Ok((
                row.get(0)?,
                row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            ))
        })?
        .filter_map(|r| r.ok())
        .collect();
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::identity::SenderIdentity;

    fn test_db_with(names: &[&str]) -> CommsDb {
        let dir = tempfile::tempdir().unwrap();
        let db = CommsDb::open_raw(&dir.path().join("t.db")).unwrap();
        db.init_db().unwrap();
        std::mem::forget(dir);
        for n in names {
            db.conn()
                .execute(
                    "INSERT INTO instances (name, tool, status, status_context, created_at) \
                     VALUES (?, 'claude', 'active', '', 0)",
                    [n],
                )
                .unwrap();
        }
        db
    }

    /// (delivered_to, text) of room notices, oldest first.
    fn notices(db: &CommsDb) -> Vec<(String, String)> {
        let mut stmt = db
            .conn()
            .prepare(
                "SELECT json_extract(data, '$.delivered_to'), json_extract(data, '$.text') \
                 FROM events WHERE type = 'message' AND instance = 'sys_rooms' ORDER BY id",
            )
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    fn room(action: RoomAction) -> RoomArgs {
        RoomArgs {
            action: Some(action),
            json: false,
        }
    }

    #[test]
    fn agents_are_told_when_others_change_their_rooms() {
        let db = test_db_with(&["luna", "nova"]);
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        // Human adds both: each is told, and who else is there.
        let add = room(RoomAction::Add {
            room: "auth".into(),
            agents: s(&["luna", "nova"]),
        });
        assert_eq!(cmd_room(&db, &add, None), 0);
        let n = notices(&db);
        assert_eq!(n.len(), 2);
        assert_eq!(n[0].0, r#"["luna"]"#);
        assert!(
            n[0].1.contains("bigboss added you to room 'auth'"),
            "{}",
            n[0].1
        );
        assert!(n[0].1.contains("now go to room 'auth'"));
        assert!(n[1].1.contains("room 'auth' with luna"), "{}", n[1].1);

        // Re-adding an existing member is silent.
        assert_eq!(cmd_room(&db, &add, None), 0);
        assert_eq!(notices(&db).len(), 2);

        let remove = room(RoomAction::Remove {
            room: "auth".into(),
            agents: s(&["nova"]),
        });
        assert_eq!(cmd_room(&db, &remove, None), 0);
        let last = notices(&db).pop().unwrap();
        assert_eq!(last.0, r#"["nova"]"#);
        assert!(last.1.contains("removed you from room 'auth'"));
        assert!(last.1.contains("now go to all agents"));

        assert_eq!(
            cmd_room(
                &db,
                &room(RoomAction::Delete {
                    room: "auth".into()
                }),
                None
            ),
            0
        );
        let last = notices(&db).pop().unwrap();
        assert_eq!(last.0, r#"["luna"]"#);
        assert!(last.1.contains("deleted room 'auth'"));
    }

    #[test]
    fn agents_changing_their_own_rooms_get_no_notice() {
        let db = test_db_with(&["luna"]);
        let ctx = CommandContext {
            explicit_name: None,
            identity: Some(SenderIdentity {
                kind: SenderKind::Instance,
                name: "luna".into(),
                instance_data: Some(serde_json::json!({"name": "luna"})),
                session_id: None,
            }),
            go: false,
            identity_warning: None,
        };
        let join = room(RoomAction::Join {
            room: "auth".into(),
        });
        assert_eq!(cmd_room(&db, &join, Some(&ctx)), 0);
        assert_eq!(
            cmd_room(&db, &room(RoomAction::Leave { room: None }), Some(&ctx)),
            0
        );
        assert!(notices(&db).is_empty());
        // A human can't "join": only agents receive messages.
        assert_eq!(cmd_room(&db, &join, None), 1);
    }

    #[test]
    fn events_room_filter_is_the_thread_filter() {
        use clap::Parser;
        let args =
            crate::commands::events::EventsArgs::try_parse_from(["events", "--room", "auth"]);
        assert!(args.is_ok(), "{:?}", args.err());
        let (filters, rest) =
            crate::core::filters::parse_event_flags(&["--room".to_string(), "auth".to_string()])
                .unwrap();
        assert!(rest.is_empty());
        assert_eq!(filters.get("thread"), Some(&vec!["auth".to_string()]));
    }

    #[test]
    fn show_lists_only_room_messages_oldest_first() {
        let dir = tempfile::tempdir().unwrap();
        let db = CommsDb::open_raw(&dir.path().join("t.db")).unwrap();
        db.init_db().unwrap();
        for n in ["luna", "nova", "kira"] {
            db.conn()
                .execute(
                    "INSERT INTO instances (name, tool, status, status_context, created_at) \
                     VALUES (?, 'claude', 'active', '', 0)",
                    [n],
                )
                .unwrap();
        }
        rooms::join(&db, "auth", "luna").unwrap();
        rooms::join(&db, "auth", "nova").unwrap();
        let agent = |n: &str| SenderIdentity {
            kind: SenderKind::Instance,
            name: n.into(),
            instance_data: None,
            session_id: None,
        };
        let send = crate::commands::send::send_message;
        send(&db, &agent("luna"), "first", None, None).unwrap();
        send(&db, &agent("kira"), "not in the room", None, None).unwrap();
        send(&db, &agent("nova"), "second", None, None).unwrap();

        let msgs = room_messages(&db, "auth", 10).unwrap();
        let texts: Vec<_> = msgs.iter().map(|m| (m.2.as_str(), m.3.as_str())).collect();
        assert_eq!(texts, vec![("luna", "first"), ("nova", "second")]);
        assert_eq!(room_messages(&db, "auth", 1).unwrap()[0].3, "second");
    }
}
