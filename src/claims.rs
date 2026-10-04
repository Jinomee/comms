//! Claims: soft, expiring locks on paths so agents editing one checkout in
//! parallel don't collide.
//!
//! An agent claims globs (`comms claim "src/auth/**"`); while a claim is live,
//! file-edit hooks deny other agents' edits under it and point them at the
//! holder. Claims expire after a TTL (renewed when the holder edits under
//! them) and are ignored once the holder is gone, so a crashed agent never
//! blocks anyone for long.
//!
//! Stored in the `kv` table under `comms_claim:` (no schema migration, so
//! upstream comms migrations merge cleanly).

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::db::CommsDb;
use crate::shared::constants::ST_INACTIVE;

const KV_PREFIX: &str = "comms_claim:";
pub const DEFAULT_TTL_SECS: i64 = 30 * 60;
/// Sender name for claim notices posted to the holder.
const NOTICE_SENDER: &str = "claims";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub id: String,
    pub holder: String,
    /// Absolute, normalized glob.
    pub pattern: String,
    #[serde(default)]
    pub note: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub ttl_secs: i64,
}

impl Claim {
    pub fn matches(&self, path: &Path) -> bool {
        pattern_matches(&self.pattern, path)
    }
}

/// Outcome of a claim attempt for one pattern.
#[derive(Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    Claimed(Claim),
    Renewed(Claim),
    Conflict { pattern: String, existing: Claim },
}

fn is_glob(s: &str) -> bool {
    s.contains(['*', '?', '[', '{'])
}

/// Resolve `.`/`..` lexically and canonicalize the longest existing ancestor,
/// so a path that doesn't exist yet (a file about to be written) still
/// compares equal to its claimed form (e.g. macOS `/tmp` → `/private/tmp`).
pub fn normalize_path(path: &Path, base: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut lexical = PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    let mut existing = lexical.as_path();
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    loop {
        if let Ok(canon) = existing.canonicalize() {
            let mut out = strip_verbatim(canon);
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return lexical,
        }
    }
}

/// Drop Windows' verbatim prefix from a canonicalized path: `\\?\C:\x` →
/// `C:\x`, `\\?\UNC\srv\share` → `\\srv\share`. Besides reading badly,
/// the `?` would make every claimed path look like a glob.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(rest);
    }
    path
}

/// Turn a user-supplied pattern into an absolute glob. A plain directory
/// (existing, or written with a trailing `/`) claims everything under it.
pub fn normalize_pattern(raw: &str, base: &Path) -> String {
    let trimmed = raw.trim();
    let dir_hint = trimmed.ends_with('/');
    let trimmed = trimmed.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { "." } else { trimmed };

    // Normalize only the literal prefix; keep glob segments verbatim.
    let parts: Vec<&str> = trimmed.split('/').collect();
    let first_glob = parts.iter().position(|p| is_glob(p)).unwrap_or(parts.len());
    let literal = parts[..first_glob].join("/");
    let literal = if literal.is_empty() && trimmed.starts_with('/') {
        "/".to_string()
    } else if literal.is_empty() {
        ".".to_string()
    } else {
        literal
    };
    let mut out = normalize_path(Path::new(&literal), base)
        .to_string_lossy()
        .into_owned();
    for part in &parts[first_glob..] {
        if !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(part);
    }
    if first_glob == parts.len() && (dir_hint || Path::new(&out).is_dir()) {
        if !out.ends_with('/') {
            out.push('/');
        }
        out.push_str("**");
    }
    out
}

fn match_options() -> glob::MatchOptions {
    glob::MatchOptions {
        case_sensitive: !cfg!(any(windows, target_os = "macos")),
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

pub fn pattern_matches(pattern: &str, path: &Path) -> bool {
    if !is_glob(pattern) {
        return Path::new(pattern) == path;
    }
    glob::Pattern::new(pattern)
        .map(|p| p.matches_path_with(path, match_options()))
        .unwrap_or(false)
}

/// Directory prefix before the first glob segment.
fn literal_dir(pattern: &str) -> &str {
    match pattern.find(['*', '?', '[', '{']) {
        Some(i) => &pattern[..pattern[..i].rfind('/').map_or(0, |j| j + 1)],
        None => pattern,
    }
}

/// Could two patterns cover the same file? Exact for literal paths,
/// conservative (directory-prefix) when both are globs.
pub fn patterns_overlap(a: &str, b: &str) -> bool {
    match (is_glob(a), is_glob(b)) {
        (false, false) => a == b,
        (false, true) => pattern_matches(b, Path::new(a)),
        (true, false) => pattern_matches(a, Path::new(b)),
        (true, true) => {
            let (la, lb) = (literal_dir(a), literal_dir(b));
            la.starts_with(lb) || lb.starts_with(la)
        }
    }
}

/// Holder and editor are the same agent, or parent and Claude subagent.
fn related(db: &CommsDb, a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let parent_of = |name: &str| {
        db.get_instance_full(name)
            .ok()
            .flatten()
            .and_then(|row| row.parent_name)
    };
    parent_of(a).as_deref() == Some(b) || parent_of(b).as_deref() == Some(a)
}

fn holder_alive(db: &CommsDb, holder: &str) -> bool {
    if holder == crate::shared::constants::SENDER {
        return true; // the human
    }
    matches!(db.get_instance_full(holder), Ok(Some(row)) if row.status != ST_INACTIVE)
}

fn store(db: &CommsDb, claim: &Claim) -> Result<()> {
    db.kv_set(
        &format!("{KV_PREFIX}{}", claim.id),
        Some(&serde_json::to_string(claim)?),
    )
}

/// Live claims. Expired claims and claims whose holder is gone are deleted;
/// entries that don't parse are left alone (not ours to remove).
pub fn active(db: &CommsDb, now: i64) -> Result<Vec<Claim>> {
    let mut live = Vec::new();
    for (key, value) in db.kv_prefix(KV_PREFIX)? {
        let Ok(claim) = serde_json::from_str::<Claim>(&value) else {
            continue;
        };
        if claim.expires_at > now && holder_alive(db, &claim.holder) {
            live.push(claim);
        } else {
            db.kv_set(&key, None)?;
        }
    }
    live.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    Ok(live)
}

/// Claim each pattern (already normalized) for `holder`.
pub fn claim(
    db: &CommsDb,
    holder: &str,
    patterns: &[String],
    note: &str,
    ttl_secs: i64,
    now: i64,
) -> Result<Vec<ClaimOutcome>> {
    let mut existing = active(db, now)?;
    let mut outcomes = Vec::new();
    for pattern in patterns {
        if let Some(conflict) = existing
            .iter()
            .find(|c| !related(db, &c.holder, holder) && patterns_overlap(&c.pattern, pattern))
        {
            outcomes.push(ClaimOutcome::Conflict {
                pattern: pattern.clone(),
                existing: conflict.clone(),
            });
            continue;
        }
        if let Some(own) = existing
            .iter_mut()
            .find(|c| c.holder == holder && &c.pattern == pattern)
        {
            own.expires_at = now + ttl_secs;
            own.ttl_secs = ttl_secs;
            if !note.is_empty() {
                own.note = note.to_string();
            }
            store(db, own)?;
            outcomes.push(ClaimOutcome::Renewed(own.clone()));
            continue;
        }
        let claim = Claim {
            id: uuid::Uuid::new_v4().simple().to_string()[..12].to_string(),
            holder: holder.to_string(),
            pattern: pattern.clone(),
            note: note.to_string(),
            created_at: now,
            expires_at: now + ttl_secs,
            ttl_secs,
        };
        store(db, &claim)?;
        existing.push(claim.clone());
        outcomes.push(ClaimOutcome::Claimed(claim));
    }
    Ok(outcomes)
}

/// Release `holder`'s claims on the given patterns, or all of them when
/// `patterns` is empty. Returns the released claims.
pub fn release(db: &CommsDb, holder: &str, patterns: &[String], now: i64) -> Result<Vec<Claim>> {
    let mut released = Vec::new();
    for claim in active(db, now)? {
        if claim.holder != holder {
            continue;
        }
        if patterns.is_empty() || patterns.contains(&claim.pattern) {
            db.kv_set(&format!("{KV_PREFIX}{}", claim.id), None)?;
            released.push(claim);
        }
    }
    Ok(released)
}

/// The first live claim held by someone else (not `editor` or its
/// parent/subagent) that covers one of `paths`.
pub fn blocking_claim(
    db: &CommsDb,
    editor: &str,
    paths: &[PathBuf],
    now: i64,
) -> Result<Option<(Claim, PathBuf)>> {
    let claims = active(db, now)?;
    for path in paths {
        if let Some(c) = claims
            .iter()
            .find(|c| c.matches(path) && !related(db, &c.holder, editor))
        {
            return Ok(Some((c.clone(), path.clone())));
        }
    }
    Ok(None)
}

/// Extend `editor`'s own claims covering any of `paths` by their TTL.
pub fn renew_for_edit(db: &CommsDb, editor: &str, paths: &[PathBuf], now: i64) -> Result<()> {
    for mut claim in active(db, now)? {
        if claim.holder == editor && paths.iter().any(|p| claim.matches(p)) {
            claim.expires_at = now + claim.ttl_secs;
            store(db, &claim)?;
        }
    }
    Ok(())
}

pub fn format_remaining(expires_at: i64, now: i64) -> String {
    let secs = (expires_at - now).max(0);
    // Round minutes up so a fresh 10m claim doesn't read "9m left".
    let mins = (secs + 59) / 60;
    if mins >= 60 {
        format!("{}h{}m", mins / 60, mins % 60)
    } else if secs >= 60 {
        format!("{mins}m")
    } else {
        format!("{secs}s")
    }
}

/// Directory paths are shown relative to: the project root (parent of
/// `.comms/`) when inside a `comms init` project, else `cwd`.
pub fn display_base(cwd: &Path) -> PathBuf {
    crate::paths::find_project_comms_dir(cwd)
        .and_then(|data| data.parent()?.parent().map(Path::to_path_buf))
        .map(|root| normalize_path(&root, cwd))
        .unwrap_or_else(|| normalize_path(cwd, cwd))
}

/// Show a claimed pattern relative to `base` when it lives under it.
pub fn display_pattern(pattern: &str, base: &Path) -> String {
    let base = base.to_string_lossy();
    let base = base.trim_end_matches('/');
    pattern
        .strip_prefix(base)
        .and_then(|rest| rest.strip_prefix('/'))
        .map(String::from)
        .unwrap_or_else(|| pattern.to_string())
}

/// Hook-side check: deny reason if `editor` may not write `raw_paths`.
/// Renews the editor's own claims on allowed edits. Also @mentions the
/// holder so it learns someone needs the file. Never fails the hook: any
/// DB error means "allow".
pub fn check_edit(db: &CommsDb, editor: &str, raw_paths: &[&str], cwd: &Path) -> Option<String> {
    if raw_paths.is_empty() {
        return None;
    }
    let now = crate::shared::time::now_epoch_i64();
    let paths: Vec<PathBuf> = raw_paths
        .iter()
        .map(|p| normalize_path(Path::new(p), cwd))
        .collect();
    match blocking_claim(db, editor, &paths, now) {
        Ok(Some((claim, path))) => {
            let shown = display_pattern(&path.to_string_lossy(), &display_base(cwd));
            let note = if claim.note.is_empty() {
                String::new()
            } else {
                format!(" (\"{}\")", claim.note)
            };
            let left = format_remaining(claim.expires_at, now);
            let comms = crate::runtime_env::build_comms_command();
            let _ = db.send_system_message(
                NOTICE_SENDER,
                &format!(
                    "@{} {editor} was blocked from editing {shown}, which you claimed{note}. \
                     Release it with `{comms} release` when you're done, or reply to coordinate.",
                    claim.holder
                ),
            );
            Some(format!(
                "{shown} is claimed by {}{note}, {left} left. Don't edit it yet: ask them first, \
                 e.g. `{comms} send @{} -- can I edit {shown}?`, or work on something else. \
                 See all claims with `{comms} claims`.",
                claim.holder, claim.holder
            ))
        }
        Ok(None) => {
            let _ = renew_for_edit(db, editor, &paths, now);
            None
        }
        Err(e) => {
            crate::log::log_warn("claims", "check_edit", &format!("{e}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_glob_matching() {
        assert!(pattern_matches("/r/src/a.rs", Path::new("/r/src/a.rs")));
        assert!(!pattern_matches("/r/src/a.rs", Path::new("/r/src/b.rs")));
        assert!(pattern_matches("/r/src/**", Path::new("/r/src/a/b/c.rs")));
        assert!(pattern_matches("/r/src/**", Path::new("/r/src/top.rs")));
        assert!(pattern_matches("/r/src/*.rs", Path::new("/r/src/x.rs")));
        assert!(!pattern_matches(
            "/r/src/*.rs",
            Path::new("/r/src/sub/x.rs")
        ));
        assert!(!pattern_matches("/r/src/**", Path::new("/r/srcx/a.rs")));
    }

    #[test]
    fn overlap_rules() {
        assert!(patterns_overlap("/r/a.rs", "/r/a.rs"));
        assert!(!patterns_overlap("/r/a.rs", "/r/b.rs"));
        assert!(patterns_overlap("/r/src/**", "/r/src/a.rs"));
        assert!(!patterns_overlap("/r/src/**", "/r/docs/a.md"));
        assert!(patterns_overlap("/r/src/**", "/r/src/auth/*.rs"));
        assert!(!patterns_overlap("/r/src/**", "/r/test/**"));
    }

    #[test]
    fn normalize_relative_and_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(base.join("src/auth")).unwrap();
        let b = base.to_string_lossy();

        assert_eq!(
            normalize_pattern("src/auth", &base),
            format!("{b}/src/auth/**")
        );
        assert_eq!(
            normalize_pattern("src/new/", &base),
            format!("{b}/src/new/**")
        );
        assert_eq!(
            normalize_pattern("src/**/*.rs", &base),
            format!("{b}/src/**/*.rs")
        );
        assert_eq!(
            normalize_pattern("./src/../a.rs", &base),
            format!("{b}/a.rs")
        );
        assert_eq!(normalize_pattern("*.md", &base), format!("{b}/*.md"));
        // Not-yet-existing file resolves through its existing ancestor.
        assert_eq!(
            normalize_path(Path::new("src/auth/new.rs"), &base),
            base.join("src/auth/new.rs")
        );
    }

    #[test]
    fn verbatim_windows_prefixes_are_stripped() {
        let p = |s: &str| strip_verbatim(PathBuf::from(s));
        assert_eq!(
            p(r"\\?\E:\comms-test\queue.py"),
            PathBuf::from(r"E:\comms-test\queue.py")
        );
        assert_eq!(
            p(r"\\?\UNC\srv\share\a.rs"),
            PathBuf::from(r"\\srv\share\a.rs")
        );
        assert_eq!(p("/home/x/a.rs"), PathBuf::from("/home/x/a.rs"));
        assert_eq!(p(r"\\?\Volume{abc}\a"), PathBuf::from(r"\\?\Volume{abc}\a"));
        assert!(!is_glob(&p(r"\\?\E:\q.py").to_string_lossy()));
    }

    #[test]
    fn display_is_relative_to_base() {
        assert_eq!(display_pattern("/r/src/**", Path::new("/r")), "src/**");
        assert_eq!(display_pattern("/other/x", Path::new("/r")), "/other/x");
    }

    fn test_db() -> CommsDb {
        let dir = tempfile::tempdir().unwrap();
        let db = CommsDb::open_raw(&dir.path().join("test.db")).unwrap();
        db.init_db().unwrap();
        std::mem::forget(dir);
        db
    }

    fn add_instance(db: &CommsDb, name: &str, status: &str, parent: Option<&str>) {
        db.conn()
            .execute(
                "INSERT INTO instances (name, tool, status, parent_name, created_at) VALUES (?, 'claude', ?, ?, 0)",
                rusqlite::params![name, status, parent],
            )
            .unwrap();
    }

    fn pats(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn claim_conflict_renew_release() {
        let db = test_db();
        add_instance(&db, "luna", "active", None);
        add_instance(&db, "nova", "active", None);

        let out = claim(&db, "luna", &pats(&["/r/src/**"]), "refactor", 600, 1000).unwrap();
        assert!(matches!(out[0], ClaimOutcome::Claimed(_)));

        // Overlapping claim by someone else is refused.
        let out = claim(
            &db,
            "nova",
            &pats(&["/r/src/a.rs", "/r/docs/**"]),
            "",
            600,
            1001,
        )
        .unwrap();
        assert!(
            matches!(&out[0], ClaimOutcome::Conflict { existing, .. } if existing.holder == "luna")
        );
        assert!(matches!(out[1], ClaimOutcome::Claimed(_)));

        // Re-claiming your own pattern renews it.
        let out = claim(&db, "luna", &pats(&["/r/src/**"]), "", 900, 1100).unwrap();
        match &out[0] {
            ClaimOutcome::Renewed(c) => {
                assert_eq!(c.expires_at, 2000);
                assert_eq!(c.note, "refactor");
            }
            other => panic!("expected renew, got {other:?}"),
        }

        let blocked = blocking_claim(&db, "nova", &[PathBuf::from("/r/src/x/y.rs")], 1200).unwrap();
        assert_eq!(blocked.unwrap().0.holder, "luna");
        assert!(
            blocking_claim(&db, "luna", &[PathBuf::from("/r/src/x/y.rs")], 1200)
                .unwrap()
                .is_none()
        );

        let released = release(&db, "luna", &[], 1300).unwrap();
        assert_eq!(released.len(), 1);
        assert!(
            blocking_claim(&db, "nova", &[PathBuf::from("/r/src/x/y.rs")], 1300)
                .unwrap()
                .is_none()
        );
        assert_eq!(active(&db, 1300).unwrap().len(), 1); // nova's docs claim
    }

    #[test]
    fn expired_and_dead_holders_are_dropped() {
        let db = test_db();
        add_instance(&db, "luna", "active", None);
        add_instance(&db, "gone", "active", None);
        claim(&db, "luna", &pats(&["/r/a.rs"]), "", 60, 1000).unwrap();
        claim(&db, "gone", &pats(&["/r/b.rs"]), "", 600, 1000).unwrap();
        assert_eq!(active(&db, 1000).unwrap().len(), 2);

        db.conn()
            .execute(
                "UPDATE instances SET status = ? WHERE name = 'gone'",
                [ST_INACTIVE],
            )
            .unwrap();
        let live = active(&db, 1061).unwrap();
        assert!(live.is_empty(), "expired + inactive holder: {live:?}");
        assert!(db.kv_prefix(KV_PREFIX).unwrap().is_empty());
    }

    #[test]
    fn subagent_shares_parent_claims_and_edit_renews() {
        let db = test_db();
        add_instance(&db, "luna", "active", None);
        add_instance(&db, "luna_task_1", "active", Some("luna"));
        add_instance(&db, "nova", "active", None);
        claim(&db, "luna", &pats(&["/r/src/**"]), "", 100, 1000).unwrap();

        let p = [PathBuf::from("/r/src/a.rs")];
        assert!(
            blocking_claim(&db, "luna_task_1", &p, 1010)
                .unwrap()
                .is_none()
        );
        assert!(blocking_claim(&db, "nova", &p, 1010).unwrap().is_some());

        renew_for_edit(&db, "luna", &p, 1050).unwrap();
        assert_eq!(active(&db, 1050).unwrap()[0].expires_at, 1150);
    }

    #[test]
    fn check_edit_denies_and_notifies_holder() {
        let db = test_db();
        add_instance(&db, "luna", "active", None);
        add_instance(&db, "nova", "active", None);
        let now = crate::shared::time::now_epoch_i64();
        claim(&db, "luna", &pats(&["/r/src/**"]), "auth work", 600, now).unwrap();

        let reason = check_edit(&db, "nova", &["src/a.rs"], Path::new("/r")).unwrap();
        assert!(
            reason.contains("src/a.rs is claimed by luna (\"auth work\")"),
            "{reason}"
        );
        assert!(reason.contains("send @luna"));
        assert!(check_edit(&db, "nova", &["docs/x.md"], Path::new("/r")).is_none());
        assert!(check_edit(&db, "luna", &["src/a.rs"], Path::new("/r")).is_none());

        let notices: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM events WHERE type = 'message' AND instance = ? \
                 AND json_extract(data, '$.text') LIKE '@luna nova was blocked%'",
                [format!("sys_{NOTICE_SENDER}")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(notices, 1);
    }

    #[test]
    fn remaining_format() {
        assert_eq!(format_remaining(100, 70), "30s");
        assert_eq!(format_remaining(1000, 100), "15m");
        assert_eq!(format_remaining(8000, 0), "2h14m");
        assert_eq!(format_remaining(599, 0), "10m");
        assert_eq!(format_remaining(0, 10), "0s");
    }
}
