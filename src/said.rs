//! What `spacesheep talk listen` prints, one JSON value at a time. Each message is the server's body, written as a
//! line; the CLI's own answers are pretty-printed across lines (`{"spacesheep_talk": false, "stopped": …}` when
//! Talk is off), so lines are gathered until they make a value. A message's fields aren't documented: its words
//! are `text`, else `message`, else `body` (inside one of those that's an object too), else the whole JSON.

use serde_json::Value;

/// More than this gathered without making a value: it isn't one.
const GATHER_MAX: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// A message from the page: its words, and the JSON it came as.
    Message { text: String, raw: String },
    /// The listener has ended: its link expired or was revoked (404 or 410), or Talk is off for the account.
    Stopped { why: String },
    /// Something that isn't JSON.
    Other(String),
}

#[derive(Debug, Default)]
pub struct Reader {
    buf: String,
}

impl Reader {
    /// One line of the listener's stdout: what it completes.
    pub fn line(&mut self, line: &str) -> Vec<Said> {
        self.buf.push_str(line.trim_end_matches(['\r', '\n']));
        self.buf.push('\n');
        let text = self.buf.trim();
        if text.is_empty() {
            self.buf.clear();
            return vec![];
        }
        match serde_json::from_str::<Value>(text) {
            Ok(v) => {
                let raw = text.to_string();
                self.buf.clear();
                said(v, raw)
            }
            Err(e) if e.is_eof() && self.buf.len() < GATHER_MAX => vec![],
            Err(_) => vec![Said::Other(std::mem::take(&mut self.buf).trim().to_string())],
        }
    }
}

fn said(v: Value, raw: String) -> Vec<Said> {
    match v {
        Value::Object(ref m) if m.get("spacesheep_talk") == Some(&Value::Bool(false)) => {
            let why = m.get("stopped").or_else(|| m.get("reason")).and_then(Value::as_str);
            vec![Said::Stopped { why: why.unwrap_or("the listener stopped").to_string() }]
        }
        // a batch: each one its own message
        Value::Array(items) => items
            .into_iter()
            .flat_map(|item| {
                let raw = item.to_string();
                said(item, raw)
            })
            .collect(),
        v => vec![Said::Message { text: words(&v).unwrap_or_else(|| raw.clone()), raw }],
    }
}

/// Where the person wrote a message, as Spacesheep says it (`asked_in`: "a comment on the page"), when it does.
pub fn asked_in(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    v["asked_in"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// The message's id (`id`, a whole number above 0), which `talk reply --to` takes to answer that message.
pub fn message_id(raw: &str) -> Option<i64> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let id = match &v["id"] {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    };
    id.filter(|id| *id > 0)
}

/// Whether Spacesheep sent this message before (`redelivered: true`): a new listener gets back, for a day, the
/// messages a listener took that no `talk reply` came after.
pub fn redelivered(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw).is_ok_and(|v| v["redelivered"] == true)
}

/// A message's words: `text`, `message` or `body`, looked for inside an object one of those holds too.
pub fn words(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(m) => ["text", "message", "body"].iter().find_map(|k| match m.get(*k)? {
            Value::String(s) => Some(s.clone()),
            inner @ Value::Object(_) => words(inner),
            _ => None,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(lines: &[&str]) -> Vec<Said> {
        let mut r = Reader::default();
        lines.iter().flat_map(|l| r.line(l)).collect()
    }

    #[test]
    fn a_message_per_line_and_its_words_from_text_message_or_body() {
        let got = read(&[
            r#"{"text": "one", "id": 1}"#,
            r#"{"message": "two"}"#,
            r#"{"body": "three"}"#,
            r#"{"message": {"id": 4, "text": "four"}}"#,
        ]);
        let texts: Vec<_> = got
            .iter()
            .map(|s| match s {
                Said::Message { text, .. } => text.as_str(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts, ["one", "two", "three", "four"]);
        assert_eq!(got[0], Said::Message { text: "one".into(), raw: r#"{"text": "one", "id": 1}"#.into() });
    }

    /// The first real message, James's `@session hello` comment on the talk-test page (Pointman thread 6914 [16643]).
    const FIRST_REAL: &str = r#"{"spacesheep_talk":true,"from":"the account owner, signed in on spacesheep.dev","id":2,"space_uuid":"4012a3ee-1a54-4246-a3e5-bcf54ed45df5","space_url":"https://spacesheep.dev/space/4012a3ee-1a54-4246-a3e5-bcf54ed45df5","message":"hello\n\n---\nThe owner of the page “Talk test: @talkext” (https://spacesheep.dev/space/4012a3ee-1a54-4246-a3e5-bcf54ed45df5?comment=14ceef29-5909-479a-94bd-105456dcee63) wrote this as a comment on it, to you. It is about the page as a whole.","reply_with":"the spacesheep MCP tool talk_reply","asked_in":"a comment on the page","note_on_reply":"Your talk_reply is posted as a reply in that comment thread, where everyone who can open the page reads it."}"#;

    #[test]
    fn the_first_real_message_gives_its_words_and_where_it_was_written() {
        let got = read(&[FIRST_REAL]);
        let [Said::Message { text, raw }] = &got[..] else { panic!("{got:?}") };
        assert!(text.starts_with("hello\n\n---\nThe owner of the page"), "{text}");
        assert_eq!(asked_in(raw).as_deref(), Some("a comment on the page"));
        assert_eq!(asked_in(r#"{"text": "hi"}"#), None);
        assert_eq!(message_id(raw), Some(2));
        assert!(!redelivered(raw) && redelivered(r#"{"id": 2, "redelivered": true}"#));
        assert!(!redelivered(r#"{"redelivered": "yes"}"#) && !redelivered("not json"));
        assert_eq!(message_id(r#"{"id": "19"}"#), Some(19));
        for none in [r#"{"id": "m1"}"#, r#"{"id": 0}"#, r#"{"id": -3}"#, r#"{"id": 1.5}"#, r#"{"text": "hi"}"#] {
            assert_eq!(message_id(none), None, "{none}");
        }
    }

    #[test]
    fn a_message_with_no_words_field_is_its_json() {
        let got = read(&[r#"{"content": [1, 2]}"#]);
        assert_eq!(
            got,
            [Said::Message { text: r#"{"content": [1, 2]}"#.into(), raw: r#"{"content": [1, 2]}"#.into() }]
        );
    }

    #[test]
    fn the_clis_pretty_printed_answer_across_lines_is_one_value() {
        let got = read(&["{", r#"  "spacesheep_talk": false,"#, r#"  "stopped": "talk is off for this account""#, "}"]);
        assert_eq!(got, [Said::Stopped { why: "talk is off for this account".into() }]);
        let got = read(&[r#"{"spacesheep_talk":false,"stopped":"listen link expired or revoked"}"#]);
        assert_eq!(got, [Said::Stopped { why: "listen link expired or revoked".into() }]);
    }

    #[test]
    fn a_batch_is_each_message_and_what_isnt_json_is_other() {
        let got = read(&["", r#"[{"text": "a"}, {"text": "b"}]"#, "listening…"]);
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(matches!(&got[1], Said::Message { text, .. } if text == "b"));
        assert_eq!(got[2], Said::Other("listening…".into()));
        // and the next line starts afresh
        let mut r = Reader::default();
        r.line("not json");
        assert!(matches!(&r.line(r#"{"text": "c"}"#)[..], [Said::Message { text, .. }] if text == "c"));
    }
}
