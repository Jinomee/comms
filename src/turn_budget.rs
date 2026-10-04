//! Turn budget: stop two agents from messaging each other forever.
//!
//! Every agent→agent message increments a counter for that pair of agents.
//! Once a pair has exchanged `limit` messages with no human input, further
//! messages between them are refused with an explanation. Messages to the
//! human are never counted. Any human message (`hcom send` from a terminal,
//! the TUI) resets every pair; a human typing into an agent's own prompt
//! resets that agent's pairs.
//!
//! Limit: `hcom budget <n>` (stored), else `HCOM_TURN_BUDGET`, else 20.
//! 0 disables. Stored in the `kv` table under `comms_budget:`.

use anyhow::Result;

use crate::db::HcomDb;

const PAIR_PREFIX: &str = "comms_budget:pair:";
const LIMIT_KEY: &str = "comms_budget:limit";
pub const DEFAULT_LIMIT: u32 = 20;
pub const LIMIT_ENV: &str = "HCOM_TURN_BUDGET";

fn pair_key(a: &str, b: &str) -> String {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    format!("{PAIR_PREFIX}{x}|{y}")
}

fn parse_pair(key: &str) -> Option<(String, String)> {
    let (a, b) = key.strip_prefix(PAIR_PREFIX)?.split_once('|')?;
    Some((a.to_string(), b.to_string()))
}

pub fn limit(db: &HcomDb) -> u32 {
    if let Ok(Some(v)) = db.kv_get(LIMIT_KEY)
        && let Ok(n) = v.parse()
    {
        return n;
    }
    std::env::var(LIMIT_ENV)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_LIMIT)
}

pub fn set_limit(db: &HcomDb, n: u32) -> Result<()> {
    db.kv_set(LIMIT_KEY, Some(&n.to_string()))
}

fn count(db: &HcomDb, key: &str) -> u32 {
    db.kv_get(key)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Refusal message if `sender` may not message `recipients` right now.
pub fn check(db: &HcomDb, sender: &str, recipients: &[String]) -> Option<String> {
    let limit = limit(db);
    if limit == 0 {
        return None;
    }
    let exhausted: Vec<&str> = recipients
        .iter()
        .filter(|r| r.as_str() != sender && count(db, &pair_key(sender, r)) >= limit)
        .map(String::as_str)
        .collect();
    if exhausted.is_empty() {
        return None;
    }
    let hcom = crate::runtime_env::build_hcom_command();
    let human = crate::shared::constants::SENDER;
    Some(format!(
        "Turn budget reached: you and {} have exchanged {limit} messages with no human input. \
         Stop here and wait for the human. Tell them where things stand with \
         `{hcom} send @{human} -- <summary>`; they can continue the conversation with \
         `{hcom} budget reset`.",
        exhausted.join(", "),
    ))
}

/// Count a delivered agent→agent message.
pub fn record(db: &HcomDb, sender: &str, recipients: &[String]) {
    for r in recipients.iter().filter(|r| r.as_str() != sender) {
        let key = pair_key(sender, r);
        let next = count(db, &key) + 1;
        if let Err(e) = db.kv_set(&key, Some(&next.to_string())) {
            crate::log::log_warn("turn_budget", "record", &format!("{e}"));
        }
    }
}

/// Human input reached everyone: clear all pairs.
pub fn reset_all(db: &HcomDb) {
    if let Err(e) = db.kv_delete_prefix(PAIR_PREFIX) {
        crate::log::log_warn("turn_budget", "reset_all", &format!("{e}"));
    }
}

/// Human typed into `name`'s prompt: clear pairs involving it.
pub fn reset_for(db: &HcomDb, name: &str) {
    let Ok(rows) = db.kv_prefix(PAIR_PREFIX) else {
        return;
    };
    for (key, _) in rows {
        if parse_pair(&key).is_some_and(|(a, b)| a == name || b == name) {
            let _ = db.kv_set(&key, None);
        }
    }
}

/// (agent, agent, count) for every pair with a nonzero count.
pub fn pairs(db: &HcomDb) -> Vec<(String, String, u32)> {
    let mut out: Vec<_> = db
        .kv_prefix(PAIR_PREFIX)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, value)| {
            let (a, b) = parse_pair(&key)?;
            Some((a, b, value.parse::<u32>().ok()?))
        })
        .collect();
    out.sort_by(|x, y| y.2.cmp(&x.2).then_with(|| (&x.0, &x.1).cmp(&(&y.0, &y.1))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> HcomDb {
        let dir = tempfile::tempdir().unwrap();
        let db = HcomDb::open_raw(&dir.path().join("test.db")).unwrap();
        db.init_db().unwrap();
        std::mem::forget(dir);
        db
    }

    fn v(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pair_exhausts_then_resets() {
        let db = test_db();
        set_limit(&db, 3).unwrap();
        for _ in 0..3 {
            assert!(check(&db, "luna", &v(&["nova"])).is_none());
            record(&db, "luna", &v(&["nova"]));
        }
        // Either direction of the pair is blocked.
        let msg = check(&db, "nova", &v(&["luna"])).unwrap();
        assert!(
            msg.contains("you and luna have exchanged 3 messages"),
            "{msg}"
        );
        // Other pairs are unaffected.
        assert!(check(&db, "luna", &v(&["kira"])).is_none());
        assert!(check(&db, "luna", &v(&["kira", "nova"])).is_some());

        reset_for(&db, "kira");
        assert!(check(&db, "luna", &v(&["nova"])).is_some());
        reset_for(&db, "nova");
        assert!(check(&db, "luna", &v(&["nova"])).is_none());

        for _ in 0..3 {
            record(&db, "luna", &v(&["nova"]));
        }
        reset_all(&db);
        assert!(check(&db, "luna", &v(&["nova"])).is_none());
        assert!(pairs(&db).is_empty());
    }

    #[test]
    fn zero_limit_disables_and_pairs_are_listed() {
        let db = test_db();
        set_limit(&db, 0).unwrap();
        for _ in 0..50 {
            record(&db, "a", &v(&["b", "c"]));
        }
        assert!(check(&db, "a", &v(&["b"])).is_none());
        let listed = pairs(&db);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0], ("a".into(), "b".into(), 50));
    }

    #[test]
    fn send_path_refuses_agents_and_human_resets() {
        use crate::commands::send::send_message;
        use crate::shared::identity::{SenderIdentity, SenderKind};

        let db = test_db();
        for name in ["luna", "nova"] {
            db.conn()
                .execute(
                    "INSERT INTO instances (name, tool, status, status_context, created_at) \
                     VALUES (?, 'claude', 'active', '', 0)",
                    [name],
                )
                .unwrap();
        }
        set_limit(&db, 2).unwrap();
        let agent = |name: &str| SenderIdentity {
            kind: SenderKind::Instance,
            name: name.into(),
            instance_data: None,
            session_id: None,
        };

        send_message(&db, &agent("luna"), "@nova one", None, None).unwrap();
        send_message(&db, &agent("nova"), "@luna two", None, None).unwrap();
        let err = send_message(&db, &agent("luna"), "@nova three", None, None).unwrap_err();
        assert!(err.contains("Turn budget reached"), "{err}");

        let human = SenderIdentity {
            kind: SenderKind::External,
            name: crate::shared::constants::SENDER.into(),
            instance_data: None,
            session_id: None,
        };
        send_message(&db, &human, "@luna keep going", None, None).unwrap();
        send_message(&db, &agent("luna"), "@nova three", None, None).unwrap();
    }

    #[test]
    fn self_messages_are_ignored() {
        let db = test_db();
        set_limit(&db, 1).unwrap();
        record(&db, "a", &v(&["a"]));
        assert!(pairs(&db).is_empty());
        assert!(check(&db, "a", &v(&["a"])).is_none());
    }
}
