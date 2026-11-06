//! EXT-141: Spacesheep CLI 1.36.0's contract, as the released CLI itself keeps it, against a local HTTP stand-in.
//! Independent of this extension's adapter: it runs the actual `spacesheep` 1.36.0 (node) and asserts what it asks of
//! a server and how it answers. The stand-in serves a trimmed `/v1/doors.json` and first/last/empty lists (fixtures
//! in tests/fixtures/ext141/, generated from server 13cea358's descriptors: schema evidence, not live captures).
//! Synthetic credentials only; empty HOME and SPACESHEEP_CONFIG_DIR; every origin is the local stand-in; nothing is
//! inherited from the environment. No spacesheep.dev, no account.
//!
//! Runs when SPACESHEEP_CLI_1_36 is the CLI's `bin/spacesheep.js` (npm install spacesheep@1.36.0) and `node` is on
//! the PATH (or SPACESHEEP_NODE names it); otherwise it says so and passes. CI sets it.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::{json, Value};

const KEY: &str = "synthetic-good-key-0000";
const FIRST_SPACE: &str = "11111111-1111-4111-8111-111111111111";
const EMPTY_SPACE: &str = "33333333-3333-4333-8333-333333333333";
const BROKEN_SPACE: &str = "55555555-5555-4555-8555-555555555555";

fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ext141").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// One request the stand-in saw.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    query: Vec<(String, String)>,
    auth: Option<String>,
    user_agent: Option<String>,
}

struct StandIn {
    origin: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl StandIn {
    /// `old`: a server without /v1 (its doors table is a 404, as before the table existed).
    fn start(old: bool) -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for conn in listener.incoming().flatten() {
                let log = log.clone();
                thread::spawn(move || serve(conn, old, log));
            }
        });
        StandIn { origin, seen }
    }
    fn calls(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().iter().filter(|s| s.path != "/v1/doors.json").cloned().collect()
    }
    fn doors_requests(&self) -> usize {
        self.seen.lock().unwrap().iter().filter(|s| s.path == "/v1/doors.json").count()
    }
}

fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn serve(mut conn: TcpStream, old: bool, log: Arc<Mutex<Vec<Seen>>>) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match conn.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or("");
    let mut parts = first.split(' ');
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let headers: HashMap<String, String> =
        lines.filter_map(|l| l.split_once(": ")).map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect();
    let (path, q) = target.split_once('?').unwrap_or((target, ""));
    let query: Vec<(String, String)> = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect();
    let q_of = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    log.lock().unwrap().push(Seen {
        path: path.to_string(),
        query: query.clone(),
        auth: headers.get("authorization").cloned(),
        user_agent: headers.get("user-agent").cloned(),
    });
    let error = |code: &str, message: &str| json!({"code": code, "message": message, "violations": []}).to_string();
    let (status, body): (u16, String) = if method != "GET" {
        (405, error("method_not_allowed", "GET only"))
    } else if path == "/v1/doors.json" {
        if old {
            (404, error("not_found", "no /v1 here"))
        } else {
            (200, fixture("doors.json"))
        }
    } else if old {
        (404, error("not_found", "no /v1 here"))
    } else if headers.get("authorization").map(String::as_str) != Some(&format!("Bearer {KEY}")) {
        (401, error("unauthenticated", "that key is not accepted"))
    } else if path == "/v1/spaces" {
        match q_of("after").as_deref() {
            None | Some("") => (200, fixture("spaces-first.json")),
            Some("1") => (200, fixture("spaces-last.json")),
            Some("empty") => (200, fixture("spaces-empty.json")),
            Some("boom") => (500, error("internal", "the server is having trouble")),
            Some(_) => (400, error("invalid_argument", "after: not a cursor")),
        }
    } else if let Some(id) = path.strip_prefix("/v1/spaces/").and_then(|p| p.strip_suffix("/versions")) {
        match (id, q_of("after").as_deref()) {
            (BROKEN_SPACE, _) => (500, error("internal", "the server is having trouble")),
            (EMPTY_SPACE, _) => (200, fixture("versions-empty.json")),
            (FIRST_SPACE, None | Some("")) => (200, fixture("versions-first.json")),
            (FIRST_SPACE, Some("1")) => (200, fixture("versions-last.json")),
            (FIRST_SPACE, Some(_)) => (400, error("invalid_argument", "after: not a cursor")),
            _ => (404, error("not_found", "no such space")),
        }
    } else {
        (404, error("not_found", "no such call"))
    };
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Other",
    };
    let _ = write!(
        conn,
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn cli() -> Option<(PathBuf, PathBuf)> {
    let Some(script) = std::env::var_os("SPACESHEEP_CLI_1_36") else {
        eprintln!("skipped: set SPACESHEEP_CLI_1_36 to spacesheep 1.36.0's bin/spacesheep.js to run the real CLI");
        return None;
    };
    let node = std::env::var_os("SPACESHEEP_NODE").map(PathBuf::from).unwrap_or_else(|| {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|d| d.join("node"))
            .find(|p| p.is_file())
            .expect("node on the PATH, or SPACESHEEP_NODE")
    });
    Some((node, PathBuf::from(script)))
}

/// The real CLI in an empty HOME and config dir, with only the stand-in's origins and a synthetic key (or none).
fn run(c: &(PathBuf, PathBuf), server: &StandIn, key: Option<&str>, args: &[&str]) -> Out {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    let mut cmd = Command::new(&c.0);
    cmd.arg(&c.1)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("xdg"))
        .env("SPACESHEEP_CONFIG_DIR", &config)
        .env("SPACESHEEP_ORIGIN", &server.origin)
        .env("SPACESHEEP_APP_ORIGIN", &server.origin)
        .env("SPACESHEEP_CONTENT_ORIGIN", &server.origin)
        .env("SPACESHEEP_NO_UPDATE_CHECK", "1")
        .current_dir(home.path());
    if let Some(k) = key {
        cmd.env("SPACESHEEP_KEY", k);
    }
    let o = cmd.output().unwrap();
    Out {
        code: o.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

fn json_of(o: &Out) -> Value {
    serde_json::from_str(&o.stdout)
        .unwrap_or_else(|e| panic!("stdout isn't JSON ({e}): {:?}\nstderr: {}", o.stdout, o.stderr))
}

fn query(s: &Seen) -> Vec<(&str, &str)> {
    s.query.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

#[test]
fn the_cli_is_1_36_0() {
    let Some(c) = cli() else { return };
    let pkg = c.1.parent().unwrap().parent().unwrap().join("package.json");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(pkg).unwrap()).unwrap();
    assert_eq!(v["version"], "1.36.0");
}

#[test]
fn spaces_list_asks_the_canonical_call_and_maps_the_cursor_to_after() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);

    let first = run(&c, &server, Some(KEY), &["spaces", "list", "--json", "--limit", "1"]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let body = json_of(&first);
    assert_eq!(body["spaces"][0]["id"], FIRST_SPACE);
    assert_eq!(body["spaces"][0]["updated_at"], "2026-11-06T08:00:00Z", "snake_case, as served");
    assert_eq!(body["spaces"][0]["team_access"], "unspecified");
    let next = body["page"]["next"].as_str().unwrap();
    assert_eq!(next, "1");

    // the last answer's page.next goes back as --after, and rides in the query as `after`
    let last = run(&c, &server, Some(KEY), &["spaces", "list", "--json", "--limit", "1", "--after", next]);
    assert_eq!(last.code, 0, "{}", last.stderr);
    let body = json_of(&last);
    assert_eq!(body["spaces"][0]["id"], "22222222-2222-4222-8222-222222222222");
    assert_eq!(body["page"]["next"], "", "the last page's next is empty");

    let empty = run(&c, &server, Some(KEY), &["spaces", "list", "--json", "--after", "empty"]);
    assert_eq!(empty.code, 0, "{}", empty.stderr);
    assert_eq!(json_of(&empty), json!({"spaces": [], "page": {"next": ""}}));

    let calls = server.calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(calls.iter().all(|s| s.path == "/v1/spaces"), "{calls:?}");
    assert_eq!(query(&calls[0]), vec![("limit", "1")]);
    assert_eq!(query(&calls[1]), vec![("limit", "1"), ("after", "1")]);
    assert_eq!(query(&calls[2]), vec![("after", "empty")]);
    for s in &calls {
        assert_eq!(s.auth.as_deref(), Some(&*format!("Bearer {KEY}")));
        assert_eq!(s.user_agent.as_deref(), Some("spacesheep-cli/1.36.0"));
    }
}

#[test]
fn versions_list_space_id_goes_in_the_path_and_not_the_query() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);

    let first = run(&c, &server, Some(KEY), &["versions", "list", "--space-id", FIRST_SPACE, "--json", "--limit", "1"]);
    assert_eq!(first.code, 0, "{}", first.stderr);
    let body = json_of(&first);
    assert_eq!(body["versions"][0]["id"], "aaaaaaaaaaaa");
    assert_eq!(body["versions"][0]["created_at"], "2026-11-06T08:00:00Z");
    // client.session_id: what a version says published it, as served
    assert_eq!(body["versions"][0]["client"]["session_id"], "session-new");
    assert_eq!(body["versions"][0]["client"]["transport"], "cli");
    assert_eq!(body["page"]["next"], "1");

    let last = run(
        &c,
        &server,
        Some(KEY),
        &["versions", "list", "--space-id", FIRST_SPACE, "--json", "--limit", "1", "--after", "1"],
    );
    assert_eq!(last.code, 0, "{}", last.stderr);
    let body = json_of(&last);
    assert_eq!(body["versions"][0]["id"], "bbbbbbbbbbbb");
    assert_eq!(body["versions"][0]["client"]["session_id"], "session-old");
    assert_eq!(body["page"]["next"], "");

    let empty = run(&c, &server, Some(KEY), &["versions", "list", "--space-id", EMPTY_SPACE, "--json"]);
    assert_eq!(empty.code, 0, "{}", empty.stderr);
    assert_eq!(json_of(&empty), json!({"versions": [], "page": {"next": ""}}));

    let calls = server.calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert_eq!(calls[0].path, format!("/v1/spaces/{FIRST_SPACE}/versions"));
    assert_eq!(query(&calls[0]), vec![("limit", "1")], "the id is in the path, left out of the query");
    assert_eq!(query(&calls[1]), vec![("limit", "1"), ("after", "1")]);
    assert_eq!(calls[2].path, format!("/v1/spaces/{EMPTY_SPACE}/versions"));
    assert!(query(&calls[2]).is_empty(), "{:?}", calls[2]);
}

#[test]
fn the_text_answer_shows_snake_case_fields_and_the_session_id() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);
    let o = run(&c, &server, Some(KEY), &["versions", "list", "--space-id", FIRST_SPACE]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("session_id: session-new"), "{}", o.stdout);
    assert!(o.stdout.contains("created_at: 2026-11-06T08:00:00Z"), "{}", o.stdout);
    assert!(o.stdout.contains("next: 1"), "{}", o.stdout);
    assert!(!o.stdout.contains("sessionId") && !o.stdout.contains("createdAt"), "{}", o.stdout);
}

#[test]
fn the_old_commands_are_the_new_calls_when_the_table_says_so() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);
    // `list` and `versions <space>` are named in the table's `replaces`: the same calls
    let o = run(&c, &server, Some(KEY), &["list", "--json"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(json_of(&o)["spaces"][0]["id"], FIRST_SPACE);
    let calls = server.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0].path, "/v1/spaces");
}

#[test]
fn errors_are_error_info_on_stdout_with_json_and_a_nonzero_exit() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);

    // 401: a key the server doesn't accept (synthetic)
    let o = run(&c, &server, Some("synthetic-wrong-key"), &["spaces", "list", "--json"]);
    assert_ne!(o.code, 0);
    assert_eq!(o.code, 1, "{}", o.stderr);
    let e = json_of(&o);
    assert_eq!(e, json!({"code": "unauthenticated", "message": "that key is not accepted", "violations": []}));

    // 500, on spaces
    let o = run(&c, &server, Some(KEY), &["spaces", "list", "--json", "--after", "boom"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert_eq!(json_of(&o), json!({"code": "internal", "message": "the server is having trouble", "violations": []}));

    // 500, on versions
    let o = run(&c, &server, Some(KEY), &["versions", "list", "--space-id", BROKEN_SPACE, "--json"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert_eq!(json_of(&o)["code"], "internal");

    // 400 and 404 too
    let o = run(&c, &server, Some(KEY), &["spaces", "list", "--json", "--after", "nonsense"]);
    assert_eq!(o.code, 1);
    assert_eq!(json_of(&o)["code"], "invalid_argument");
    let o = run(&c, &server, Some(KEY), &["versions", "list", "--space-id", "no-such-space", "--json"]);
    assert_eq!(o.code, 1);
    assert_eq!(json_of(&o)["code"], "not_found");

    // without --json the same ErrorInfo goes to stderr in words, nothing on stdout
    let o = run(&c, &server, Some("synthetic-wrong-key"), &["spaces", "list"]);
    assert_eq!(o.code, 1);
    assert!(o.stdout.is_empty(), "{}", o.stdout);
    assert!(o.stderr.contains("that key is not accepted"), "{}", o.stderr);
}

#[test]
fn no_key_is_a_usage_failure_that_asks_nothing_of_the_server() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);
    let o = run(&c, &server, None, &["spaces", "list", "--json"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("not signed in"), "{}", o.stderr);
    assert!(server.calls().is_empty(), "{:?}", server.calls());
}

#[test]
fn a_wrong_command_line_exits_2_before_any_request() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);
    // versions list needs --space or --space-id
    let o = run(&c, &server, Some(KEY), &["versions", "list", "--json"]);
    assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("needs --space"), "{}", o.stderr);
    let o = run(&c, &server, Some(KEY), &["spaces", "list", "--limit", "many"]);
    assert_eq!(o.code, 2, "{}", o.stderr);
    assert!(server.calls().is_empty(), "{:?}", server.calls());
}

#[test]
fn an_old_server_without_a_doors_table_has_no_new_commands() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(true);

    // `spaces list` is not a command the CLI knows by itself: it must not reach a /v1/spaces call
    let o = run(&c, &server, Some(KEY), &["spaces", "list", "--json"]);
    assert_ne!(o.code, 0, "{}{}", o.stdout, o.stderr);
    // the table was asked for (404), and nothing else on /v1 was ever called
    assert!(server.doors_requests() >= 1);
    assert!(server.calls().iter().all(|s| !s.path.starts_with("/v1/")), "{:?}", server.calls());

    // `versions list --space-id X` isn't read as `versions list` with a flag either
    let o = run(&c, &server, Some(KEY), &["versions", "list", "--space-id", FIRST_SPACE, "--json"]);
    assert_ne!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(server.calls().iter().all(|s| !s.path.starts_with("/v1/spaces")), "{:?}", server.calls());
}

#[test]
fn on_an_old_server_help_for_a_new_command_is_top_level_help_exit_1_for_spaces_and_0_for_versions() {
    // What an adapter probing `spaces list --help` meets when the doors table is absent (404): not the command's
    // help, not exit 0. A probe must read exit 1 plus this help as "not served"; no /v1 call is made for it.
    let Some(c) = cli() else { return };
    let server = StandIn::start(true);
    // `spaces` is no command of the CLI's own: exit 1. `versions` is (the old `versions <space>`): the same top-level
    // help at exit 0. So the exit status alone cannot say "not served"; the help text is the same either way.
    for (args, code) in [(&["spaces", "list", "--help"][..], 1), (&["versions", "list", "--help"][..], 0)] {
        let o = run(&c, &server, Some(KEY), args);
        assert_eq!(o.code, code, "{args:?}: {}{}", o.stdout, o.stderr);
        let all = format!("{}{}", o.stdout, o.stderr);
        assert!(all.contains("spacesheep 1.36.0 — publish web pages"), "{args:?}: {all}");
        assert!(!all.contains("--space-id") && !all.contains("--after"), "{args:?}: {all}");
    }
    assert!(server.calls().iter().all(|s| !s.path.starts_with("/v1/spaces")), "{:?}", server.calls());
}

#[test]
fn on_a_server_with_the_table_help_for_the_new_command_is_its_own_help_with_exit_0() {
    let Some(c) = cli() else { return };
    let server = StandIn::start(false);
    let o = run(&c, &server, None, &["spaces", "list", "--help"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("spacesheep spaces list") && o.stdout.contains("--after"), "{}", o.stdout);
    let o = run(&c, &server, None, &["versions", "list", "--help"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("--space-id"), "{}", o.stdout);
    assert!(server.calls().is_empty());
}
