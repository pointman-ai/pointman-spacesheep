//! What the tests share: the extension under the SDK's stand-in node, which answers `agents.sessions` from a
//! state the test sets and keeps each `board.post`, with a fake `spacesheep` (a shell script that logs each call
//! beside itself, prints what the test put there for each run of `talk listen`, then listens until it's stopped,
//! printing whatever the test pushes; and answers `talk status`, `list` and `versions` from files the test writes).
//! No test reaches spacesheep.dev or the board.
#![allow(dead_code)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pointman_extension::protocol::{BoardPostRequest, BoardPostResponse};
use pointman_extension::testing::StandIn;
use pointman_extension::v1::PostRef;
use serde_json::{json, Value};
use tempfile::TempDir;

pub const ROOT: &str = env!("CARGO_MANIFEST_DIR");
/// The extension's clock, this many times faster: a look every second, a listener started again in 0.1 s.
pub const FASTER: &str = "60";
pub const WITHIN: Duration = Duration::from_secs(10);

/// The fake CLI. It finds its folder from how it was started, so each test's is a link to this one script, written
/// once: one test writing a script while another starts a process is "text file busy" on Linux. As `npx` it takes
/// `-y spacesheep@latest` first.
const FAKE: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "$*" >> "$here/calls.log"
if [ "$1" = "-y" ]; then shift 2; fi
case "$1 $2" in
  "talk listen")
    session="$4"
    n=$(grep -c -- "talk listen --session $session\$" "$here/calls.log")
    echo $$ >> "$here/$session.pids"
    for f in "$here/$session.$n" "$here/$session"; do
      if [ -f "$f" ]; then cat "$f"; break; fi
    done
    if [ -f "$here/$session.$n.exit" ]; then
      cat "$here/$session.$n.stderr" >&2 2>/dev/null
      exit "$(cat "$here/$session.$n.exit")"
    fi
    while :; do
      if [ -f "$here/$session.push" ]; then
        mv "$here/$session.push" "$here/$session.pushed" && cat "$here/$session.pushed"
      fi
      sleep 0.05
    done;;
  "talk reply")
    echo '{"ok": true, "id": 1}';;
  "--help "*)
    if [ -f "$here/help" ]; then cat "$here/help"; else echo '    spacesheep talk reply <text> [--session ID]'; fi;;
  "talk status")
    if [ -f "$here/status" ]; then cat "$here/status"; else printf '{\n  "pro": true,\n  "enabled": true,\n  "this_machine": false\n}\n'; fi;;
  "list --json")
    if [ -f "$here/spaces.json" ]; then cat "$here/spaces.json"; else echo '[]'; fi;;
  "versions "*)
    if [ -f "$here/versions.$2.json" ]; then cat "$here/versions.$2.json"; else printf '\n  ✗ no space %s\n\n' "$2" >&2; exit 1; fi;;
  *)
    printf '\n  ✗ unknown command: %s\n\n' "$*" >&2; exit 1;;
esac
"#;

fn script() -> PathBuf {
    static SCRIPT: OnceLock<PathBuf> = OnceLock::new();
    SCRIPT
        .get_or_init(|| {
            let dir = std::env::temp_dir().join(format!("pointman-spacesheep-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("fake-spacesheep");
            std::fs::write(&path, FAKE).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        })
        .clone()
}

/// One test's fake CLI: its folder (the log, what each run prints) and the PATH that finds it.
pub struct Fake {
    pub dir: TempDir,
}

impl Fake {
    /// As `spacesheep`, the machine's own CLI.
    pub fn new() -> Fake {
        Fake::named("spacesheep")
    }

    /// As `npx`, for a machine with no `spacesheep`.
    pub fn npx() -> Fake {
        Fake::named("npx")
    }

    fn named(name: &str) -> Fake {
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(script(), dir.path().join(name)).unwrap();
        Fake { dir }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// The node's PATH: the fake first, then only the system's folders.
    pub fn path_var(&self) -> String {
        format!("{}:/usr/bin:/bin", self.dir.path().display())
    }

    /// What every run of `talk listen` for this session prints first.
    pub fn prints(&self, session: &str, text: &str) {
        std::fs::write(self.file(session), text).unwrap();
    }

    /// What its `run`th run prints (from 1), and its exit code, with this on stderr: it ends there.
    pub fn ends(&self, session: &str, run: u32, text: &str, code: i32, stderr: &str) {
        std::fs::write(self.file(&format!("{session}.{run}")), text).unwrap();
        std::fs::write(self.file(&format!("{session}.{run}.stderr")), stderr).unwrap();
        std::fs::write(self.file(&format!("{session}.{run}.exit")), code.to_string()).unwrap();
    }

    /// What its `run`th run prints before it listens on.
    pub fn prints_on(&self, session: &str, run: u32, text: &str) {
        std::fs::write(self.file(&format!("{session}.{run}")), text).unwrap();
    }

    /// A message arriving now, at the listener running for this session.
    pub fn push(&self, session: &str, text: &str) {
        let tmp = self.file(&format!("{session}.push.tmp"));
        std::fs::write(&tmp, text).unwrap();
        std::fs::rename(tmp, self.file(&format!("{session}.push"))).unwrap();
    }

    /// What `talk status` prints.
    pub fn status(&self, text: &str) {
        std::fs::write(self.file("status"), text).unwrap();
    }

    /// The account's Talk switch, as `talk status` says it.
    pub fn talk(&self, on: bool) {
        self.status(&format!("{{\n  \"pro\": true,\n  \"enabled\": {on}\n}}\n"));
    }

    /// A page this session published: in `list`, with its versions saying who pushed it.
    pub fn published(&self, session: &str) {
        let file = self.file("spaces.json");
        let mut spaces: Vec<Value> =
            std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let id = format!("page-{session}");
        let at = format!("{}.000Z", spacesheep::pages::iso(spacesheep::words::now() as i64));
        spaces.push(json!({"id": id, "updated_at": at, "title": "a page"}));
        let versions = json!({"space": {"uuid": id}, "versions": [{"number": 1, "pushed_by": {"label": "Claude Code", "session_id": session}}]});
        std::fs::write(self.file(&format!("versions.{id}.json")), versions.to_string()).unwrap();
        let tmp = self.file("spaces.json.tmp");
        std::fs::write(&tmp, Value::Array(spaces).to_string()).unwrap();
        std::fs::rename(tmp, file).unwrap();
    }

    /// Only these sessions' pages in `list`, as when newer pages have pushed the others out of the account's latest.
    pub fn listed_only(&self, sessions: &[&str]) {
        let file = self.file("spaces.json");
        let spaces: Vec<Value> =
            std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let keep = |s: &Value| sessions.iter().any(|x| s["id"] == format!("page-{x}"));
        let tmp = self.file("spaces.json.tmp");
        std::fs::write(&tmp, Value::Array(spaces.into_iter().filter(keep).collect()).to_string()).unwrap();
        std::fs::rename(tmp, file).unwrap();
    }

    /// A CLI whose `talk reply` takes `--to`, the message it answers, as its help says.
    pub fn answers_named(&self) {
        std::fs::write(self.file("help"), "    spacesheep talk reply <text> [--session ID] [--to MESSAGE_ID]\n")
            .unwrap();
    }

    /// Each call, its words.
    pub fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.file("calls.log")).unwrap_or_default().lines().map(String::from).collect()
    }

    /// How many times `talk listen` ran for this session.
    pub fn listens(&self, session: &str) -> usize {
        self.calls().iter().filter(|c| c.ends_with(&format!("talk listen --session {session}"))).count()
    }

    /// The pid of each run of `talk listen` for this session.
    pub fn pids(&self, session: &str) -> Vec<i32> {
        let text = std::fs::read_to_string(self.file(&format!("{session}.pids"))).unwrap_or_default();
        text.lines().filter_map(|l| l.trim().parse().ok()).collect()
    }
}

pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the process is there
    unsafe { libc::kill(pid, 0) == 0 }
}

/// An agent as `agents.sessions` gives it (an AgentSessionOnMachine, in standard JSON).
pub fn agent(handle: &str, session: &str, thread: i64) -> Value {
    json!({"agent": {"name": handle}, "client": "claude", "sessionId": session, "thread": {"id": format!("thr_{thread}")}})
}

/// The node's side: the agents it says, and each post made.
#[derive(Clone, Default)]
pub struct Board {
    pub agents: Arc<Mutex<Vec<Value>>>,
    pub posts: Arc<Mutex<Vec<Value>>>,
    pub failures: Arc<Mutex<usize>>,
    pub deleted_roots: Arc<Mutex<Vec<String>>>,
}

impl Board {
    pub fn with(agents: Vec<Value>) -> Board {
        let b = Board::default();
        *b.agents.lock().unwrap() = agents;
        b
    }

    pub fn set_agents(&self, agents: Vec<Value>) {
        *self.agents.lock().unwrap() = agents;
    }

    pub fn posts(&self) -> Vec<Value> {
        self.posts.lock().unwrap().clone()
    }

    /// Waits until the posts pass `test`, and gives them.
    pub fn wait_posts(&self, within: Duration, test: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let until = Instant::now() + within;
        loop {
            let got = self.posts();
            if test(&got) || Instant::now() >= until {
                return got;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

/// The extension under a stand-in node, with the fake CLI and these settings (none: `on()`).
pub fn spacesheep(fake: &Fake, board: &Board, settings: Value) -> StandIn {
    spacesheep_on(&fake.path_var(), board, settings)
}

/// The extension under a stand-in node, with this PATH.
pub fn spacesheep_on(path_var: &str, board: &Board, settings: Value) -> StandIn {
    let (agents, posts) = (board.agents.clone(), board.posts.clone());
    let failures = board.failures.clone();
    let deleted_roots = board.deleted_roots.clone();
    StandIn::new(ROOT)
        .binary(env!("CARGO_BIN_EXE_spacesheep"))
        .settings(settings)
        .env("PATH", path_var)
        .env("POINTMAN_SPACESHEEP_FASTER", FASTER)
        .method("agents.sessions", move |_| Ok(json!({"agents": *agents.lock().unwrap()})))
        .on(move |post: BoardPostRequest| {
            if let Some(root) = post.side_of.as_option() {
                if deleted_roots.lock().unwrap().contains(&root.id) {
                    return Err(anyhow::anyhow!("core didn't take the post: side_of: post {} was deleted", root.id));
                }
            }
            let mut failures = failures.lock().unwrap();
            if *failures > 0 {
                *failures -= 1;
                return Err(anyhow::anyhow!("posting is unavailable"));
            }
            let mut posts = posts.lock().unwrap();
            // each post as the tests read it: its thread's number and its text
            let thread = post.thread.as_option().and_then(|t| t.id.strip_prefix("thr_")?.parse::<i64>().ok());
            posts.push(json!({"thread_id": thread, "text": post.text, "blocks": post.blocks, "links": post.links, "side_of": post.side_of}));
            let id = PostRef { id: format!("post_{}", 1000 + posts.len()), ..Default::default() };
            Ok(BoardPostResponse { post: id.into(), ..Default::default() })
        })
}

/// The settings it runs with: it has none, since the account's Talk switch is the one switch.
pub fn on() -> Value {
    json!({})
}

/// Waits until `test` passes; whether it did.
pub fn wait(within: Duration, mut test: impl FnMut() -> bool) -> bool {
    let until = Instant::now() + within;
    while Instant::now() < until {
        if test() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    test()
}

/// One of health.check's parts, by id.
pub fn part<'a>(health: &'a Value, id: &str) -> &'a Value {
    health["parts"].as_array().unwrap().iter().find(|p| p["id"] == id).unwrap_or_else(|| panic!("no {id} in {health}"))
}

/// The handles a post @mentions, as the board reads them (board.py's MENTION, with CODE's spans taken out).
pub fn mentioned(text: &str) -> Vec<String> {
    let code = fancy_regex::Regex::new(r"(?s)(`+).*?\1").unwrap();
    let mention = fancy_regex::Regex::new(r"(?i)(?<![\w@/.:])@([a-z0-9][a-z0-9_-]{0,31})").unwrap();
    let bare = code.replace_all(text, " ");
    let mut out: Vec<String> =
        mention.captures_iter(&bare).map(|c| c.unwrap().get(1).unwrap().as_str().to_lowercase()).collect();
    out.sort();
    out.dedup();
    out
}
