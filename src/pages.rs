//! Which sessions published a page: a page's Session tab talks to the session that deployed it, and every deploy
//! records its session (`spacesheep versions <space> --json`: each version's `pushed_by.session_id`). The spaces come
//! from `spacesheep list --json` (the account's latest, with `id` and `updated_at`), so a page's versions are read
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

/// The sessions that pushed a version, from `versions <space> --json`, the first version's first, each once; None if
/// it isn't that answer.
pub fn sessions(text: &str) -> Option<Vec<String>> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let mut versions: Vec<&Value> = v["versions"].as_array()?.iter().collect();
    versions.sort_by_key(|x| x["number"].as_i64().unwrap_or(i64::MAX));
    let mut out: Vec<String> = Vec::new();
    for s in versions.into_iter().filter_map(|x| x["pushed_by"]["session_id"].as_str()) {
        if !out.iter().any(|o| o == s) {
            out.push(s.to_string());
        }
    }
    Some(out)
}

/// What's known of a page: when it last changed, the sessions that pushed it (the first version's first), and the
/// thread it was published from (EXT-128).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    pub updated_at: String,
    pub sessions: Vec<String>,
    /// Whether `sessions` is in the versions' order: a page kept before 0.8.2 isn't, so its versions are read again
    /// while it's in the latest list.
    pub ordered: bool,
    /// The thread of the first session here that pushed it, as that session's agent's run was in (0: not known yet).
    /// It's set once, so a page's messages stay in its own thread when its agent works elsewhere later.
    pub thread: i64,
}

impl Page {
    pub fn new(updated_at: &str, sessions: Vec<String>) -> Page {
        Page { updated_at: updated_at.to_string(), sessions, ordered: true, thread: 0 }
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
            Some((id.clone(), Page { updated_at, sessions, ordered: thread.is_some(), thread: thread.unwrap_or(0) }))
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
    pub fn read(&mut self, s: &Space, sessions: Vec<String>) {
        let thread = self.0.get(&s.id).map(|p| p.thread).unwrap_or(0);
        self.0.insert(s.id.clone(), Page { thread, ..Page::new(&s.updated_at, sessions) });
    }

    /// Each page without its thread yet gets the thread of the first session that pushed it whose thread is known
    /// (`threads`: each session here, its agent's run's thread). The pages that got one, with it. A page whose
    /// sessions' order isn't known gets none: its messages go to its agent's latest thread, as before 0.8.2.
    pub fn settle(&mut self, threads: &BTreeMap<String, i64>) -> Vec<(String, i64)> {
        let mut settled = Vec::new();
        for (id, p) in self.0.iter_mut().filter(|(_, p)| p.thread == 0 && p.ordered) {
            if let Some(t) = p.sessions.iter().find_map(|s| threads.get(s).copied().filter(|t| *t > 0)) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_and_their_publishers_from_the_clis_answers() {
        let list = r#"[{"id": "p1", "updated_at": "2026-10-20T13:27:34Z", "title": "a"}, {"id": "p2", "updated_at": "t2"}, {"title": "no id"}]"#;
        let got = spaces(list).unwrap();
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["p1", "p2"]);
        assert_eq!(spaces("not json"), None);
        let versions = r#"{"space": {"uuid": "p1"}, "versions": [
            {"number": 2, "pushed_by": {"label": "Claude Code", "session_id": "s-2"}},
            {"number": 1, "pushed_by": {"label": "Claude Code", "session_id": "s-1"}},
            {"number": 0, "via": "web"}]}"#;
        assert_eq!(sessions(versions).unwrap(), ["s-1", "s-2"], "the first version's first");
        assert_eq!(sessions("{}"), None);
    }

    #[test]
    fn a_page_is_read_again_only_when_it_has_changed() {
        let mut known = Known::default();
        let p1 = Space { id: "p1".into(), updated_at: "t1".into() };
        assert!(known.stale(&p1));
        known.read(&p1, vec!["s-1".into()]);
        assert!(!known.stale(&p1));
        assert!(known.stale(&Space { id: "p1".into(), updated_at: "t2".into() }));
        let again = Known::from_json(&known.to_json());
        assert_eq!(again, known);
        assert_eq!(again.publishers(), ["s-1"]);
        known.read(&Space { id: "p2".into(), updated_at: "t9".into() }, vec!["s-2".into(), "s-1".into()]);
        assert_eq!(known.publishers(), ["s-1", "s-2"], "the newest page's sessions first, its latest first, each once");
    }

    #[test]
    fn a_page_keeps_the_thread_it_was_published_from() {
        let threads = |t: &[(&str, i64)]| t.iter().map(|(s, t)| (s.to_string(), *t)).collect::<BTreeMap<_, _>>();
        let p1 = Space { id: "p1".into(), updated_at: "t1".into() };
        let mut known = Known::default();
        // published by a session on another machine first, then by s-1 here, in thread 19090
        known.read(&p1, vec!["elsewhere".into(), "s-1".into(), "s-2".into()]);
        assert_eq!(known.settle(&threads(&[])), [], "no session's thread known yet");
        assert_eq!(known.threads(), BTreeMap::new());
        assert_eq!(known.settle(&threads(&[("s-2", 25460), ("s-1", 19090)])), [("p1".to_string(), 19090)]);
        // its agent works in another thread now, and pushes the page again from there: it stays in its own
        known.read(&Space { id: "p1".into(), updated_at: "t2".into() }, vec!["s-1".into(), "s-2".into(), "s-3".into()]);
        assert_eq!(known.settle(&threads(&[("s-1", 25460), ("s-3", 25460)])), []);
        assert_eq!(known.threads(), BTreeMap::from([("p1".to_string(), 19090)]));
        let again = Known::from_json(&known.to_json());
        assert_eq!(again, known);

        // a page kept before 0.8.2: its sessions still count, its versions are read again, and only then it settles
        let old = Known::from_json(r#"{"p1": {"updated_at": "t1", "sessions": ["s-2", "s-1"]}}"#);
        assert_eq!(old.publishers(), ["s-1", "s-2"]);
        assert!(old.stale(&p1), "read again for the order");
        let mut old = old;
        assert_eq!(old.settle(&threads(&[("s-1", 19090)])), [], "not in the versions' order");
        assert_eq!(Known::from_json(&old.to_json()), old, "still without the order");
        old.read(&p1, vec!["s-1".into(), "s-2".into()]);
        assert!(!old.stale(&p1));
        assert_eq!(old.settle(&threads(&[("s-1", 19090), ("s-2", 25460)])), [("p1".to_string(), 19090)]);
    }

    #[test]
    fn a_page_that_leaves_the_latest_list_is_kept_for_thirty_days() {
        assert_eq!(iso(0), "1970-01-01T00:00:00");
        assert_eq!(iso(1_700_000_000), "2023-11-14T22:13:20");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00");
        let now = 1_792_743_790; // 2026-10-23T08:23:10
        assert_eq!(iso(now), "2026-10-23T08:23:10");
        let page = |at: &str, who: &str| Page::new(at, vec![who.to_string()]);
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
