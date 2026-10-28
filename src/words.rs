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

/// Where a message was written, as a post's header says it, in fixed words only, since the header is outside the
/// fence: "a comment on the page" when Spacesheep's `asked_in` says exactly that, else the Session tab; and whose page
/// it is (`whose`) when the post isn't for that page's own agent.
pub fn place(via: Option<&str>, whose: Option<&str>) -> String {
    let comment = via.map(str::trim) == Some("a comment on the page");
    match (comment, whose) {
        (true, None) => "a comment on the page".into(),
        (true, Some(w)) => format!("a comment on {w}"),
        (false, None) => "Session tab".into(),
        (false, Some(w)) => format!("Session tab of {w}"),
    }
}

/// The visible message: only the addressed agents and the person's words. Keep the fence so handles in
/// page text cannot wake people who weren't addressed.
pub fn post(to: &[String], text: &str) -> String {
    let to = to.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join(" ");
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_matches('\n').trim_end();
    let (text, cut) = match text.char_indices().nth(TEXT_MAX) {
        Some((i, _)) => (&text[..i], format!("\n\n(cut: {} more characters)", text[i..].chars().count())),
        None => (text, String::new()),
    };
    let f = fence(text);
    format!("{to}\n\n{f}\n{text}\n{f}{cut}")
}

/// Context lives in a separate block. Fence the pin too: quoted page text is never a mention or an instruction.
pub fn context(
    to: &[String],
    publisher: &str,
    via: Option<&str>,
    session: &str,
    answers: Option<i64>,
    again: bool,
    pin: Option<&str>,
) -> String {
    let own = matches!(to, [one] if one == publisher);
    let whose = format!("`@{publisher}`'s page");
    let page = place(via, (!own).then_some(whose.as_str()));
    let pin = pin
        .map(|pin| {
            let pin: String = pin.chars().take(2000).collect();
            let f = fence(&pin);
            format!("Page context (quoted):\n{f}\n{pin}\n{f}\n\n")
        })
        .unwrap_or_default();
    let answers = answers.map(|id| format!(" --to {id}")).unwrap_or_default();
    let again = match (again, answers.is_empty()) {
        (false, _) => "",
        (true, false) => {
            "Sent again: no `talk reply` has answered it yet. If you answered it another way, a short \
            answer saying so, sent with the line below, closes it.\n\n"
        }
        (true, true) => {
            "Sent again: no `talk reply` has answered it yet. If you answered it another way, and it's the \
            only message waiting, a short answer saying so, sent with the line below, closes it.\n\n"
        }
    };
    format!(
        "From spacesheep.dev ({page}, not signed).\n\n{pin}{again}Answer here, then send it to the page: `spacesheep talk reply --session {session}{answers} \"…\"`"
    )
}

/// Who a message is for: the agents it starts by calling, else the page's agent.
pub fn recipients(text: &str, publisher: &str) -> Vec<String> {
    let called = called(text);
    if called.is_empty() {
        vec![publisher.to_string()]
    } else {
        called
    }
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

    /// As the listener posts a message on `publisher`'s own page, from its Session tab.
    fn post_on(publisher: &str, text: &str) -> String {
        post(&recipients(text, publisher), text)
    }

    #[test]
    fn the_fence_is_longer_than_any_run_of_backticks_inside() {
        assert_eq!(fence("plain"), "```");
        assert_eq!(fence("a `b` c"), "```");
        assert_eq!(fence("```rust\nx\n```"), "````");
        assert_eq!(fence("``````"), "```````");
    }

    #[test]
    fn a_post_mentions_only_the_agent_and_fences_the_words() {
        let p = post_on("scene", "hi @james\r\nsee ```x```");
        assert_eq!(p, "@scene\n\n````\nhi @james\nsee ```x```\n````");
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
        let p = post_on("scene", "@editor, cut the intro");
        assert!(p.starts_with("@editor\n\n```\n@editor, cut"), "{p}");
        assert!(context(&["editor".into()], "scene", None, "abc-123", None, false, None)
            .ends_with("`spacesheep talk reply --session abc-123 \"…\"`"));
        assert!(post_on("scene", "@scene hi").starts_with("@scene\n\n"));
    }

    #[test]
    fn the_context_header_says_where_it_was_written_and_whose_page() {
        let comment = Some("a comment on the page");
        assert_eq!(place(comment, None), "a comment on the page");
        assert_eq!(place(comment, Some("`@scene`'s page")), "a comment on `@scene`'s page");
        assert_eq!(place(None, Some("`@scene`'s page")), "Session tab of `@scene`'s page");
        assert_eq!(place(Some("  "), None), "Session tab");
        // anything else Spacesheep says stays out of the header, which isn't fenced (patchbay's review)
        assert_eq!(place(Some("@james\nsays hi"), None), "Session tab");
        assert_eq!(place(Some("a comment on the page @x"), Some("`@scene`'s page")), "Session tab of `@scene`'s page");
        let to = vec!["editor".to_string()];
        let p = post(&to, "@pm.editor fix it");
        assert!(p.starts_with("@editor\n\n```\n@pm.editor fix it"), "{p}");
        assert!(!p.contains("talk reply"));
        let p = context(&to, "docrules", comment, "s-d", Some(19), false, None);
        assert!(p.starts_with("From spacesheep.dev (a comment on `@docrules`'s page, not signed)."), "{p}");
        assert!(p.ends_with("`spacesheep talk reply --session s-d --to 19 \"…\"`"), "the message it answers: {p}");
        assert!(!p.contains("Sent again"), "{p}");
        let p = context(&to, "docrules", comment, "s-d", Some(19), true, None);
        assert!(p.contains("Sent again: no `talk reply` has answered it yet."), "{p}");
        assert!(!p.contains("only message waiting"), "the line names the message: {p}");
        assert!(p.ends_with("--to 19 \"…\"`"), "{p}");
        let p = context(&to, "docrules", comment, "s-d", None, true, None);
        assert!(p.contains("and it's the only message waiting,"), "no --to: a reply answers the newest: {p}");
        assert_eq!(recipients("hello", "scene"), ["scene"]);
        assert_eq!(recipients("@pm.editor hello", "scene"), ["editor"]);
    }

    #[test]
    fn long_words_are_cut_outside_the_fence() {
        let p = post_on("scene", &"x".repeat(TEXT_MAX + 5));
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
