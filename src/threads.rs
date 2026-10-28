//! One Spacesheep comment thread stays under its first Pointman post, including after a restart.

use std::io;
use std::path::Path;

use serde_json::{json, Value};

const MAX: usize = 1000;
const KEEP: u64 = 30 * 24 * 60 * 60;

#[derive(Default)]
pub struct Threads {
    entries: Vec<Entry>,
}

struct Entry {
    thread: i64,
    link: String,
    post: String,
    touched: u64,
}

fn post_ok(post: &str) -> bool {
    post.strip_prefix("post_").and_then(|id| id.parse::<u64>().ok()).is_some_and(|id| id > 0)
}

impl Threads {
    pub fn read(path: &Path, now: u64) -> io::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e),
        };
        let rows: Vec<Value> = serde_json::from_str(&text)?;
        let mut this = Self::default();
        for row in rows {
            let entry =
                match (row["thread"].as_i64(), row["link"].as_str(), row["post"].as_str(), row["touched"].as_u64()) {
                    (Some(thread), Some(link), Some(post), Some(touched))
                        if thread > 0 && link.len() <= 2048 && link.starts_with("https://") && post_ok(post) =>
                    {
                        Entry { thread, link: link.into(), post: post.into(), touched }
                    }
                    _ => continue,
                };
            this.entries.push(entry);
        }
        this.prune(now);
        Ok(this)
    }

    fn prune(&mut self, now: u64) {
        self.entries.retain(|e| now.saturating_sub(e.touched) <= KEEP);
        self.entries.sort_by_key(|e| e.touched);
        if self.entries.len() > MAX {
            self.entries.drain(..self.entries.len() - MAX);
        }
    }

    pub fn root(&self, thread: i64, link: &str, now: u64) -> Option<&str> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.thread == thread && e.link == link && now.saturating_sub(e.touched) <= KEEP)
            .map(|e| e.post.as_str())
    }

    /// Call only after posting succeeds. A failed delivery must never become a thread's root.
    pub fn remember(&mut self, thread: i64, link: &str, post: &str, now: u64) {
        if !post_ok(post) {
            return;
        }
        self.prune(now);
        if let Some(entry) = self.entries.iter_mut().find(|e| e.thread == thread && e.link == link) {
            entry.touched = now;
        } else {
            self.entries.push(Entry { thread, link: link.into(), post: post.into(), touched: now });
        }
        self.prune(now);
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        let rows: Vec<_> = self
            .entries
            .iter()
            .map(|e| json!({"thread": e.thread, "link": e.link, "post": e.post, "touched": e.touched}))
            .collect();
        let temp = path.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec(&rows)?)?;
        std::fs::rename(temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roots_survive_restart_and_stay_separate_by_comment_and_pointman_thread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("comment-threads.json");
        let mut threads = Threads::read(&path, 10).unwrap();
        threads.remember(1, "https://page?comment=a", "post_11", 10);
        threads.remember(1, "https://page?comment=b", "post_12", 10);
        threads.remember(2, "https://page?comment=a", "post_13", 10);
        threads.remember(1, "https://page?comment=a", "post_14", 11);
        threads.save(&path).unwrap();
        let threads = Threads::read(&path, 12).unwrap();
        assert_eq!(threads.root(1, "https://page?comment=a", 12), Some("post_11"));
        assert_eq!(threads.root(1, "https://page?comment=b", 12), Some("post_12"));
        assert_eq!(threads.root(2, "https://page?comment=a", 12), Some("post_13"));
        assert_eq!(threads.root(1, "https://page?comment=a", KEEP + 12), None);
        let mut threads = threads;
        threads.remember(1, "https://page?comment=a", "post_15", KEEP + 12);
        assert_eq!(threads.root(1, "https://page?comment=a", KEEP + 12), Some("post_15"));
    }

    #[test]
    fn storage_keeps_only_the_latest_thousand_threads_and_ignores_invalid_posts() {
        let mut threads = Threads::default();
        for i in 1..=MAX + 1 {
            threads.remember(i as i64, "https://page?comment=a", "post_1", i as u64);
        }
        assert_eq!(threads.entries.len(), MAX);
        assert_eq!(threads.root(1, "https://page?comment=a", 1002), None);
        threads.remember(1, "https://page?comment=a", "garbage", 1002);
        assert_eq!(threads.entries.len(), MAX);
        threads.prune(KEEP + 2000);
        assert!(threads.entries.is_empty());
    }
}
