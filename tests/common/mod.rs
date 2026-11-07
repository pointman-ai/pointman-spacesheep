//! What the tests share: the extension under the SDK's stand-in node, which answers `agents.sessions` from a
//! state the test sets and keeps each `board.post`, with a fake `spacesheep` (a shell script that logs each call
//! beside itself, prints what the test put there for each run of `talk listen`, then listens until it's stopped,
//! printing whatever the test pushes; and answers `talk status`, `list` and `versions` from files the test writes,
//! and `keys create` and `keys save` as a signed-out machine signing itself in, a save failing when told to).
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
/// `-y spacesheep@latest` first, and names its copy (`.bin/spacesheep`) when the test made one; as `node` it runs the
/// script it's given.
const FAKE: &str = r#"#!/bin/sh
here=$(dirname "$0")
if [ "$(basename "$0")" = node ]; then exec "$@"; fi
if [ "$(basename "$here")" = .bin ]; then here=$(dirname "$here"); fi
echo "$*" >> "$here/calls.log"
if [ "$1 $2 $3 $4 $5" = "-y -p spacesheep@latest -c command -v spacesheep" ]; then
  if [ -e "$here/.bin/spacesheep" ]; then echo "$here/.bin/spacesheep"; exit 0; fi
  echo "npm error could not determine executable to run" >&2; exit 1
fi
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
    if [ -f "$here/signed-out" ] && [ ! -f "$here/saved" ]; then
      printf '\n  ✗ not signed in — run `spacesheep login`\n\n' >&2; exit 3
    fi
    if [ -f "$here/status" ]; then cat "$here/status"; else printf '{\n  "pro": true,\n  "enabled": true,\n  "this_machine": false\n}\n'; fi;;
  "keys create")
    printf '%s' "$SPACESHEEP_KEY" > "$here/parent"
    if [ -z "$SPACESHEEP_KEY" ]; then printf '\n  ✗ not signed in\n\n' >&2; exit 3; fi
    if [ -f "$here/refuse" ]; then printf '\n  ✗ the server rejected that key\n\n' >&2; exit 1; fi
    printf '{"key": "ss_minted%s", "name": "%s", "scope": "full"}\n' "$$" "$4";;
  "keys save")
    if [ -f "$here/save-fails" ]; then
      key=$(cat); echo "$key" >> "$here/unsaved"
      printf '\n  ✗ could not check %s: fetch failed\n\n' "$key" >&2; exit 1
    fi
    cat > "$here/saved";;
  "versions list")
    page=""; space=""; prev=""
    for a in "$@"; do
      case "$a" in --after=*) page=".after-${a#--after=}";; esac
      if [ "$prev" = --space-id ]; then space="$a"; fi
      prev="$a"
    done
    if [ -f "$here/versions-fail" ]; then cat "$here/versions-fail"; exit 1; fi
    f="$here/versions.$space$page.json"
    if [ -f "$f" ]; then cat "$f"; else printf '{"code":"not_found","message":"no such page of results"}\n'; exit 1; fi;;
  "spaces list")
    page=""
    for a in "$@"; do case "$a" in --after=*) page=".after-${a#--after=}";; esac; done
    if [ -f "$here/new-fail" ]; then cat "$here/new-fail"; exit 1; fi
    if [ -f "$here/spaces$page.json" ]; then cat "$here/spaces$page.json"; else echo '{"spaces": [], "page": {"next": ""}}'; fi;;
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

    /// As `npx` with its copy of spacesheep@latest (`.bin/spacesheep`), and `node` beside it.
    pub fn npx_with_copy() -> Fake {
        let fake = Fake::named("npx");
        std::os::unix::fs::symlink(script(), fake.file("node")).unwrap();
        std::fs::create_dir(fake.file(".bin")).unwrap();
        std::os::unix::fs::symlink(script(), fake.file(".bin/spacesheep")).unwrap();
        fake
    }

    /// npm's cache cleaned: npx's copy has gone.
    pub fn copy_gone(&self) {
        std::fs::remove_file(self.file(".bin/spacesheep")).unwrap();
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
        // NixOS keeps coreutils (dirname, grep, sleep …, which the fake script uses) in its system profile, not /usr/bin
        let nixos = if std::path::Path::new("/run/current-system/sw/bin/dirname").exists() {
            ":/run/current-system/sw/bin"
        } else {
            ""
        };
        format!("{}:/usr/bin:/bin{nixos}", self.dir.path().display())
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

    /// A page this session published: in `spaces list`, with its versions saying who pushed it.
    pub fn published(&self, session: &str) {
        self.published_new_api(session);
    }

    /// The first page of `spaces list`'s rows so far.
    fn space_rows(&self) -> Vec<Value> {
        std::fs::read_to_string(self.file("spaces.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["spaces"].as_array().cloned())
            .unwrap_or_default()
    }

    /// The account's one page, as Spacesheep's new API answers: `list` is `{spaces, page}`, and the versions are newest
    /// first with no number, naming the session in `client`, after a newer version another account pushed.
    pub fn published_new_api(&self, session: &str) {
        let id = format!("page-{session}");
        let at = format!("{}Z", spacesheep::pages::iso(spacesheep::words::now() as i64));
        let versions = json!({"versions": [
            {"id": "c2", "created_at": at, "client": {"transport": "mcp", "other": "agent"}},
            {"id": "c1", "created_at": at, "client": {"name": "claude-code", "session_id": session}}],
            "page": {"next": ""}});
        std::fs::write(self.file(&format!("versions.{id}.json")), versions.to_string()).unwrap();
        let mut rows = self.space_rows();
        rows.push(json!({"id": id, "name": "@james/a-page", "updated_at": at}));
        let tmp = self.file("spaces.json.tmp");
        std::fs::write(&tmp, json!({"spaces": rows, "page": {"next": ""}}).to_string()).unwrap();
        std::fs::rename(tmp, self.file("spaces.json")).unwrap();
    }

    /// The account's one page, its versions in two pages of the new API's answer: the newest a version another account
    /// pushed, with `page.next` "c1"; the older this session's own, whose `page.next` is `next` ("" for the last page).
    pub fn published_in_two_pages(&self, session: &str, next: &str) {
        let id = format!("page-{session}");
        let at = format!("{}Z", spacesheep::pages::iso(spacesheep::words::now() as i64));
        let newest = json!({"versions": [{"id": "c2", "created_at": at, "client": {"transport": "mcp"}}], "page": {"next": "c1"}});
        std::fs::write(self.file(&format!("versions.{id}.json")), newest.to_string()).unwrap();
        self.older_page(session, "c1", next);
        let spaces = json!({"spaces": [{"id": id, "updated_at": at}], "page": {"next": ""}});
        let tmp = self.file("spaces.json.tmp");
        std::fs::write(&tmp, spaces.to_string()).unwrap();
        std::fs::rename(tmp, self.file("spaces.json")).unwrap();
    }

    /// What `versions --after=<after>` answers for this session's page, naming `next`: `list` doesn't change, so the
    /// page itself hasn't.
    pub fn older_page(&self, session: &str, after: &str, next: &str) {
        let at = format!("{}Z", spacesheep::pages::iso(spacesheep::words::now() as i64));
        let older = json!({"versions": [{"id": after, "created_at": at, "client": {"session_id": session}}], "page": {"next": next}});
        let tmp = self.file(&format!("versions.{session}.{after}.tmp"));
        std::fs::write(&tmp, older.to_string()).unwrap();
        std::fs::rename(tmp, self.file(&format!("versions.page-{session}.after-{after}.json"))).unwrap();
    }

    /// Every `spaces list` call prints this on stdout and exits 1: ErrorInfo, as `--json` does for a 401 or a 500.
    pub fn spaces_refuse(&self, error_info: &str) {
        std::fs::write(self.file("new-fail"), error_info).unwrap();
    }

    /// Every `versions list` call does.
    pub fn versions_refuse(&self, error_info: &str) {
        std::fs::write(self.file("versions-fail"), error_info).unwrap();
    }

    /// `spaces list`'s page after `after` ("" for the first), naming the account's pages `ids` (each changed now) and `next`.
    pub fn spaces_page(&self, after: &str, ids: &[&str], next: &str) {
        let at = format!("{}Z", spacesheep::pages::iso(spacesheep::words::now() as i64));
        let rows: Vec<Value> = ids.iter().map(|id| json!({"id": id, "updated_at": at})).collect();
        let name = if after.is_empty() { "spaces.json".to_string() } else { format!("spaces.after-{after}.json") };
        std::fs::write(self.file(&name), json!({"spaces": rows, "page": {"next": next}}).to_string()).unwrap();
    }

    /// `versions list`'s one page for `space`: this session's version, the last page.
    pub fn versions_of(&self, space: &str, session: &str) {
        let at = format!("{}Z", spacesheep::pages::iso(spacesheep::words::now() as i64));
        let v = json!({"versions": [{"id": "c1", "created_at": at, "client": {"session_id": session}}], "page": {"next": ""}});
        std::fs::write(self.file(&format!("versions.{space}.json")), v.to_string()).unwrap();
    }

    /// Only these sessions' pages in `list`, as when newer pages have pushed the others out of the account's latest.
    pub fn listed_only(&self, sessions: &[&str]) {
        let file = self.file("spaces.json");
        let spaces = self.space_rows();
        let keep = |s: &Value| sessions.iter().any(|x| s["id"] == format!("page-{x}"));
        let tmp = self.file("spaces.json.tmp");
        let kept: Vec<Value> = spaces.into_iter().filter(keep).collect();
        std::fs::write(&tmp, json!({"spaces": kept, "page": {"next": ""}}).to_string()).unwrap();
        std::fs::rename(tmp, file).unwrap();
    }

    /// A machine that isn't signed in: `talk status` exits 3, as the CLI's does, until `keys save` has kept a key.
    pub fn signed_out(&self) {
        std::fs::write(self.file("signed-out"), "").unwrap();
    }

    /// Spacesheep refuses the key `keys create` mints with.
    pub fn refuses(&self) {
        std::fs::write(self.file("refuse"), "").unwrap();
    }

    /// The key `keys create` had in its environment (SPACESHEEP_KEY), when it ran.
    pub fn parent_key(&self) -> Option<String> {
        std::fs::read_to_string(self.file("parent")).ok()
    }

    /// What `keys save` read on its stdin, when it ran.
    pub fn saved_key(&self) -> Option<String> {
        std::fs::read_to_string(self.file("saved")).ok()
    }

    /// `keys save` fails, as with no network, repeating the key it read in its error; or saves again.
    pub fn save_fails(&self, fails: bool) {
        if fails {
            std::fs::write(self.file("save-fails"), "").unwrap();
        } else {
            std::fs::remove_file(self.file("save-fails")).unwrap();
        }
    }

    /// Each key a failed `keys save` read, in turn.
    pub fn unsaved_keys(&self) -> Vec<String> {
        std::fs::read_to_string(self.file("unsaved")).unwrap_or_default().lines().map(String::from).collect()
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
    StandIn::new(ROOT)
        .binary(env!("CARGO_BIN_EXE_spacesheep"))
        .settings(settings)
        .env("PATH", path_var)
        // nothing of the machine's own Spacesheep sign-in or origin reaches a test: its HOME and config are the fake's folder
        .env("HOME", path_var.split(':').next().unwrap_or("/nonexistent"))
        .env("SPACESHEEP_CONFIG_DIR", path_var.split(':').next().unwrap_or("/nonexistent"))
        .env("SPACESHEEP_ORIGIN", "http://127.0.0.1:9")
        .env("POINTMAN_SPACESHEEP_FASTER", FASTER)
        .method("agents.sessions", move |_| Ok(json!({"agents": *agents.lock().unwrap()})))
        .on(move |post: BoardPostRequest| {
            let mut posts = posts.lock().unwrap();
            // each post as the tests read it: its thread's number and its text
            let thread = post.thread.as_option().and_then(|t| t.id.strip_prefix("thr_")?.parse::<i64>().ok());
            posts.push(json!({"thread_id": thread, "text": post.text, "blocks": post.blocks, "links": post.links}));
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
