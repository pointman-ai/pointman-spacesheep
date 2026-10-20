//! The extension's process: the node starts it as extension.toml's `[run]` says (`bin/spacesheep`) and talks to
//! it over stdin and stdout (the SDK handles that). Log with `h.log(…)`: stdout is the node's.
//!
//! Every second on the `talk` lane it checks whether a look is due: every 60 s, and at once after the settings
//! change. A look, while the `agents` setting lists anyone: the node's `agents.sessions`, then a listener for each
//! listed agent with a session and a thread, and none for the rest. A listener
//! is a thread running `spacesheep talk listen --session <id>`, started again with a growing wait whenever it ends.
//! Each message goes out as a `board.post` in the agent's thread, in order, from a thread of their own. The
//! listeners' process groups are kept in the state folder, so a restart stops any left behind.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use pointman_extension::{Extension, Handle};
use serde_json::{json, Value};
use spacesheep::cli::{self, Cli};
use spacesheep::said::{Reader, Said};
use spacesheep::words::{self, clock, now};

/// How often a look is checked for.
const TICK: Duration = Duration::from_secs(1);
/// How often the node's agents are read.
const EVERY: Duration = Duration::from_secs(60);
/// How often the account's Talk switch is read (`talk status`), and how long it may take: npx's first run fetches
/// the CLI.
const STATUS_EVERY: Duration = Duration::from_secs(600);
const STATUS_TIMEOUT: Duration = Duration::from_secs(90);
/// A listener that ends is started again after this, doubling each time it ends again, to AGAIN_MAX; one that ran
/// HEALTHY or delivered a message starts the wait afresh.
const AGAIN_FIRST: Duration = Duration::from_secs(5);
const AGAIN_MAX: Duration = Duration::from_secs(300);
const HEALTHY: Duration = Duration::from_secs(120);
/// A post is tried this many times, this far apart.
const POST_TRIES: u32 = 3;
const POST_AGAIN: Duration = Duration::from_secs(10);
/// A session id as Spacesheep's CLI takes one (lib/talk.js's SESSION_RE).
fn session_ok(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c))
}

/// Tests only: every wait above, this many times shorter (POINTMAN_SPACESHEEP_FASTER).
fn scaled(d: Duration) -> Duration {
    static FASTER: OnceLock<f64> = OnceLock::new();
    let f = *FASTER.get_or_init(|| {
        std::env::var("POINTMAN_SPACESHEEP_FASTER")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|f| *f >= 1.0)
            .unwrap_or(1.0)
    });
    d.div_f64(f)
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// An agent to listen for: its handle, its session, and the thread of its latest run (where messages go).
#[derive(Debug, Clone, PartialEq)]
struct Agent {
    handle: String,
    session: String,
    thread: i64,
}

/// The agents Talk is on for: the `agents` setting's handles (`@` or not), each once. None: Talk is off.
fn listed(settings: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for h in settings["agents"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let h = h.trim().trim_start_matches('@').to_lowercase();
        if words::handle_ok(&h) && !out.contains(&h) {
            out.push(h);
        }
    }
    out
}

/// The listed agents to listen for, from `agents.sessions`, one per session; and why the others aren't.
fn wanted(answer: &Value, listed: &[String]) -> (Vec<Agent>, Vec<String>) {
    let (mut agents, mut waiting) = (Vec::<Agent>::new(), Vec::new());
    let here: Vec<&Value> = answer["agents"].as_array().into_iter().flatten().collect();
    for handle in listed {
        let Some(a) = here.iter().find(|a| a["handle"].as_str().map(|h| h.trim_start_matches('@')) == Some(handle))
        else {
            waiting.push(format!("@{handle} isn't one of this node's agents, or hasn't run here yet"));
            continue;
        };
        let handle = handle.clone();
        let session = a["session_id"].as_str().unwrap_or("");
        let thread = a["thread_id"].as_i64().filter(|t| *t > 0);
        let why = match (session, thread) {
            ("", _) => Some(format!("@{handle} has no session yet")),
            (s, _) if !session_ok(s) => Some(format!("@{handle}'s session id {s:?} isn't one Spacesheep takes")),
            (_, None) => Some(format!("@{handle} has no thread yet")),
            _ if agents.iter().any(|x| x.session == session) => Some(format!("@{handle} shares a session")),
            _ => None,
        };
        match (why, thread) {
            (None, Some(thread)) => agents.push(Agent { handle, session: session.to_string(), thread }),
            (why, _) => waiting.push(why.unwrap_or_default()),
        }
    }
    (agents, waiting)
}

/// The account's Talk switch, from `spacesheep talk status`.
#[derive(Debug, Clone, Default, PartialEq)]
enum Account {
    #[default]
    Unknown,
    On,
    Off(String),
    SignedOut(String),
    Failed(String),
}

fn account(ran: &std::io::Result<cli::Ran>) -> Account {
    let ran = match ran {
        Ok(ran) => ran,
        Err(e) => return Account::Failed(e.to_string()),
    };
    let v: Value = serde_json::from_str(ran.stdout.trim()).unwrap_or(Value::Null);
    match ran.code {
        Some(3) => Account::SignedOut(ran.stderr.clone()),
        Some(0) if v["enabled"] == true => Account::On,
        Some(0) if v["pro"] == false => Account::Off("Talk needs a Pro or Team plan on spacesheep.dev".into()),
        Some(0) if v["enabled"] == false => {
            let url = v["settings_url"].as_str().filter(|u| u.starts_with("https://spacesheep.dev/"));
            Account::Off(format!(
                "Talk is off for the Spacesheep account, so a page's Session tab can't send: turn on 'Talk to your \
                 sessions' at {} (it's account-wide)",
                url.unwrap_or("https://spacesheep.dev/settings/talk")
            ))
        }
        code => Account::Failed(format!(
            "`spacesheep talk status` said {:?} (exit {})",
            if ran.stderr.is_empty() { ran.stdout.trim() } else { &ran.stderr },
            code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into())
        )),
    }
}

/// One listener, as health and the event say.
#[derive(Debug, Clone, Default)]
struct Listening {
    handle: String,
    thread: i64,
    running: bool,
    runs: u32,
    problem: Option<String>,
}

/// What health says: the last look's, and each listener's.
#[derive(Debug, Clone, Default)]
struct Status {
    looked: bool,
    /// The `agents` setting's handles: Talk is on while there's one.
    listed: Vec<String>,
    cli: Option<String>,
    problem: Option<String>,
    listeners: BTreeMap<String, Listening>,
    waiting: Vec<String>,
    account: Account,
    account_at: Option<f64>,
    /// The last message: when, and for whom.
    last: Option<(f64, String)>,
    /// The last post's problem, when it didn't go out.
    posted: Option<String>,
    /// A listener was stopped by Spacesheep (Talk off, its link gone): the next look reads the account's switch.
    recheck: bool,
}

/// The listeners' process groups, by session, kept in the state folder for the next start.
struct Groups {
    path: PathBuf,
    pids: Mutex<BTreeMap<String, u32>>,
}

impl Groups {
    fn set(&self, session: &str, pid: Option<u32>) {
        let mut pids = lock(&self.pids);
        match pid {
            Some(pid) => pids.insert(session.to_string(), pid),
            None => pids.remove(session),
        };
        let _ = std::fs::write(&self.path, json!(*pids).to_string());
    }

    /// Stops the groups a run before this one left (a crash, a kill), and forgets them.
    fn stop_leftovers(&self, h: &Handle) {
        let left: BTreeMap<String, u32> =
            std::fs::read_to_string(&self.path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        for (session, pid) in left {
            if cli::stop_leftover(pid) {
                h.log(format!("stopped a listener left from before, for session {session} (pid {pid})"));
            }
        }
        let _ = std::fs::write(&self.path, "{}");
    }
}

/// A message to post: in this thread, these words.
struct Post {
    thread: i64,
    handle: String,
    text: String,
}

/// What a listener's thread shares with the look that started it.
struct Line {
    agent: Mutex<Agent>,
    stop: AtomicBool,
    /// The running CLI's pid, its group's: set and cleared under this lock, so a stop never misses one.
    pid: Mutex<Option<u32>>,
}

impl Line {
    fn stop(&self) {
        let pid = lock(&self.pid);
        self.stop.store(true, Ordering::SeqCst);
        if let Some(pid) = *pid {
            // SAFETY: kill(2) on the group the listener's CLI leads; its pid is cleared before it's reaped
            unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
        }
    }
}

/// The first message's JSON goes in the log once, so we learn its shape (Spacesheep doesn't document it).
static FIRST: AtomicBool = AtomicBool::new(true);

/// Changes a listener's part of the status, while it has one (a stopped listener's is gone).
fn update(status: &Mutex<Status>, session: &str, f: impl FnOnce(&mut Listening)) {
    if let Some(l) = lock(status).listeners.get_mut(session) {
        f(l)
    }
}

/// One listener's life: `talk listen` for its session, its messages posted, and started again when it ends.
fn listen(h: Handle, cli: Cli, line: Arc<Line>, posts: Sender<Post>, status: Arc<Mutex<Status>>, groups: Arc<Groups>) {
    let session = lock(&line.agent).session.clone();
    let mut wait = scaled(AGAIN_FIRST);
    while !line.stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        let (ended, delivered) = match cli.command(&["talk", "listen", "--session", &session]).spawn() {
            Ok(child) => run_once(&h, child, &line, &posts, &status, &groups),
            Err(e) => (format!("couldn't start {}: {e}", cli.describe()), false),
        };
        if line.stop.load(Ordering::SeqCst) {
            break;
        }
        if delivered || started.elapsed() >= scaled(HEALTHY) {
            wait = scaled(AGAIN_FIRST);
        }
        let handle = lock(&line.agent).handle.clone();
        h.log(format!("the listener for @{handle} ended ({ended}): again in {:.0} s", wait.as_secs_f64()));
        update(&status, &session, |l| {
            l.running = false;
            l.problem = Some(ended.clone());
        });
        let until = Instant::now() + wait;
        while Instant::now() < until && !line.stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20).min(wait));
        }
        wait = (wait * 2).min(scaled(AGAIN_MAX));
    }
}

/// One `talk listen`, to its end: why it ended, and whether it delivered anything.
fn run_once(
    h: &Handle,
    mut child: Child,
    line: &Line,
    posts: &Sender<Post>,
    status: &Mutex<Status>,
    groups: &Groups,
) -> (String, bool) {
    let session = lock(&line.agent).session.clone();
    {
        let mut pid = lock(&line.pid);
        *pid = Some(child.id());
        groups.set(&session, Some(child.id()));
        if line.stop.load(Ordering::SeqCst) {
            // SAFETY: as in Line::stop
            unsafe { libc::kill(-(child.id() as libc::pid_t), libc::SIGTERM) };
        }
    }
    update(status, &session, |l| {
        l.running = true;
        l.runs += 1;
    });
    let mut stderr = child.stderr.take().expect("piped");
    let erring = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stderr.read_to_end(&mut s);
        String::from_utf8_lossy(&s).into_owned()
    });
    let (mut reader, mut stopped, mut delivered) = (Reader::default(), None, false);
    for text in BufReader::new(child.stdout.take().expect("piped")).lines() {
        let Ok(text) = text else { break };
        for said in reader.line(&text) {
            match said {
                Said::Message { text, raw } => {
                    let agent = lock(&line.agent).clone();
                    if FIRST.swap(false, Ordering::SeqCst) {
                        h.log(format!("the first message's JSON, as talk listen printed it: {raw}"));
                    }
                    if text.trim().is_empty() {
                        h.log(format!("a message for @{} with no words: not posted", agent.handle));
                        continue;
                    }
                    h.log(format!(
                        "a message for @{} ({} characters): posting it in thread {}",
                        agent.handle,
                        text.chars().count(),
                        agent.thread
                    ));
                    lock(status).last = Some((now(), agent.handle.clone()));
                    let text = words::post(&agent.handle, &text, &agent.session);
                    let _ = posts.send(Post { thread: agent.thread, handle: agent.handle, text });
                    delivered = true;
                    update(status, &session, |l| l.problem = None);
                }
                Said::Stopped { why } => stopped = Some(why),
                Said::Other(text) => h.log(format!("talk listen printed something that isn't JSON: {text}")),
            }
        }
    }
    // its stdout has closed: it's ending, so its pid stops being a stop's to signal before it's reaped
    *lock(&line.pid) = None;
    cli::stop(&mut child, Duration::from_secs(5));
    groups.set(&session, None);
    let code = child.wait().ok().and_then(|s| s.code());
    let said = cli::last_words(&erring.join().unwrap_or_default());
    let by_spacesheep = stopped.is_some();
    let ended = match (stopped, code) {
        (Some(why), _) => why,
        (None, Some(3)) => format!("this machine isn't signed in to Spacesheep ({said})"),
        (None, code) => format!(
            "talk listen exited {}{}",
            code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into()),
            if said.is_empty() { String::new() } else { format!(": {said}") }
        ),
    };
    if code == Some(3) {
        lock(status).account = Account::SignedOut(said);
    } else if by_spacesheep {
        lock(status).recheck = true;
    }
    (ended, delivered)
}

/// Posts go out one at a time, in order, each tried POST_TRIES times.
fn poster(h: Handle, status: Arc<Mutex<Status>>) -> Sender<Post> {
    let (tx, rx) = mpsc::channel::<Post>();
    std::thread::spawn(move || {
        for post in rx {
            let mut last = None;
            for attempt in 0..POST_TRIES {
                if attempt > 0 {
                    std::thread::sleep(scaled(POST_AGAIN));
                }
                match h.request("board.post", json!({"thread_id": post.thread, "text": post.text})) {
                    Ok(answer) => {
                        h.log(format!(
                            "posted @{}'s message in thread {}: message {}",
                            post.handle, post.thread, answer["message_id"]
                        ));
                        last = None;
                        break;
                    }
                    Err(e) => last = Some(format!("{e:#}")),
                }
            }
            if let Some(e) = &last {
                h.log(format!("couldn't post @{}'s message in thread {}: {e}", post.handle, post.thread));
            }
            lock(&status).posted =
                last.map(|e| format!("a message for @{} didn't reach thread {}: {e}", post.handle, post.thread));
        }
    });
    tx
}

/// The listeners running, by session: shared with main, which stops them all once the node has gone.
type Lines = Arc<Mutex<BTreeMap<String, Arc<Line>>>>;

struct Talk {
    h: Handle,
    next: Instant,
    lines: Lines,
    status: Arc<Mutex<Status>>,
    posts: Sender<Post>,
    groups: Option<Arc<Groups>>,
    checking: Arc<AtomicBool>,
    checked: Option<Instant>,
    /// The settings changed: look at the next tick, and read the account's switch again. An atomic, since it's set
    /// on the thread that reads the node's answers, which a look waits on.
    look_now: Arc<AtomicBool>,
}

impl Talk {
    /// The state folder (initialize's, so not before the first tick), and the listeners left from before.
    fn groups(&mut self) -> Arc<Groups> {
        if let Some(g) = &self.groups {
            return g.clone();
        }
        let dir = self.h.node()["state_dir"].as_str().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let g = Arc::new(Groups { path: dir.join("listeners.json"), pids: Mutex::default() });
        g.stop_leftovers(&self.h);
        self.groups = Some(g.clone());
        g
    }

    /// Each second: the event (sent only when it's changed), and a look when one is due.
    fn tick(&mut self) {
        let groups = self.groups();
        self.event();
        let started = Instant::now();
        if self.look_now.swap(false, Ordering::SeqCst) {
            self.next = started;
            self.checked = None;
        }
        if started < self.next {
            return;
        }
        self.next = started + scaled(EVERY);
        let listed = listed(&self.h.settings());
        lock(&self.status).listed = listed.clone();
        if !listed.is_empty() {
            self.look(groups, &listed);
        } else {
            self.stop_all();
            let mut st = lock(&self.status);
            st.problem = None;
            st.waiting.clear();
        }
        lock(&self.status).looked = true;
        self.event();
    }

    fn look(&mut self, groups: Arc<Groups>, listed: &[String]) {
        let own = std::env::current_exe().ok();
        let places: Vec<PathBuf> = cli::PLACES.iter().map(PathBuf::from).collect();
        let Some(cli) = cli::find(std::env::var_os("PATH").as_deref(), &places, own.as_deref()) else {
            self.stop_all();
            let mut st = lock(&self.status);
            st.cli = None;
            st.problem = Some(
                "neither spacesheep nor Node.js's npx is on the node's PATH (or in /opt/homebrew/bin, /usr/local/bin): \
                 install Node.js on this machine"
                    .into(),
            );
            return;
        };
        lock(&self.status).cli = Some(cli.describe());
        let answer = match self.h.request("agents.sessions", json!({})) {
            Ok(answer) => answer,
            Err(e) => {
                // the listeners running keep going, and the next look tries again
                self.h.log(format!("agents.sessions: {e:#}"));
                lock(&self.status).problem =
                    Some(format!("the node didn't say its agents' sessions (agents.sessions): {e:#}"));
                return;
            }
        };
        let (agents, waiting) = wanted(&answer, listed);
        {
            let mut st = lock(&self.status);
            st.problem = None;
            st.waiting = waiting;
        }
        let gone: Vec<String> =
            lock(&self.lines).keys().filter(|s| agents.iter().all(|a| &a.session != *s)).cloned().collect();
        for session in gone {
            self.stop_one(&session);
        }
        for agent in agents {
            if let Some(line) = lock(&self.lines).get(&agent.session) {
                // the same session: its messages go to the agent's thread now
                *lock(&line.agent) = agent.clone();
                if let Some(l) = lock(&self.status).listeners.get_mut(&agent.session) {
                    l.handle = agent.handle.clone();
                    l.thread = agent.thread;
                }
                continue;
            }
            self.h.log(format!(
                "listening for @{} (session {}), posting in thread {}",
                agent.handle, agent.session, agent.thread
            ));
            let line = Arc::new(Line {
                agent: Mutex::new(agent.clone()),
                stop: AtomicBool::new(false),
                pid: Mutex::new(None),
            });
            lock(&self.status).listeners.insert(
                agent.session.clone(),
                Listening { handle: agent.handle.clone(), thread: agent.thread, ..Default::default() },
            );
            lock(&self.lines).insert(agent.session.clone(), line.clone());
            let (h, cli, posts, status, groups) =
                (self.h.clone(), cli.clone(), self.posts.clone(), self.status.clone(), groups.clone());
            std::thread::spawn(move || listen(h, cli, line, posts, status, groups));
        }
        self.check_account(cli);
    }

    /// The account's Talk switch, read now and then on a thread of its own.
    fn check_account(&mut self, cli: Cli) {
        let recheck = std::mem::take(&mut lock(&self.status).recheck);
        let due = recheck || self.checked.is_none_or(|at| at.elapsed() >= scaled(STATUS_EVERY));
        if !due || self.checking.swap(true, Ordering::SeqCst) {
            return;
        }
        self.checked = Some(Instant::now());
        let (h, status, checking) = (self.h.clone(), self.status.clone(), self.checking.clone());
        std::thread::spawn(move || {
            let got = account(&cli::run(&cli, &["talk", "status"], STATUS_TIMEOUT));
            if let Account::Failed(why) | Account::SignedOut(why) = &got {
                h.log(format!("talk status: {why}"));
            }
            let mut st = lock(&status);
            st.account = got;
            st.account_at = Some(now());
            checking.store(false, Ordering::SeqCst);
        });
    }

    fn stop_one(&mut self, session: &str) {
        let line = lock(&self.lines).remove(session);
        if let Some(line) = line {
            self.h.log(format!("not listening for @{} (session {session}) any more", lock(&line.agent).handle));
            line.stop();
        }
        lock(&self.status).listeners.remove(session);
    }

    fn stop_all(&mut self) {
        let sessions: Vec<String> = lock(&self.lines).keys().cloned().collect();
        for session in sessions {
            self.stop_one(&session);
        }
    }

    fn event(&self) {
        let st = lock(&self.status);
        let listening: Vec<&str> = st.listeners.values().filter(|l| l.running).map(|l| l.handle.as_str()).collect();
        self.h.event(
            "spacesheep.talk",
            json!({"on": !st.listed.is_empty(), "agents": st.listed, "listening": listening, "last_message_at": st.last.as_ref().map(|l| l.0)}),
        );
    }
}

const SETTING: &str = "`pointman config set node.extensions.spacesheep.agents '[\"scene\"]'` on this machine (or \
                       its settings in the app), with the handles of the agents a page may message";

/// The parts health shows: the node's switch, the account's, and the listeners.
fn health(st: &Status) -> Value {
    let part = |id: &str, kind: &str, name: &str, state: &str, needs: Vec<&str>, why: String| json!({"id": id, "kind": kind, "name": name, "state": state, "needs": needs, "why": why});
    let on = !st.listed.is_empty();
    let switch = match on {
        true => {
            let who: Vec<String> = st.listed.iter().map(|h| format!("@{h}")).collect();
            part("node", "switch", "Talk on this node", "ok", vec![], format!("on for {}", who.join(", ")))
        }
        false => part(
            "node",
            "switch",
            "Talk on this node",
            "stopped",
            vec![],
            format!("off: no agent is listed. Turn it on with {SETTING}"),
        ),
    };
    let when = st.account_at.map(|at| format!(" (checked at {})", clock(at))).unwrap_or_default();
    let (state, needs, why) = match (&st.account, on) {
        (_, false) => ("unknown", vec![], "not checked while Talk is off on this node".to_string()),
        (Account::Unknown, _) => ("unknown", vec![], "not checked yet".to_string()),
        (Account::On, _) => ("ok", vec![], format!("Talk is on for the account{when}")),
        (Account::Off(why), _) => ("failing", vec![], format!("{why}{when}")),
        (Account::SignedOut(said), _) => (
            "failing",
            vec!["auth"],
            format!(
                "this machine isn't signed in to Spacesheep{}",
                if said.is_empty() { String::new() } else { format!(": {said}") }
            ),
        ),
        (Account::Failed(why), _) => ("failing", vec![], format!("{why}{when}")),
    };
    let account = part("account", "account", "Spacesheep account's Talk", state, needs, why);
    let running: Vec<String> = st.listeners.values().filter(|l| l.running).map(|l| format!("@{}", l.handle)).collect();
    let failing: Vec<String> = st
        .listeners
        .values()
        .filter(|l| !l.running)
        .filter_map(|l| Some(format!("@{}: {}", l.handle, l.problem.as_deref()?)))
        .collect();
    let last = match &st.last {
        Some((at, handle)) => format!("; the last message was at {} for @{handle}", clock(*at)),
        None => "; no message yet".into(),
    };
    let (state, why) = if !on {
        ("stopped", "off on this node".to_string())
    } else if let Some(problem) = &st.problem {
        ("failing", problem.clone())
    } else if let Some(posted) = &st.posted {
        ("failing", posted.clone())
    } else if !failing.is_empty() {
        (
            "failing",
            format!(
                "{}{}",
                failing.join("; "),
                if running.is_empty() { String::new() } else { format!("; listening for {}", running.join(", ")) }
            ),
        )
    } else if !running.is_empty() {
        ("ok", format!("listening for {}{last}", running.join(", ")))
    } else if !st.looked {
        ("unknown", "not looked yet".into())
    } else if !st.waiting.is_empty() {
        ("stopped", format!("nobody to listen for: {}", st.waiting.join("; ")))
    } else {
        ("stopped", "nobody to listen for".into())
    };
    let listeners = part("listeners", "listener", "Talk listeners", state, vec![], why);
    json!([switch, account, listeners])
}

fn main() -> anyhow::Result<()> {
    let mut ext = Extension::new()?;
    let h = ext.handle();
    let status = Arc::new(Mutex::new(Status::default()));
    let posts = poster(h.clone(), status.clone());
    let lines = Lines::default();
    let look_now = Arc::new(AtomicBool::new(false));
    let talk = Arc::new(Mutex::new(Talk {
        h: h.clone(),
        next: Instant::now(),
        lines: lines.clone(),
        status: status.clone(),
        posts,
        groups: None,
        checking: Arc::new(AtomicBool::new(false)),
        checked: None,
        look_now: look_now.clone(),
    }));

    let t = talk.clone();
    ext.every(TICK, move |_: &Handle| lock(&t).tick(), "talk");

    // the settings changed (the node may start it again instead): look at the next tick. Never the Talk lock here:
    // this runs on the thread that reads the node's answers, and a look holds that lock while it waits for one
    ext.on_settings(move |_: &Handle| look_now.store(true, Ordering::SeqCst));

    let st = status.clone();
    ext.method("health.check", move |_| Ok(health(&lock(&st))));
    let st = status.clone();
    ext.method("health.fix", move |params: Value| {
        let message = match (params["part"].as_str(), params["action"].as_str()) {
            (Some("account"), Some("auth")) => "Sign this machine in to Spacesheep yourself, in a terminal on it: \
                                                `npx -y spacesheep@latest login`, or `npx -y spacesheep@latest connect \
                                                <key>` where there's no browser, with a key from \
                                                https://spacesheep.dev/settings/api-keys (never posted)."
                .to_string(),
            _ => {
                let parts = health(&lock(&st));
                let why = parts
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|p| p["id"] == params["part"])
                    .map(|p| p["why"].clone());
                format!(
                    "Nothing to fix from here{}",
                    why.and_then(|w| w.as_str().map(|w| format!(": {w}"))).unwrap_or_default()
                )
            }
        };
        Ok(json!({"message": message}))
    });

    let done = ext.run();
    for line in lock(&lines).values() {
        line.stop();
    }
    done
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listed_agents_with_a_session_and_a_thread_are_listened_for() {
        let answer = json!({"agents": [
            {"handle": "scene", "client": "claude", "session_id": "s-1", "thread_id": 5},
            {"handle": "quiet", "client": "claude", "session_id": "s-2", "thread_id": 6},
            {"handle": "fresh", "client": "claude", "session_id": null, "thread_id": 7},
            {"handle": "nowhere", "client": "claude", "session_id": "s-4", "thread_id": null},
            {"handle": "odd", "client": "claude", "session_id": "s 5", "thread_id": 8},
            {"handle": "twin", "client": "claude", "session_id": "s-1", "thread_id": 9},
        ]});
        let settings =
            json!({"agents": ["scene", "@Fresh", "nowhere", "odd", "twin", "ghost", "scene", "not a handle"]});
        let names = listed(&settings);
        assert_eq!(names, ["scene", "fresh", "nowhere", "odd", "twin", "ghost"]);
        let (agents, waiting) = wanted(&answer, &names);
        assert_eq!(agents, [Agent { handle: "scene".into(), session: "s-1".into(), thread: 5 }], "not quiet: unlisted");
        assert_eq!(
            waiting,
            [
                "@fresh has no session yet",
                "@nowhere has no thread yet",
                "@odd's session id \"s 5\" isn't one Spacesheep takes",
                "@twin shares a session",
                "@ghost isn't one of this node's agents, or hasn't run here yet",
            ]
        );
        assert!(listed(&json!({})).is_empty() && listed(&json!({"agents": []})).is_empty(), "off by default");
    }

    #[test]
    fn the_accounts_switch_from_talk_status() {
        let ran = |code, stdout: &str, stderr: &str| {
            Ok(cli::Ran { code: Some(code), stdout: stdout.into(), stderr: stderr.into() })
        };
        assert_eq!(account(&ran(0, "{\n  \"pro\": true,\n  \"enabled\": true\n}", "")), Account::On);
        assert!(
            matches!(account(&ran(0, r#"{"pro": true, "enabled": false, "settings_url": "https://spacesheep.dev/settings/talk"}"#, "")), Account::Off(w) if w.contains("turn on 'Talk to your sessions' at https://spacesheep.dev/settings/talk"))
        );
        assert!(
            matches!(account(&ran(0, r#"{"pro": false, "enabled": false}"#, "")), Account::Off(w) if w.contains("Pro"))
        );
        assert!(matches!(account(&ran(3, "", "not signed in")), Account::SignedOut(_)));
        assert!(matches!(account(&ran(1, "", "fetch failed")), Account::Failed(w) if w.contains("fetch failed")));
    }
}
