//! Which sessions published a page: a page's Session tab talks to the session that deployed it, and every deploy
//! records its session (`spacesheep versions <space> --json`: each version's `client.session_id`, or today's
//! `pushed_by.session_id`). The spaces come from `spacesheep list --json` (the account's latest, with `id` and
//! `updated_at`: a bare array today, `{spaces, page}` from the new API), so a page's versions are read
//! again only when it has changed. That list is only the latest 50, so a page that leaves it is kept a while: it
//! hasn't changed since, so who published it still holds (EXT-75).

use std::collections::BTreeMap;

use serde_json::Value;

/// How long a page that has left the latest list is kept, from its last change, and how many pages are kept in all
/// (Pointman EXT-75: one session updating 48 pages in 20 minutes pushed every other agent's pages out of the 50).
pub const KEEP_DAYS: i64 = 30;
pub const KEEP_MAX: usize = 500;

/// A space as `list --json` gives it: its id, and when it last changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub id: String,
    pub updated_at: String,
}

/// The spaces in `list --json`'s answer (an array, or `{spaces: […]}`); None if it isn't one.
pub fn spaces(text: &str) -> Option<Vec<Space>> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let rows = v.as_array().or_else(|| v["spaces"].as_array())?;
    Some(
        rows.iter()
            .filter_map(|r| {
                let id = r["id"].as_str().or_else(|| r["uuid"].as_str())?;
                Some(Space { id: id.to_string(), updated_at: r["updated_at"].as_str().unwrap_or("").to_string() })
            })
            .collect(),
    )
}

/// An answer's cursor to its next page: `page.next`, a string (empty on the last page). Outer None: the cursor is
/// malformed (not a string, or `page` not an object), so the page can't be told from a last one and the read must
/// fail rather than keep part of the history as all of it.  `versions list` always carries it.
fn cursor(v: &Value) -> Option<Option<String>> {
    match v.get("page") {
        Some(Value::Object(p)) => match p.get("next") {
            Some(Value::String(s)) => Some(Some(s.clone()).filter(|n| !n.is_empty())),
            _ => None,
        },
        _ => None,
    }
}

/// One answer of `versions list --space-id <id> --json`: its versions, and the cursor to the next page
/// (`page.next`, empty on the last page). None if it isn't one.
pub fn versions_page(text: &str) -> Option<(Vec<Value>, Option<String>)> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let versions = v["versions"].as_array()?.clone();
    let next = cursor(&v)?;
    Some((versions, next))
}

/// The sessions that pushed a page's versions (every page of `versions <space> --json`, in the order read), the
/// first version's first, each once with when its first version was pushed (`created_at`, "" when it's not given).
///
/// Two shapes: today's numbers each version and names its session in `pushed_by.session_id`. Spacesheep's new API
/// (ListVersions) lists them newest first with no number, and names it in `client.session_id`, read first. Either
/// is set only on the reading key's own account's versions, so an empty one is someone else's and is skipped.
pub fn sessions(versions: &[Value]) -> Vec<(String, String)> {
    let mut versions: Vec<&Value> = versions.iter().collect();
    if versions.iter().any(|x| x["number"].is_i64()) {
        versions.sort_by_key(|x| x["number"].as_i64().unwrap_or(i64::MAX));
    } else {
        versions.reverse();
        versions.sort_by_key(|x| x["created_at"].as_str().and_then(secs).unwrap_or(i64::MAX));
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for x in versions {
        let named = [&x["client"]["session_id"], &x["pushed_by"]["session_id"]];
        let Some(s) = named.into_iter().filter_map(Value::as_str).find(|s| !s.is_empty()) else { continue };
        if !out.iter().any(|(o, _)| o == s) {
            out.push((s.to_string(), x["created_at"].as_str().unwrap_or("").to_string()));
        }
    }
    out
}

/// Where this node has seen each of its sessions' agents run, for a page's own thread (EXT-128).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Seen {
    /// Each session's threads as the looks saw them, oldest first: from when (seconds since 1970), which thread.
    pub history: BTreeMap<String, Vec<(i64, i64)>>,
    /// Each session's last thread known (agents.json): for one not seen since 0.8.2 began keeping its history.
    pub latest: BTreeMap<String, i64>,
}

/// How long before a session was first seen a push still counts in that first thread: a look sees a new session
/// within a minute, so a page pushed straight after its agent woke is its first thread's.
pub const FIRST_SEEN_GRACE: i64 = 120;

impl Seen {
    /// Whether this node has seen this session (its own agents' only).
    pub fn knows(&self, session: &str) -> bool {
        self.history.get(session).is_some_and(|h| !h.is_empty()) || self.latest.get(session).is_some_and(|t| *t > 0)
    }

    /// The thread this session's agent was running in when it pushed (`at`, Spacesheep's `created_at`): the last one
    /// seen from then or before, or the first seen for a push up to FIRST_SEEN_GRACE before it (or with no time).
    /// None for an earlier push: where it ran then wasn't seen, so nothing is pinned (shipwright's review). A session
    /// with no history (seen before 0.8.2 kept it, and not running since) has its last known thread.
    pub fn thread_at(&self, session: &str, at: &str) -> Option<i64> {
        let thread = match self.history.get(session).filter(|h| !h.is_empty()) {
            Some(h) => match secs(at) {
                Some(at) if at < h[0].0 - FIRST_SEEN_GRACE => return None,
                Some(at) => h.iter().rev().find(|(since, _)| *since <= at).unwrap_or(&h[0]).1,
                None => h[0].1,
            },
            None => *self.latest.get(session)?,
        };
        Some(thread).filter(|t| *t > 0)
    }
}

/// What's known of a page: when it last changed, the sessions that pushed it (the first version's first), and the
/// thread it was published from (EXT-128).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub updated_at: String,
    pub sessions: Vec<String>,
    /// When each session's first version was pushed (Spacesheep's `created_at`).
    pub pushed: BTreeMap<String, String>,
    /// Whether `sessions` is in the versions' order: a page kept before 0.8.2 isn't, so its versions are read again
    /// while it's in the latest list.
    pub ordered: bool,
    /// The thread of the first session here that pushed it, as that session's agent's run was in (0: not known yet).
    /// It's set once, so a page's messages stay in its own thread when its agent works elsewhere later.
    pub thread: i64,
}

impl Page {
    pub fn new(updated_at: &str, sessions: Vec<(String, String)>) -> Page {
        let pushed = sessions.iter().filter(|(_, at)| !at.is_empty()).cloned().collect();
        let sessions = sessions.into_iter().map(|(s, _)| s).collect();
        Page { updated_at: updated_at.to_string(), sessions, pushed, ordered: true, thread: 0 }
    }
}

/// What's known of each page, by its id. Kept in the state folder (pages.json).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Known(pub BTreeMap<String, Page>);

impl Known {
    pub fn from_json(text: &str) -> Known {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let pages = v.as_object().into_iter().flatten().filter_map(|(id, p)| {
            let updated_at = p["updated_at"].as_str()?.to_string();
            let sessions = p["sessions"].as_array()?.iter().filter_map(Value::as_str).map(String::from).collect();
            // `thread` came with the sessions' order: without it, the order isn't known
            let thread = p["thread"].as_i64();
            let pushed = p["pushed"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(s, at)| Some((s.clone(), at.as_str()?.to_string())))
                .collect();
            let (ordered, thread) = (thread.is_some(), thread.unwrap_or(0));
            Some((id.clone(), Page { updated_at, sessions, pushed, ordered, thread }))
        });
        Known(pages.collect())
    }

    pub fn to_json(&self) -> String {
        let m: serde_json::Map<String, Value> = self
            .0
            .iter()
            .map(|(id, p)| {
                let mut page = serde_json::json!({"updated_at": p.updated_at, "sessions": p.sessions});
                if p.ordered {
                    page["thread"] = p.thread.into();
                    page["pushed"] = serde_json::json!(p.pushed);
                }
                (id.clone(), page)
            })
            .collect();
        Value::Object(m).to_string()
    }

    /// What's kept after reading the latest list: every page in it, and each that has left it but changed in the last
    /// KEEP_DAYS days (`now`: seconds since 1970); the KEEP_MAX most recently changed in all.
    pub fn keep(&mut self, list: &[Space], now: i64) {
        let since = iso(now - KEEP_DAYS * 86_400);
        self.0.retain(|id, p| {
            list.iter().any(|s| &s.id == id) || p.updated_at.get(..since.len()).is_some_and(|at| at >= since.as_str())
        });
        if self.0.len() > KEEP_MAX {
            let mut newest: Vec<(String, String)> =
                self.0.iter().map(|(id, p)| (p.updated_at.clone(), id.clone())).collect();
            newest.sort_by(|a, b| b.cmp(a));
            for (_, id) in newest.into_iter().skip(KEEP_MAX) {
                self.0.remove(&id);
            }
        }
    }

    /// Whether this page needs its versions read: new, changed since, or kept without its sessions' order.
    pub fn stale(&self, s: &Space) -> bool {
        self.0.get(&s.id).is_none_or(|p| p.updated_at != s.updated_at || p.updated_at.is_empty() || !p.ordered)
    }

    /// Its versions read again: its new sessions, the thread it was published from kept.
    pub fn read(&mut self, s: &Space, sessions: Vec<(String, String)>) {
        let thread = self.0.get(&s.id).map(|p| p.thread).unwrap_or(0);
        self.0.insert(s.id.clone(), Page { thread, ..Page::new(&s.updated_at, sessions) });
    }

    /// Each page without its thread yet gets the thread the first session here that pushed it was running in when
    /// it did ([`Seen::thread_at`]), and none if that isn't known: a later session's thread isn't the page's. The
    /// pages that got one, with it. A page that gets none, or whose sessions' order isn't known, has its messages go
    /// to its agent's latest thread, as before 0.8.2.
    pub fn settle(&mut self, seen: &Seen) -> Vec<(String, i64)> {
        let mut settled = Vec::new();
        for (id, p) in self.0.iter_mut().filter(|(_, p)| p.thread == 0 && p.ordered) {
            let at = |s: &str| p.pushed.get(s).map(String::as_str).unwrap_or("");
            let first = p.sessions.iter().find(|s| seen.knows(s));
            if let Some(t) = first.and_then(|s| seen.thread_at(s, at(s))) {
                p.thread = t;
                settled.push((id.clone(), t));
            }
        }
        settled
    }

    /// Each page's thread, where it's known.
    pub fn threads(&self) -> BTreeMap<String, i64> {
        self.0.iter().filter(|(_, p)| p.thread > 0).map(|(id, p)| (id.clone(), p.thread)).collect()
    }

    /// Every session that published one of these pages, the most recently changed page's first, and in each page the
    /// latest version's first (the session its Session tab talks to), each once.
    pub fn publishers(&self) -> Vec<String> {
        let mut pages: Vec<&Page> = self.0.values().collect();
        pages.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let mut out: Vec<String> = Vec::new();
        for s in pages.into_iter().flat_map(|p| p.sessions.iter().rev()) {
            if !out.contains(s) {
                out.push(s.clone());
            }
        }
        out
    }
}

/// Seconds since 1970 as a UTC time to the second, the way Spacesheep's `updated_at` starts: "2026-10-23T08:03:10".
pub fn iso(secs: i64) -> String {
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // days to a civil date (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let (era, doe) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// Spacesheep's time ("2026-11-04T08:05:08.589Z") as seconds since 1970, to the second; None if it isn't one.
pub fn secs(at: &str) -> Option<i64> {
    let num = |r: std::ops::Range<usize>| at.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hh, mm, ss) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if at.get(4..5) != Some("-") || at.get(10..11) != Some("T") || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // a civil date to days (Howard Hinnant's algorithm), the other way from iso()
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400 + hh * 3600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sessions in one answer, a page whose versions all came in one call.
    fn read(text: &str) -> Vec<(String, String)> {
        let v: serde_json::Value = serde_json::from_str(text).unwrap();
        sessions(v["versions"].as_array().unwrap())
    }

    #[test]
    fn spaces_and_their_publishers_from_the_clis_answers() {
        let list = r#"[{"id": "p1", "updated_at": "2026-10-20T13:27:34Z", "title": "a"}, {"id": "p2", "updated_at": "t2"}, {"title": "no id"}]"#;
        let got = spaces(list).unwrap();
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["p1", "p2"]);
        assert_eq!(spaces("not json"), None);
        let versions = r#"{"space": {"uuid": "p1"}, "versions": [
            {"number": 3, "created_at": "2026-11-04T09:00:00.000Z", "pushed_by": {"session_id": "s-1"}},
            {"number": 2, "created_at": "2026-11-04T08:06:16.089Z", "pushed_by": {"label": "Claude Code", "session_id": "s-2"}},
            {"number": 1, "created_at": "2026-11-04T08:05:08.589Z", "pushed_by": {"label": "Claude Code", "session_id": "s-1"}},
            {"number": 0, "via": "web"}]}"#;
        let got = read(versions);
        let want = [("s-1", "2026-11-04T08:05:08.589Z"), ("s-2", "2026-11-04T08:06:16.089Z")];
        assert_eq!(got, want.map(|(s, at)| (s.to_string(), at.to_string())), "the first version's first, at its first");
        assert_eq!(versions_page("{}"), None);
    }

    #[test]
    fn the_new_apis_answers_name_the_same_spaces_and_sessions() {
        let list = r#"{"spaces": [{"id": "p1", "name": "@james/a", "updated_at": "2026-11-05T10:42:00Z"},
            {"id": "p2", "updated_at": "2026-11-05T10:41:59.120Z"}], "page": {"next": "50"}}"#;
        let got = spaces(list).unwrap();
        let want = [("p1", "2026-11-05T10:42:00Z"), ("p2", "2026-11-05T10:41:59.120Z")];
        assert_eq!(got.iter().map(|s| (s.id.as_str(), s.updated_at.as_str())).collect::<Vec<_>>(), want);
        // ListVersions: newest first, no number; on someone else's version `client` has only `transport` and `other`
        let versions = r#"{"versions": [
            {"id": "c4", "created_at": "2026-11-04T09:00:00Z", "client": {"name": "claude-code", "session_id": "s-1"}},
            {"id": "c3", "created_at": "2026-11-04T08:30:00Z", "client": {"transport": "mcp", "other": "agent"}},
            {"id": "c2", "created_at": "2026-11-04T08:06:16.089Z", "client": {"session_id": "s-2"}},
            {"id": "c1", "created_at": "2026-11-04T08:05:08.589Z", "client": {"session_id": "s-1"}},
            {"id": "c0", "created_at": "2026-11-04T08:00:00Z", "client": {"session_id": ""}}], "page": {"next": ""}}"#;
        let got = read(versions);
        let want = [("s-1", "2026-11-04T08:05:08.589Z"), ("s-2", "2026-11-04T08:06:16.089Z")];
        assert_eq!(got, want.map(|(s, at)| (s.to_string(), at.to_string())), "the first version's first, at its first");
    }

    #[test]
    fn a_page_of_versions_names_the_next_and_the_older_pages_first_push_comes_first() {
        // the newest page: s-2's version, and a cursor to the older ones
        let newest = r#"{"versions": [
            {"id": "c3", "created_at": "2026-11-05T10:00:00Z", "client": {"session_id": "s-2"}},
            {"id": "c2", "created_at": "2026-11-05T09:00:00Z", "client": {"transport": "mcp"}}], "page": {"next": "c2"}}"#;
        let (mut versions, next) = versions_page(newest).unwrap();
        assert_eq!(next.as_deref(), Some("c2"));
        // the oldest: s-1 pushed the page first, within the same second as s-2's first push on it
        let oldest = r#"{"versions": [
            {"id": "c1", "created_at": "2026-11-04T08:05:08Z", "client": {"session_id": "s-2"}},
            {"id": "c0", "created_at": "2026-11-04T08:05:08Z", "client": {"session_id": "s-1"}}], "page": {"next": ""}}"#;
        let (older, next) = versions_page(oldest).unwrap();
        assert_eq!(next, None, "an empty next is the last page");
        versions.extend(older);
        let got = sessions(&versions);
        let want = [("s-1", "2026-11-04T08:05:08Z"), ("s-2", "2026-11-04T08:05:08Z")];
        assert_eq!(got, want.map(|(s, at)| (s.to_string(), at.to_string())), "the oldest page's first push first");
    }

    #[test]
    fn a_malformed_cursor_is_not_a_last_page() {
        // answers always carry `page.next` as a string; anything else must fail the read, not end it
        for page in [
            r#""page": {"next": 123}"#,
            r#""page": {"next": null}"#,
            r#""page": {}"#,
            r#""page": null"#,
            r#""page": "50""#,
        ] {
            let v = format!(r#"{{"versions": [{{"id": "c1"}}], {page}}}"#);
            assert_eq!(versions_page(&v), None, "{page}");
        }
        // no `page` at all is no last page
        assert_eq!(versions_page(r#"{"versions": []}"#), None);
        // the codec's empty collection
        assert_eq!(versions_page(r#"{"versions": [], "page": {"next": ""}}"#), Some((vec![], None)));
    }

    #[test]
    fn client_session_is_read_first_and_an_empty_one_falls_back_to_pushed_by() {
        let versions = r#"{"versions": [
            {"number": 2, "client": {"session_id": "s-new"}, "pushed_by": {"session_id": "s-old"}},
            {"number": 1, "client": {"session_id": ""}, "pushed_by": {"session_id": "s-3"}},
            {"number": 3, "pushed_by": {"session_id": ""}}]}"#;
        let got: Vec<String> = read(versions).into_iter().map(|(s, _)| s).collect();
        assert_eq!(got, ["s-3", "s-new"], "an empty session is someone else's: skipped");
    }

    fn who(sessions: &[&str]) -> Vec<(String, String)> {
        sessions.iter().map(|s| (s.to_string(), String::new())).collect()
    }

    #[test]
    fn a_page_is_read_again_only_when_it_has_changed() {
        let mut known = Known::default();
        let p1 = Space { id: "p1".into(), updated_at: "t1".into() };
        assert!(known.stale(&p1));
        known.read(&p1, who(&["s-1"]));
        assert!(!known.stale(&p1));
        assert!(known.stale(&Space { id: "p1".into(), updated_at: "t2".into() }));
        let again = Known::from_json(&known.to_json());
        assert_eq!(again, known);
        assert_eq!(again.publishers(), ["s-1"]);
        known.read(&Space { id: "p2".into(), updated_at: "t9".into() }, who(&["s-2", "s-1"]));
        assert_eq!(known.publishers(), ["s-1", "s-2"], "the newest page's sessions first, its latest first, each once");
    }

    #[test]
    fn spacesheeps_times_in_seconds() {
        assert_eq!(secs("2026-10-23T08:23:10"), Some(1_792_743_790));
        assert_eq!(secs("2026-11-04T08:05:08.589Z"), Some(1_793_779_508));
        for t in [0, 951_782_400, 1_700_000_000, 1_792_743_790] {
            assert_eq!(secs(&iso(t)), Some(t));
        }
        assert_eq!(secs(""), None);
        assert_eq!(secs("t1"), None);
        assert_eq!(secs("2026-13-04T08:05:08Z"), None);
    }

    #[test]
    fn a_session_pushed_from_the_thread_it_was_running_in_then() {
        let at = |t: &str| secs(t).unwrap();
        let seen = Seen {
            history: BTreeMap::from([(
                "s-1".to_string(),
                vec![(at("2026-11-04T08:00:00Z"), 19090), (at("2026-11-04T09:20:00Z"), 25460)],
            )]),
            latest: BTreeMap::from([("s-1".to_string(), 25460), ("old".to_string(), 4159), ("none".to_string(), 0)]),
        };
        assert_eq!(seen.thread_at("s-1", "2026-11-04T08:05:08.589Z"), Some(19090), "where it was then");
        assert_eq!(seen.thread_at("s-1", "2026-11-04T09:23:38Z"), Some(25460));
        assert_eq!(
            seen.thread_at("s-1", "2026-11-04T07:58:30Z"),
            Some(19090),
            "just before it was seen: the first seen"
        );
        assert_eq!(seen.thread_at("s-1", "2026-11-04T07:50:00Z"), None, "long before it was seen: not known");
        assert_eq!(seen.thread_at("s-1", ""), Some(19090), "no time: the first seen");
        assert_eq!(seen.thread_at("old", "2026-11-04T08:05:08Z"), Some(4159), "no history: its last known");
        assert_eq!(seen.thread_at("none", ""), None);
        assert_eq!(seen.thread_at("elsewhere", ""), None);
        assert!(seen.knows("s-1") && seen.knows("old") && !seen.knows("none") && !seen.knows("elsewhere"));
    }

    #[test]
    fn a_page_keeps_the_thread_it_was_published_from() {
        let seen = |h: Vec<(&str, Vec<(i64, i64)>)>| Seen {
            history: h.into_iter().map(|(s, t)| (s.to_string(), t)).collect(),
            latest: BTreeMap::new(),
        };
        let p1 = Space { id: "p1".into(), updated_at: "t1".into() };
        let pushed = |s: &str, t: i64| (s.to_string(), iso(t));
        let mut known = Known::default();
        // published by a session on another machine first, then by s-1 here at 100, while it ran in thread 19090
        known.read(&p1, vec![pushed("elsewhere", 50), pushed("s-1", 100), pushed("s-2", 300)]);
        assert_eq!(known.settle(&seen(vec![])), [], "no session's thread known yet");
        assert_eq!(known.threads(), BTreeMap::new());
        let moved = vec![(90, 19090), (200, 25460)];
        let settled = known.settle(&seen(vec![("s-1", moved), ("s-2", vec![(290, 25460)])]));
        assert_eq!(settled, [("p1".to_string(), 19090)], "where s-1 ran when it pushed, not where it is now");
        // a page pushed long before its first session was seen: not pinned, even though a later session's thread is
        let p2 = Space { id: "p2".into(), updated_at: "t1".into() };
        known.read(&p2, vec![pushed("s-1", 10), pushed("s-2", 300)]);
        assert_eq!(known.settle(&seen(vec![("s-1", vec![(1000, 19090)]), ("s-2", vec![(290, 25460)])])), []);
        known.0.remove("p2");
        // its agent works in another thread now, and pushes the page again from there: it stays in its own
        known.read(&Space { id: "p1".into(), updated_at: "t2".into() }, vec![pushed("s-1", 100), pushed("s-3", 400)]);
        assert_eq!(known.settle(&seen(vec![("s-1", vec![(200, 25460)]), ("s-3", vec![(390, 25460)])])), []);
        assert_eq!(known.threads(), BTreeMap::from([("p1".to_string(), 19090)]));
        let again = Known::from_json(&known.to_json());
        assert_eq!(again, known);

        // a page kept before 0.8.2: its sessions still count, its versions are read again, and only then it settles
        let old = Known::from_json(r#"{"p1": {"updated_at": "t1", "sessions": ["s-2", "s-1"]}}"#);
        assert_eq!(old.publishers(), ["s-1", "s-2"]);
        assert!(old.stale(&p1), "read again for the order");
        let mut old = old;
        let latest = Seen {
            latest: BTreeMap::from([("s-1".to_string(), 19090), ("s-2".to_string(), 25460)]),
            ..Seen::default()
        };
        assert_eq!(old.settle(&latest), [], "not in the versions' order");
        assert_eq!(Known::from_json(&old.to_json()), old, "still without the order");
        old.read(&p1, vec![pushed("s-1", 100), pushed("s-2", 300)]);
        assert!(!old.stale(&p1));
        assert_eq!(
            old.settle(&latest),
            [("p1".to_string(), 19090)],
            "a session with no history: its last known thread"
        );
    }

    #[test]
    fn a_page_that_leaves_the_latest_list_is_kept_for_thirty_days() {
        assert_eq!(iso(0), "1970-01-01T00:00:00");
        assert_eq!(iso(1_700_000_000), "2023-11-14T22:13:20");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00");
        let now = 1_792_743_790; // 2026-10-23T08:23:10
        assert_eq!(iso(now), "2026-10-23T08:23:10");
        let page = |at: &str, who: &str| Page::new(at, vec![(who.to_string(), String::new())]);
        let mut known = Known(BTreeMap::from([
            ("listed".to_string(), page("2026-08-01T00:00:00Z", "s-1")),
            ("left".to_string(), page("2026-10-21T09:00:00.120Z", "s-2")),
            ("edge".to_string(), page("2026-09-23T08:23:10Z", "s-3")),
            ("old".to_string(), page("2026-09-23T08:23:09.999Z", "s-4")),
            ("undated".to_string(), page("", "s-5")),
        ]));
        known.keep(&[Space { id: "listed".into(), updated_at: "2026-08-01T00:00:00Z".into() }], now);
        assert_eq!(known.0.keys().collect::<Vec<_>>(), ["edge", "left", "listed"], "a listed page stays however old");

        let mut many =
            Known((0..KEEP_MAX + 3).map(|i| (format!("p{i:04}"), page(&iso(now - i as i64), "s"))).collect());
        many.keep(&[], now);
        assert_eq!(many.0.len(), KEEP_MAX);
        assert!(many.0.contains_key("p0000") && !many.0.contains_key(&format!("p{:04}", KEEP_MAX)), "the newest kept");
    }
}
