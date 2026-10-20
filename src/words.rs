//! What the extension posts, and how it says the time.

use std::time::{SystemTime, UNIX_EPOCH};

/// A message's words in a post, at most: the board takes 40,000 characters, and the rest is the post's own.
pub const TEXT_MAX: usize = 30_000;

/// A handle as the board reads a mention (board.py's MENTION): what's posted after the @.
pub fn handle_ok(h: &str) -> bool {
    let mut chars = h.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && h.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A code fence longer than any run of backticks in `text` (three at least), so nothing in it can close it: the
/// board reads no mention inside code (board.py's CODE), and the apps draw it as it was typed.
pub fn fence(text: &str) -> String {
    let (mut run, mut longest) = (0, 0);
    for c in text.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    "`".repeat((longest + 1).max(3))
}

/// The post a message from a page's Session tab becomes: the agent's mention (which wakes it), the words fenced,
/// and how to answer. The reply names the session the page talks to, which is the one listened for, whatever
/// session the agent wakes in.
pub fn post(handle: &str, text: &str, session: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_matches('\n').trim_end();
    let (text, cut) = match text.char_indices().nth(TEXT_MAX) {
        Some((i, _)) => (&text[..i], format!("\n\n(cut: {} more characters)", text[i..].chars().count())),
        None => (text, String::new()),
    };
    let f = fence(text);
    format!(
        "@{handle} from spacesheep.dev (Session tab, not signed):\n\n{f}\n{text}\n{f}{cut}\n\nAnswer here, then send it \
         to the page: `spacesheep talk reply --session {session} \"…\"`"
    )
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A time of day on this machine, `11:14:05`.
pub fn clock(at: f64) -> String {
    let t = at as libc::time_t;
    // SAFETY: a zeroed tm is a valid one, and localtime_r only writes the tm it's given
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    match unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        false => format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec),
        true => format!("{at:.0}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fence_is_longer_than_any_run_of_backticks_inside() {
        assert_eq!(fence("plain"), "```");
        assert_eq!(fence("a `b` c"), "```");
        assert_eq!(fence("```rust\nx\n```"), "````");
        assert_eq!(fence("``````"), "```````");
    }

    #[test]
    fn a_post_mentions_the_agent_fences_the_words_and_names_the_session() {
        let p = post("scene", "hi @james\r\nsee ```x```", "abc-123");
        assert_eq!(
            p,
            "@scene from spacesheep.dev (Session tab, not signed):\n\n````\nhi @james\nsee ```x```\n````\n\nAnswer \
             here, then send it to the page: `spacesheep talk reply --session abc-123 \"…\"`"
        );
    }

    #[test]
    fn long_words_are_cut_outside_the_fence() {
        let p = post("scene", &"x".repeat(TEXT_MAX + 5), "s");
        assert!(p.contains(&format!("{}\n```\n\n(cut: 5 more characters)", "x".repeat(10))), "{}", &p[p.len() - 200..]);
        assert!(p.chars().count() < 40_000);
    }

    #[test]
    fn handles_are_the_boards() {
        assert!(handle_ok("scene") && handle_ok("a1_b-c") && handle_ok("9lives"));
        assert!(
            !handle_ok("")
                && !handle_ok("Scene")
                && !handle_ok("-x")
                && !handle_ok("a b")
                && !handle_ok(&"a".repeat(33))
        );
    }
}
