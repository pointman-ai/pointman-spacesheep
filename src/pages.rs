//! Which sessions published a page: a page's Session tab talks to the session that deployed it, and every deploy
//! records its session (`spacesheep versions <space> --json`: each version's `pushed_by.session_id`). The spaces come
//! from `spacesheep list --json` (the account's latest, with `id` and `updated_at`), so a page's versions are read
//! again only when it has changed.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

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

/// The sessions that pushed a version, from `versions <space> --json`; None if it isn't that answer.
pub fn sessions(text: &str) -> Option<BTreeSet<String>> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    let versions = v["versions"].as_array()?;
    Some(versions.iter().filter_map(|x| x["pushed_by"]["session_id"].as_str()).map(String::from).collect())
}

/// What's known of each page: when it last changed, and who pushed it. Kept in the state folder (pages.json).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Known(pub BTreeMap<String, (String, BTreeSet<String>)>);

impl Known {
    pub fn from_json(text: &str) -> Known {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let pages = v.as_object().into_iter().flatten().filter_map(|(id, p)| {
            let at = p["updated_at"].as_str()?.to_string();
            let who = p["sessions"].as_array()?.iter().filter_map(Value::as_str).map(String::from).collect();
            Some((id.clone(), (at, who)))
        });
        Known(pages.collect())
    }

    pub fn to_json(&self) -> String {
        let m: serde_json::Map<String, Value> = self
            .0
            .iter()
            .map(|(id, (at, who))| (id.clone(), serde_json::json!({"updated_at": at, "sessions": who})))
            .collect();
        Value::Object(m).to_string()
    }

    /// Whether this page needs its versions read: new, or changed since.
    pub fn stale(&self, s: &Space) -> bool {
        self.0.get(&s.id).is_none_or(|(at, _)| *at != s.updated_at || at.is_empty())
    }

    /// Every session that published one of these pages.
    pub fn publishers(&self) -> BTreeSet<String> {
        self.0.values().flat_map(|(_, who)| who.iter().cloned()).collect()
    }
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
        assert_eq!(sessions(versions).unwrap(), BTreeSet::from(["s-1".to_string(), "s-2".to_string()]));
        assert_eq!(sessions("{}"), None);
    }

    #[test]
    fn a_page_is_read_again_only_when_it_has_changed() {
        let mut known = Known::default();
        let p1 = Space { id: "p1".into(), updated_at: "t1".into() };
        assert!(known.stale(&p1));
        known.0.insert("p1".into(), ("t1".into(), BTreeSet::from(["s-1".to_string()])));
        assert!(!known.stale(&p1));
        assert!(known.stale(&Space { id: "p1".into(), updated_at: "t2".into() }));
        let again = Known::from_json(&known.to_json());
        assert_eq!(again, known);
        assert_eq!(again.publishers(), BTreeSet::from(["s-1".to_string()]));
    }
}
