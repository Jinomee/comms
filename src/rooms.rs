//! Rooms: named, opt-in chatrooms layered on threads.
//!
//! A room is a registered thread name with explicit membership. Membership
//! reuses the thread-member rows that `send --thread` already routes by, so a
//! room message is just a thread message and everything thread-aware (events
//! filters, receipts, the TUI) sees it unchanged.
//!
//! This is purely additive:
//! - Agents not in any room behave exactly as before.
//! - `--thread`, @mentions, and broadcasts work exactly as before.
//! - The one new behavior: an agent that is in a room and sends a plain
//!   message (no @mention, no --thread) sends it to its current room instead
//!   of broadcasting. `send --all` still broadcasts.
//!
//! Stored in the `kv` table under `comms_room:` (registry) and
//! `comms_room_of:` (each agent's current room).

use anyhow::{Result, bail};

use crate::db::CommsDb;
use crate::messages::MessageEnvelope;
use crate::shared::identity::{SenderIdentity, SenderKind};

const ROOM_PREFIX: &str = "comms_room:";
const CURRENT_PREFIX: &str = "comms_room_of:";
pub const LAUNCH_FLAG: &str = "--room";

/// Room names follow thread-name rules (they are thread names).
pub fn validate_name(room: &str) -> Result<()> {
    if room.is_empty() || room.len() > 64 {
        bail!("room name must be 1-64 characters");
    }
    if !room
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        bail!("room name must be alphanumeric with hyphens/underscores");
    }
    if room.eq_ignore_ascii_case("all") {
        bail!("'all' is reserved");
    }
    Ok(())
}

pub fn exists(db: &CommsDb, room: &str) -> bool {
    matches!(db.kv_get(&format!("{ROOM_PREFIX}{room}")), Ok(Some(_)))
}

fn ensure(db: &CommsDb, room: &str) -> Result<()> {
    if !exists(db, room) {
        let data = serde_json::json!({ "created_at": crate::shared::time::now_epoch_i64() });
        db.kv_set(&format!("{ROOM_PREFIX}{room}"), Some(&data.to_string()))?;
    }
    Ok(())
}

/// All registered rooms, sorted by name.
pub fn list(db: &CommsDb) -> Result<Vec<String>> {
    let mut rooms: Vec<String> = db
        .kv_prefix(ROOM_PREFIX)?
        .into_iter()
        .filter_map(|(key, _)| key.strip_prefix(ROOM_PREFIX).map(String::from))
        .collect();
    rooms.sort();
    Ok(rooms)
}

/// Current members (live agents), in join order.
pub fn members(db: &CommsDb, room: &str) -> Vec<String> {
    db.get_thread_members(room)
}

pub fn is_member(db: &CommsDb, room: &str, name: &str) -> bool {
    members(db, room).iter().any(|m| m == name)
}

/// Rooms `name` belongs to.
pub fn rooms_of(db: &CommsDb, name: &str) -> Vec<String> {
    list(db)
        .unwrap_or_default()
        .into_iter()
        .filter(|room| is_member(db, room, name))
        .collect()
}

/// The room `name` sends plain messages to, if it is (still) a member.
pub fn current(db: &CommsDb, name: &str) -> Option<String> {
    let room = db
        .kv_get(&format!("{CURRENT_PREFIX}{name}"))
        .ok()
        .flatten()?;
    (exists(db, &room) && is_member(db, &room, name)).then_some(room)
}

fn set_current(db: &CommsDb, name: &str, room: Option<&str>) -> Result<()> {
    db.kv_set(&format!("{CURRENT_PREFIX}{name}"), room)
}

/// Add `name` to `room` (creating it) and make it `name`'s current room.
pub fn join(db: &CommsDb, room: &str, name: &str) -> Result<()> {
    validate_name(room)?;
    ensure(db, room)?;
    db.add_thread_memberships(room, None, &[name.to_string()]);
    set_current(db, name, Some(room))
}

/// Remove `name` from `room`. If it was the current room, fall back to
/// another room `name` is in, else none.
pub fn leave(db: &CommsDb, room: &str, name: &str) -> Result<bool> {
    let was_member = is_member(db, room, name);
    let sub_id = crate::db::subscriptions::thread_membership_sub_id(room, name);
    db.kv_set(&format!("events_sub:{sub_id}"), None)?;
    if db.kv_get(&format!("{CURRENT_PREFIX}{name}"))?.as_deref() == Some(room) {
        let next = rooms_of(db, name).into_iter().next();
        set_current(db, name, next.as_deref())?;
    }
    Ok(was_member)
}

/// Remove every member and the room itself. Past messages are kept.
pub fn delete(db: &CommsDb, room: &str) -> Result<usize> {
    let all = members(db, room);
    for name in &all {
        leave(db, room, name)?;
    }
    db.kv_set(&format!("{ROOM_PREFIX}{room}"), None)?;
    Ok(all.len())
}

/// If an agent in a room sends a plain message, route it to its current room
/// by returning an envelope with `thread` set. `None` means "send as usual".
pub fn default_envelope(
    db: &CommsDb,
    identity: &SenderIdentity,
    message: &str,
    envelope: Option<&MessageEnvelope>,
    explicit_targets: Option<&[String]>,
) -> Option<MessageEnvelope> {
    if !matches!(identity.kind, SenderKind::Instance) {
        return None;
    }
    if envelope.is_some_and(|env| env.thread.is_some() || env.skip_room) {
        return None;
    }
    if explicit_targets.is_some_and(|t| !t.is_empty()) {
        return None;
    }
    if !crate::shared::constants::extract_mentions(message).is_empty() {
        return None;
    }
    let room = current(db, &identity.name)?;
    let mut routed = envelope.cloned().unwrap_or_default();
    routed.thread = Some(room);
    Some(routed)
}

/// Peel `--room <name>` (or `--room=<name>`) off launch args, before any `--`.
pub fn take_launch_flag(args: &[String]) -> Result<(Option<String>, Vec<String>)> {
    let mut room = None;
    let mut rest = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--" {
            rest.push(arg.clone());
            rest.extend(iter.by_ref().cloned());
            break;
        }
        if arg == LAUNCH_FLAG {
            let Some(value) = iter.next() else {
                bail!("{LAUNCH_FLAG} needs a room name");
            };
            room = Some(value.clone());
        } else if let Some(value) = arg.strip_prefix("--room=") {
            room = Some(value.to_string());
        } else {
            rest.push(arg.clone());
        }
    }
    if let Some(ref r) = room {
        validate_name(r)?;
    }
    Ok((room, rest))
}

/// System-prompt note for an agent launched into a room.
pub fn launch_note(db: &CommsDb, room: &str) -> String {
    let comms = crate::runtime_env::build_comms_command();
    let others = members(db, room);
    let who = if others.is_empty() {
        String::new()
    } else {
        format!(" Already here: {}.", others.join(", "))
    };
    format!(
        "You are in the comms room '{room}'.{who} A plain `{comms} send -- <text>` (no @mention) \
         goes to everyone in this room, not to all agents. Use @name to message one agent, \
         `{comms} send --all -- <text>` to reach every agent. `{comms} room members {room}` lists who is here; \
         `{comms} room show {room}` shows the room's history."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> CommsDb {
        let dir = tempfile::tempdir().unwrap();
        let db = CommsDb::open_raw(&dir.path().join("test.db")).unwrap();
        db.init_db().unwrap();
        std::mem::forget(dir);
        db
    }

    fn add_instance(db: &CommsDb, name: &str) {
        db.conn()
            .execute(
                "INSERT INTO instances (name, tool, status, status_context, created_at) \
                 VALUES (?, 'claude', 'active', '', 0)",
                [name],
            )
            .unwrap();
    }

    fn agent(name: &str) -> SenderIdentity {
        SenderIdentity {
            kind: SenderKind::Instance,
            name: name.into(),
            instance_data: None,
            session_id: None,
        }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn names_are_validated() {
        assert!(validate_name("auth-refactor_2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("has space").is_err());
        assert!(validate_name("all").is_err());
        assert!(validate_name(&"x".repeat(65)).is_err());
    }

    #[test]
    fn join_leave_current_and_delete() {
        let db = test_db();
        for n in ["luna", "nova"] {
            add_instance(&db, n);
        }
        join(&db, "auth", "luna").unwrap();
        join(&db, "auth", "nova").unwrap();
        join(&db, "docs", "luna").unwrap();
        assert_eq!(list(&db).unwrap(), s(&["auth", "docs"]));
        assert_eq!(members(&db, "auth"), s(&["luna", "nova"]));
        assert_eq!(current(&db, "luna").as_deref(), Some("docs"));

        // Leaving the current room falls back to another room.
        assert!(leave(&db, "docs", "luna").unwrap());
        assert_eq!(current(&db, "luna").as_deref(), Some("auth"));
        assert!(!leave(&db, "docs", "luna").unwrap());

        assert_eq!(delete(&db, "auth").unwrap(), 2);
        assert!(!exists(&db, "auth"));
        assert_eq!(current(&db, "luna"), None);
        assert_eq!(current(&db, "nova"), None);
    }

    #[test]
    fn default_envelope_only_for_plain_messages_from_room_members() {
        let db = test_db();
        for n in ["luna", "nova", "kira"] {
            add_instance(&db, n);
        }
        join(&db, "auth", "luna").unwrap();

        let routed = default_envelope(&db, &agent("luna"), "status update", None, None).unwrap();
        assert_eq!(routed.thread.as_deref(), Some("auth"));

        // Not in a room, @mentions, explicit targets, explicit thread: unchanged.
        assert!(default_envelope(&db, &agent("kira"), "hi", None, None).is_none());
        assert!(default_envelope(&db, &agent("luna"), "@nova hi", None, None).is_none());
        let everyone = MessageEnvelope {
            skip_room: true,
            ..Default::default()
        };
        assert!(default_envelope(&db, &agent("luna"), "hi", Some(&everyone), None).is_none());
        assert!(default_envelope(&db, &agent("luna"), "hi", None, Some(&s(&["nova"]))).is_none());
        let threaded = MessageEnvelope {
            thread: Some("other".into()),
            ..Default::default()
        };
        assert!(default_envelope(&db, &agent("luna"), "hi", Some(&threaded), None).is_none());
        // Humans are never rerouted.
        let human = SenderIdentity {
            kind: SenderKind::External,
            ..agent("bigboss")
        };
        assert!(default_envelope(&db, &human, "hi", None, None).is_none());
    }

    #[test]
    fn room_messages_reach_only_members_and_others_are_unchanged() {
        use crate::commands::send::send_message;

        let db = test_db();
        for n in ["luna", "nova", "kira"] {
            add_instance(&db, n);
        }
        join(&db, "auth", "luna").unwrap();
        join(&db, "auth", "nova").unwrap();

        // Room member's plain message: room only.
        let (_, to) = send_message(&db, &agent("luna"), "pushing the fix", None, None).unwrap();
        assert_eq!(to, s(&["nova"]));
        // Non-member's plain message: still a broadcast.
        let (_, mut to) = send_message(&db, &agent("kira"), "lunch?", None, None).unwrap();
        to.sort();
        assert_eq!(to, s(&["luna", "nova"]));
        // Room member can still reach anyone directly, or everyone with --all.
        let (_, to) =
            send_message(&db, &agent("luna"), "@kira want to review?", None, None).unwrap();
        assert_eq!(to, s(&["kira"]));
        let everyone = MessageEnvelope {
            skip_room: true,
            ..Default::default()
        };
        let (_, mut to) =
            send_message(&db, &agent("luna"), "heads up", Some(&everyone), None).unwrap();
        to.sort();
        assert_eq!(to, s(&["kira", "nova"]));
    }

    #[test]
    fn launch_flag_parsing() {
        let (room, rest) =
            take_launch_flag(&s(&["--room", "auth", "--model", "opus", "--", "--room"])).unwrap();
        assert_eq!(room.as_deref(), Some("auth"));
        assert_eq!(rest, s(&["--model", "opus", "--", "--room"]));
        let (room, _) = take_launch_flag(&s(&["--room=docs"])).unwrap();
        assert_eq!(room.as_deref(), Some("docs"));
        assert!(take_launch_flag(&s(&["--room"])).is_err());
        assert!(take_launch_flag(&s(&["--room", "bad name"])).is_err());
        assert_eq!(take_launch_flag(&s(&["-p", "x"])).unwrap().0, None);
    }
}
