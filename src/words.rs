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

/// The agents a message calls by starting with their handles (`@editor cut the intro`, `@editor, @scene: …`), each
/// once, lowercased. `@pm.editor` and `@pointman.editor` call @editor too: in a page's comments a bare `@editor` would also be a mention of
/// a Spacesheep user called editor (James, Pointman thread 6914 [16565]). A handle later in the message calls nobody,
/// and neither does one that isn't addressing (`@editor's cut`).
pub fn called(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text.trim_start();
    while let Some(after) = rest.strip_prefix('@') {
        // Pointman's prefixes, `@pm.editor` or `@pointman.editor`: the rest is the handle
        let after = ["pm.", "pointman."]
            .iter()
            .find_map(|p| after.get(..p.len()).filter(|a| a.eq_ignore_ascii_case(p)).map(|_| &after[p.len()..]))
            .unwrap_or(after);
        let end = after.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-')).unwrap_or(after.len());
        let (name, tail) = after.split_at(end);
        let name = name.to_ascii_lowercase();
        let ends = tail.chars().next().is_none_or(|c| c.is_whitespace() || ",:;.!?".contains(c));
        if !handle_ok(&name) || !ends {
            break;
        }
        if !out.contains(&name) {
            out.push(name);
        }
        rest = tail.trim_start_matches([',', ':', ';', '.', '!', '?']).trim_start();
    }
    out
}

/// The post a message from a page's Session tab becomes: the mentions of the agents it's for (which wake them), the
/// words fenced, and how to answer. It's for the agents the message calls, else the one that published the page.
/// The reply names the session the page talks to, the publisher's, whichever agent answers and whatever session it
/// wakes in.
pub fn post(publisher: &str, text: &str, session: &str) -> String {
    let called = called(text);
    let (to, page) = match called.as_slice() {
        [] => (format!("@{publisher}"), "Session tab".to_string()),
        [one] if one == publisher => (format!("@{publisher}"), "Session tab".to_string()),
        many => (
            many.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join(" "),
            // the publisher in code, so naming it doesn't wake it too
            format!("Session tab of `@{publisher}`'s page"),
        ),
    };
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_matches('\n').trim_end();
    let (text, cut) = match text.char_indices().nth(TEXT_MAX) {
        Some((i, _)) => (&text[..i], format!("\n\n(cut: {} more characters)", text[i..].chars().count())),
        None => (text, String::new()),
    };
    let f = fence(text);
    format!(
        "{to} from spacesheep.dev ({page}, not signed):\n\n{f}\n{text}\n{f}{cut}\n\nAnswer here, then send it to the \
         page: `spacesheep talk reply --session {session} \"…\"`"
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
    fn a_message_starting_with_handles_is_for_those_agents() {
        assert_eq!(called("@editor cut the intro"), ["editor"]);
        assert_eq!(called("  @Editor, @scene: both of you, and @james later"), ["editor", "scene"]);
        assert_eq!(called("@editor @editor again"), ["editor"]);
        assert_eq!(called("@pm.editor, @PM.scene and @pm.editor cut it"), ["editor", "scene"], "Pointman's prefix");
        assert_eq!(called("@pointman.editor @Pointman.scene: hi"), ["editor", "scene"], "the long one too");
        assert!(called("@pm. cut it").is_empty(), "the prefix alone calls nobody");
        assert_eq!(called("@pm hi"), ["pm"], "an agent called pm is still called");
        assert!(called("hey @editor").is_empty(), "only at the start");
        assert!(called("@editor's cut is long").is_empty(), "not addressing");
        assert!(called("@-x hi").is_empty() && called("@ hi").is_empty() && called("").is_empty());
        let p = post("scene", "@editor, cut the intro", "abc-123");
        assert!(
            p.starts_with(
                "@editor from spacesheep.dev (Session tab of `@scene`'s page, not signed):\n\n```\n@editor, cut"
            ),
            "{p}"
        );
        assert!(p.ends_with("`spacesheep talk reply --session abc-123 \"…\"`"), "the page's session: {p}");
        assert!(post("scene", "@scene hi", "s").starts_with("@scene from spacesheep.dev (Session tab, not signed)"));
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
