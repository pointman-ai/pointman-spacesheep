//! The extension's process: the node starts it as extension.toml's `[run]` says (`bin/spacesheep`) and talks to
//! it over stdin and stdout (the SDK handles that). Log with `h.log(…)`: stdout is the node's.
//!
//! Every second on the `talk` lane: the account's Talk switch is read every minute (`talk status`), the pages and
//! who published them every five while it's on (`list` and `versions`, pages.rs), and a look is due every minute
//! or as soon as either changes. A look, while the account's Talk is on: the node's `agents.sessions`, then a
//! listener for each agent whose session published a page, with a thread, and none for the rest. A listener is a
//! thread running `spacesheep talk listen --session <id>`, started again with a growing wait whenever it ends. Each
//! message goes out as a `board.post` in the publisher's thread, for the agents it starts by calling or else the
//! publisher, in order, from a thread of their own. The listeners' process groups are kept in the state folder, so
//! a restart stops any left behind.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use pointman_extension::protocol::{
    AgentsSessionsRequest, AgentsSessionsResponse, BoardPostRequest, HealthCheckRequest, HealthCheckResponse,
    HealthFixRequest, HealthFixResponse,
};
use pointman_extension::v1::health::{part::State, Action, Part};
use pointman_extension::v1::{member_ref, ThreadId};
use pointman_extension::{Extension, Handle};
use serde_json::{json, Value};
use spacesheep::cli::{self, Cli};
use spacesheep::pages::{self, Known};
use spacesheep::said::{self, Reader, Said};
use spacesheep::words::{self, clock, now};

/// How often a look is checked for.
const TICK: Duration = Duration::from_secs(1);
/// How often the node's agents are read.
const EVERY: Duration = Duration::from_secs(60);
/// How often the account's Talk switch is read (`talk status`): it's the one switch, so a minute after it's turned
/// on the listeners start. And how long it may take: npx's first run fetches the CLI.
const STATUS_EVERY: Duration = Duration::from_secs(60);
const STATUS_TIMEOUT: Duration = Duration::from_secs(90);
/// The pages of agents that have finished (despawned, or moved on to a new session) listened on, at most: the
/// pages changed most recently. Each listener is a process.
const GONE_PAGES: usize = 10;
/// How long a reply on a page may take.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the pages are read for who published them, and how long each read may take.
const PAGES_EVERY: Duration = Duration::from_secs(300);
const PAGES_TIMEOUT: Duration = Duration::from_secs(60);
/// How often the CLI's help is read for whether `talk reply` takes `--to` (npx's spacesheep@latest moves on its own).
const HELP_EVERY: Duration = Duration::from_secs(3600);
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
    /// The page's agent has finished: its session runs nowhere, so only an agent the message calls can answer.
    gone: bool,
}

/// What's remembered of each session an agent here has had: its handle and latest thread, kept in the state folder
/// (agents.json), so a page still has a name and a thread after its agent has gone.
type Remembered = BTreeMap<String, (String, i64)>;

fn remembered_from(text: &str) -> Remembered {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let rows =
        v.as_object().into_iter().flatten().filter_map(|(s, a)| {
            Some((s.clone(), (a["handle"].as_str()?.to_string(), a["thread"].as_i64().unwrap_or(0))))
        });
    rows.collect()
}

fn remembered_json(r: &Remembered) -> String {
    let m: serde_json::Map<String, Value> =
        r.iter().map(|(s, (h, t))| (s.clone(), json!({"handle": h, "thread": t}))).collect();
    Value::Object(m).to_string()
}

/// `agents.sessions`' answer as the JSON [`remember`] and [`wanted`] read: each agent's `handle`, `session_id` and
/// `thread_id` (the thread's number, 0 for none).
fn sessions(answer: &AgentsSessionsResponse) -> Value {
    let agents: Vec<Value> = answer
        .agents
        .iter()
        .map(|a| {
            let handle = match a.handle.as_option().and_then(|h| h.kind.as_ref()) {
                Some(member_ref::Kind::Handle(h)) => h.clone(),
                _ => String::new(),
            };
            let thread = a.thread.as_option().and_then(|t| t.value.strip_prefix("thr_")?.parse::<i64>().ok());
            json!({"handle": handle, "client": a.client, "session_id": a.session_id, "thread_id": thread.unwrap_or(0)})
        })
        .collect();
    json!({"agents": agents})
}

/// Each agent here with a session and a thread, as `agents.sessions` gives them, into what's remembered; and a
/// session forgotten once it's neither an agent's here nor among the latest pages' publishers (patchbay's review).
/// `publishers` is None until the pages have been read without a problem: until then nothing is forgotten, since an
/// empty list at a start would forget every finished agent's page.
fn remember(r: &mut Remembered, answer: &Value, publishers: Option<&[String]>) -> bool {
    let current: Vec<&str> =
        answer["agents"].as_array().into_iter().flatten().filter_map(|a| a["session_id"].as_str()).collect();
    let before = r.len();
    if let Some(publishers) = publishers {
        r.retain(|s, _| current.contains(&s.as_str()) || publishers.contains(s));
    }
    let mut changed = r.len() != before;
    for a in answer["agents"].as_array().into_iter().flatten() {
        let handle = a["handle"].as_str().unwrap_or("").trim_start_matches('@');
        let (Some(session), Some(thread)) = (a["session_id"].as_str(), a["thread_id"].as_i64().filter(|t| *t > 0))
        else {
            continue;
        };
        if words::handle_ok(handle) && session_ok(session) && r.get(session) != Some(&(handle.to_string(), thread)) {
            r.insert(session.to_string(), (handle.to_string(), thread));
            changed = true;
        }
    }
    changed
}

/// The sessions to listen for: each agent here whose current session published a page, from `agents.sessions`;
/// then the pages whose agent here has finished (a session an agent here once had, so never another node's, or the
/// person's own), the most recently changed first, up to GONE_PAGES, as remembered. Also why a publisher here isn't listened for, and how many agents here haven't published a page in their
/// current session.
fn wanted(answer: &Value, publishers: &[String], remembered: &Remembered) -> (Vec<Agent>, Vec<String>, usize) {
    let (mut agents, mut waiting, mut others) = (Vec::<Agent>::new(), Vec::new(), 0);
    let current: Vec<&str> =
        answer["agents"].as_array().into_iter().flatten().filter_map(|a| a["session_id"].as_str()).collect();
    for a in answer["agents"].as_array().into_iter().flatten() {
        let handle = a["handle"].as_str().unwrap_or("").trim_start_matches('@').to_string();
        let session = a["session_id"].as_str().unwrap_or("");
        if !publishers.iter().any(|p| p == session) {
            others += 1;
            continue;
        }
        let thread = a["thread_id"].as_i64().filter(|t| *t > 0);
        let why = match (words::handle_ok(&handle), thread) {
            (false, _) => Some(format!("{:?} isn't a handle", a["handle"])),
            _ if !session_ok(session) => Some(format!("@{handle}'s session id {session:?} isn't one Spacesheep takes")),
            (_, None) => Some(format!("@{handle} published a page but has no thread yet")),
            _ if agents.iter().any(|x| x.session == session) => Some(format!("@{handle} shares a session")),
            _ => None,
        };
        match (why, thread) {
            (None, Some(thread)) => agents.push(Agent { handle, session: session.to_string(), thread, gone: false }),
            (why, _) => waiting.push(why.unwrap_or_default()),
        }
    }
    let gone = publishers
        .iter()
        .filter(|p| !current.contains(&p.as_str()))
        .filter_map(|p| Some((p, remembered.get(p)?)))
        .take(GONE_PAGES);
    for (session, (handle, thread)) in gone {
        agents.push(Agent { handle: handle.clone(), session: session.clone(), thread: *thread, gone: true });
    }
    (agents, waiting, others)
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
    /// On a finished agent's page.
    gone: bool,
    thread: i64,
    running: bool,
    runs: u32,
    problem: Option<String>,
}

/// What health says: the last look's, and each listener's.
#[derive(Debug, Clone, Default)]
struct Status {
    looked: bool,
    cli: Option<String>,
    problem: Option<String>,
    listeners: BTreeMap<String, Listening>,
    waiting: Vec<String>,
    /// Agents here whose current session hasn't published a page.
    others: usize,
    /// The sessions that published a page, the most recently changed page's first, as last read; when, and what
    /// went wrong reading them.
    publishers: Vec<String>,
    pages_at: Option<f64>,
    pages_problem: Option<String>,
    account: Account,
    account_at: Option<f64>,
    /// The last message: when, and for whom.
    last: Option<(f64, String)>,
    /// The last post's problem, when it didn't go out.
    posted: Option<String>,
    /// A listener was stopped by Spacesheep (Talk off, its link gone): the next look reads the account's switch.
    recheck: bool,
    /// The CLI's `talk reply` takes `--to`, the message it answers, as its help last said; and when it was read.
    answers_named: bool,
    helped_at: Option<f64>,
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

/// A message to post: in this thread, for these agents (`@editor`, as the log says), these words.
struct Post {
    thread: i64,
    to: String,
    text: String,
    /// the page it was written on, at the comment (EXT-83): a chip under the post that opens it
    link: Option<String>,
    context: String,
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
            Ok(child) => run_once(&h, &cli, child, &line, &posts, &status, &groups),
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
    cli: &Cli,
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
                        h.log(format!("a message on {} with no words: not posted", page_of(&agent)));
                        continue;
                    }
                    deliver(h, cli, &agent, &text, &raw, posts, status);
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

/// A page as the log says it: `@scene's page`, or a finished agent's.
fn page_of(agent: &Agent) -> String {
    match agent.gone {
        false => format!("@{}'s page", agent.handle),
        true => format!("@{}'s page (finished)", agent.handle),
    }
}

/// A message on `agent`'s page goes to whom it's for: the agents it starts by calling, else the page's agent, in
/// the page's thread. A finished agent can't answer, so its page's message reaches only the other agents it calls;
/// one that calls nobody else gets a reply on the page saying how to ask.
fn deliver(h: &Handle, cli: &Cli, agent: &Agent, text: &str, raw: &str, posts: &Sender<Post>, status: &Mutex<Status>) {
    let (text, pin) = said::comment_parts(text, raw);
    // the message the answer is for, when the CLI can say so (EXT-73)
    let answers = if lock(status).answers_named { said::message_id(raw) } else { None };
    let mut to = words::recipients(text, &agent.handle);
    if agent.gone {
        to.retain(|t| *t != agent.handle);
    }
    if to.is_empty() {
        h.log(format!("a message on {} nobody here can take: answering on the page", page_of(agent)));
        let answer = format!(
            "{} has finished, so no agent answers this page now. To ask another, start a new comment with \
             @session @pm.<agent>, like @session @pm.search.",
            agent.handle
        );
        let to = answers.map(|id| id.to_string());
        let mut args = vec!["talk", "reply", "--session", agent.session.as_str()];
        if let Some(to) = &to {
            args.extend(["--to", to.as_str()]);
        }
        args.push(answer.as_str());
        match cli::run(cli, &args, REPLY_TIMEOUT) {
            Ok(r) if r.code == Some(0) => {}
            Ok(r) => h.log(format!("the reply on {} didn't go: {}", page_of(agent), r.stderr)),
            Err(e) => h.log(format!("the reply on {} didn't go: {e}", page_of(agent))),
        }
        return;
    }
    let who = to.iter().map(|t| format!("@{t}")).collect::<Vec<_>>().join(" ");
    h.log(format!(
        "a message on {} for {who} ({} characters): posting it in thread {}",
        page_of(agent),
        text.chars().count(),
        agent.thread
    ));
    lock(status).last = Some((now(), who.clone()));
    let post = words::post(&to, &agent.handle, text, said::asked_in(raw).as_deref());
    let context = words::context(&agent.session, answers, said::redelivered(raw), pin);
    let _ = posts.send(Post { thread: agent.thread, to: who, text: post, link: said::link(raw), context });
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
                let ask = BoardPostRequest {
                    thread: ThreadId { value: format!("thr_{}", post.thread), ..Default::default() }.into(),
                    text: post.text.clone(),
                    blocks: vec![serde_json::from_value(json!({"text": {"markdown": post.context}}))
                        .expect("text block encodes")],
                    links: post.link.iter().cloned().collect(),
                    ..Default::default()
                };
                match h.request(&ask) {
                    Ok(answer) => {
                        h.log(format!(
                            "posted the message for {} in thread {}: message {}",
                            post.to, post.thread, answer.post.value
                        ));
                        last = None;
                        break;
                    }
                    Err(e) => last = Some(format!("{e:#}")),
                }
            }
            if let Some(e) = &last {
                h.log(format!("couldn't post the message for {} in thread {}: {e}", post.to, post.thread));
            }
            lock(&status).posted =
                last.map(|e| format!("a message for {} didn't reach thread {}: {e}", post.to, post.thread));
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
    state: PathBuf,
    remembered: Remembered,
    checking: Arc<AtomicBool>,
    checked: Option<Instant>,
    scanning: Arc<AtomicBool>,
    scanned: Option<Instant>,
    /// The account's switch or the publishers changed (on their own threads): look at the next tick.
    look_now: Arc<AtomicBool>,
}

impl Talk {
    /// The state folder (initialize's, so not before the first tick), and the listeners left from before.
    fn groups(&mut self) -> Arc<Groups> {
        if let Some(g) = &self.groups {
            return g.clone();
        }
        let dir = Some(self.h.state_dir()).filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(std::env::temp_dir);
        self.state = dir.clone();
        self.remembered = remembered_from(&std::fs::read_to_string(dir.join("agents.json")).unwrap_or_default());
        let g = Arc::new(Groups { path: dir.join("listeners.json"), pids: Mutex::default() });
        g.stop_leftovers(&self.h);
        self.groups = Some(g.clone());
        g
    }

    /// Each second: the event (sent only when it's changed), the account's switch and the pages when they're due,
    /// and a look when one is.
    fn tick(&mut self) {
        let groups = self.groups();
        self.event();
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
        self.check_account(&cli);
        if lock(&self.status).account == Account::On {
            self.read_pages(&cli);
        }
        let started = Instant::now();
        if self.look_now.swap(false, Ordering::SeqCst) {
            self.next = started;
        }
        if started < self.next {
            return;
        }
        self.next = started + scaled(EVERY);
        self.look(groups, cli);
        lock(&self.status).looked = true;
        self.event();
    }

    fn look(&mut self, groups: Arc<Groups>, cli: Cli) {
        let account = lock(&self.status).account.clone();
        match account {
            Account::On => {}
            // the account couldn't be read: what runs keeps going, and the next read decides
            Account::Failed(_) => return,
            // Talk off for the account, the machine signed out, or not read yet: nobody to listen for
            _ => {
                self.stop_all();
                lock(&self.status).problem = None;
                return;
            }
        }
        let answer = match self.h.request(&AgentsSessionsRequest::default()) {
            Ok(answer) => sessions(&answer),
            Err(e) => {
                // the listeners running keep going, and the next look tries again
                self.h.log(format!("agents.sessions: {e:#}"));
                lock(&self.status).problem =
                    Some(format!("the node didn't say its agents' sessions (agents.sessions): {e:#}"));
                return;
            }
        };
        let (publishers, read) = {
            let st = lock(&self.status);
            (st.publishers.clone(), st.pages_at.is_some() && st.pages_problem.is_none())
        };
        if remember(&mut self.remembered, &answer, read.then_some(publishers.as_slice())) {
            let _ = std::fs::write(self.state.join("agents.json"), remembered_json(&self.remembered));
        }
        let (agents, waiting, others) = wanted(&answer, &publishers, &self.remembered);
        {
            let mut st = lock(&self.status);
            st.problem = None;
            st.waiting = waiting;
            st.others = others;
        }
        let gone: Vec<String> =
            lock(&self.lines).keys().filter(|s| agents.iter().all(|a| &a.session != *s)).cloned().collect();
        for session in gone {
            self.stop_one(&session);
        }
        for agent in agents {
            if let Some(line) = lock(&self.lines).get(&agent.session) {
                // the same session: its messages go to the agent's thread now, or it's a finished agent's page now
                let mut was = lock(&line.agent);
                if agent.gone && !was.gone {
                    self.h.log(format!("listening on {} (session {})", page_of(&agent), agent.session));
                }
                *was = agent.clone();
                drop(was);
                if let Some(l) = lock(&self.status).listeners.get_mut(&agent.session) {
                    l.handle = agent.handle.clone();
                    l.gone = agent.gone;
                    l.thread = agent.thread;
                }
                continue;
            }
            match agent.gone {
                false => self.h.log(format!(
                    "listening for @{} (session {}), posting in thread {}",
                    agent.handle, agent.session, agent.thread
                )),
                true => self.h.log(format!("listening on {} (session {})", page_of(&agent), agent.session)),
            }
            let line = Arc::new(Line {
                agent: Mutex::new(agent.clone()),
                stop: AtomicBool::new(false),
                pid: Mutex::new(None),
            });
            lock(&self.status).listeners.insert(
                agent.session.clone(),
                Listening {
                    handle: agent.handle.clone(),
                    gone: agent.gone,
                    thread: agent.thread,
                    ..Default::default()
                },
            );
            lock(&self.lines).insert(agent.session.clone(), line.clone());
            let (h, cli, posts, status, groups) =
                (self.h.clone(), cli.clone(), self.posts.clone(), self.status.clone(), groups.clone());
            std::thread::spawn(move || listen(h, cli, line, posts, status, groups));
        }
    }

    /// The account's Talk switch, read every minute on a thread of its own; a change means a look at once.
    fn check_account(&mut self, cli: &Cli) {
        let recheck = std::mem::take(&mut lock(&self.status).recheck);
        let due = recheck || self.checked.is_none_or(|at| at.elapsed() >= scaled(STATUS_EVERY));
        if !due || self.checking.swap(true, Ordering::SeqCst) {
            return;
        }
        self.checked = Some(Instant::now());
        let (h, status, checking, look_now, cli) =
            (self.h.clone(), self.status.clone(), self.checking.clone(), self.look_now.clone(), cli.clone());
        std::thread::spawn(move || {
            let got = account(&cli::run(&cli, &["talk", "status"], STATUS_TIMEOUT));
            if let Account::Failed(why) | Account::SignedOut(why) = &got {
                h.log(format!("talk status: {why}"));
            }
            let mut st = lock(&status);
            if st.account != got {
                h.log(format!("the account's Talk: {}", account_words(&got)));
                look_now.store(true, Ordering::SeqCst);
            }
            st.account = got;
            st.account_at = Some(now());
            checking.store(false, Ordering::SeqCst);
        });
    }

    /// Who published the account's pages, read every five minutes on a thread of its own: each page's versions only
    /// when it has changed (pages.json keeps what's known). New publishers mean a look at once.
    fn read_pages(&mut self, cli: &Cli) {
        if self.scanned.is_some_and(|at| at.elapsed() < scaled(PAGES_EVERY))
            || self.scanning.swap(true, Ordering::SeqCst)
        {
            return;
        }
        self.scanned = Some(Instant::now());
        let (h, status, scanning, look_now, cli, file) = (
            self.h.clone(),
            self.status.clone(),
            self.scanning.clone(),
            self.look_now.clone(),
            cli.clone(),
            self.state.join("pages.json"),
        );
        std::thread::spawn(move || {
            let due = lock(&status).helped_at.is_none_or(|at| now() - at >= scaled(HELP_EVERY).as_secs_f64());
            if due {
                let named = cli::run(&cli, &["--help"], PAGES_TIMEOUT).ok().map(|r| cli::answers_named(&r));
                let mut st = lock(&status);
                st.helped_at = Some(now());
                if let Some(named) = named.filter(|n| *n != st.answers_named) {
                    h.log(format!(
                        "talk reply {} the message it answers (--to)",
                        if named { "names" } else { "can't name" }
                    ));
                    st.answers_named = named;
                }
            }
            let got = publishers(&h, &cli, &file);
            let mut st = lock(&status);
            match got {
                Ok(who) => {
                    if who != st.publishers {
                        look_now.store(true, Ordering::SeqCst);
                    }
                    st.publishers = who;
                    st.pages_at = Some(now());
                    st.pages_problem = None;
                }
                Err(why) => {
                    h.log(format!("reading the pages: {why}"));
                    st.pages_problem = Some(why);
                }
            }
            scanning.store(false, Ordering::SeqCst);
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
        let listening: Vec<&str> =
            st.listeners.values().filter(|l| l.running && !l.gone).map(|l| l.handle.as_str()).collect();
        self.h.event(
            "spacesheep.talk",
            json!({"on": st.account == Account::On, "listening": listening, "last_message_at": st.last.as_ref().map(|l| l.0)}),
        );
    }
}

/// Who published the account's latest pages (`list --json`), each page's versions read only when it has changed.
fn publishers(h: &Handle, cli: &Cli, file: &std::path::Path) -> Result<Vec<String>, String> {
    let ran = cli::run(cli, &["list", "--json"], PAGES_TIMEOUT).map_err(|e| e.to_string())?;
    let list = match ran.code {
        Some(0) => pages::spaces(&ran.stdout)
            .ok_or_else(|| format!("`spacesheep list` said {:?}", cli::last_words(&ran.stdout)))?,
        _ => return Err(format!("`spacesheep list` failed: {}", ran.stderr)),
    };
    let mut known = Known::from_json(&std::fs::read_to_string(file).unwrap_or_default());
    known.keep(&list, now() as i64);
    let stale: Vec<&pages::Space> = list.iter().filter(|s| known.stale(s)).collect();
    for space in stale {
        let ran = cli::run(cli, &["versions", &space.id, "--json"], PAGES_TIMEOUT);
        match ran.as_ref().ok().filter(|r| r.code == Some(0)).and_then(|r| pages::sessions(&r.stdout)) {
            Some(who) => {
                known.0.insert(space.id.clone(), (space.updated_at.clone(), who));
            }
            None => h.log(format!(
                "couldn't read who published page {}: {}",
                space.id,
                match ran {
                    Ok(r) => r.stderr,
                    Err(e) => e.to_string(),
                }
            )),
        }
    }
    let _ = std::fs::write(file, known.to_json());
    Ok(known.publishers())
}

/// The account's switch as the log says it.
fn account_words(a: &Account) -> String {
    match a {
        Account::Unknown => "not read yet".into(),
        Account::On => "on".into(),
        Account::Off(why) | Account::Failed(why) => why.clone(),
        Account::SignedOut(_) => "this machine isn't signed in to Spacesheep".into(),
    }
}

/// The parts health shows: the account's switch, which is Talk's one switch, and the listeners.
fn health(st: &Status) -> Vec<Part> {
    let part = |id: &str, kind: &str, name: &str, state: &str, needs: Vec<&str>, why: String| Part {
        id: id.to_string(),
        kind: kind.to_string(),
        name: Some(name.to_string()),
        state: match state {
            "ok" => State::STATE_OK,
            "failing" => State::STATE_FAILING,
            "stopped" => State::STATE_STOPPED,
            _ => State::STATE_UNSPECIFIED,
        }
        .into(),
        needs: needs.iter().filter(|n| **n == "auth").map(|_| Action::ACTION_AUTH.into()).collect(),
        why: Some(why),
        ..Default::default()
    };
    let on = st.account == Account::On;
    let when = st.account_at.map(|at| format!(" (checked at {})", clock(at))).unwrap_or_default();
    let (state, needs, why) = match &st.account {
        Account::Unknown => ("unknown", vec![], "not checked yet".to_string()),
        Account::On => ("ok", vec![], format!("Talk is on for the account{when}")),
        Account::Off(why) => ("stopped", vec![], format!("{why}{when}")),
        Account::SignedOut(said) => (
            "failing",
            vec!["auth"],
            format!(
                "this machine isn't signed in to Spacesheep{}",
                if said.is_empty() { String::new() } else { format!(": {said}") }
            ),
        ),
        Account::Failed(why) => ("failing", vec![], format!("{why}{when}")),
    };
    let account = part("account", "account", "Spacesheep account's Talk", state, needs, why);
    let running: Vec<String> =
        st.listeners.values().filter(|l| l.running && !l.gone).map(|l| format!("@{}", l.handle)).collect();
    let finished = st.listeners.values().filter(|l| l.running && l.gone).count();
    let finished = match finished {
        0 => String::new(),
        1 => ", and on 1 page whose agent has finished".into(),
        n => format!(", and on {n} pages whose agents have finished"),
    };
    let failing: Vec<String> = st
        .listeners
        .values()
        .filter(|l| !l.running)
        .filter_map(|l| {
            let who = match (l.handle.is_empty(), l.gone) {
                (false, false) => format!("@{}", l.handle),
                (false, true) => format!("@{}'s page (finished)", l.handle),
                (true, _) => "a finished agent's page".into(),
            };
            Some(format!("{who}: {}", l.problem.as_deref()?))
        })
        .collect();
    let last = match &st.last {
        Some((at, to)) => format!("; the last message was at {} for {to}", clock(*at)),
        None => "; no message yet".into(),
    };
    let (state, why) = if !on && running.is_empty() {
        ("stopped", format!("not listening: {}", account_words(&st.account)))
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
    } else if let Some(why) = &st.pages_problem {
        ("failing", format!("who published the pages couldn't be read: {why}"))
    } else if !running.is_empty() {
        ("ok", format!("listening for {} (whose sessions published pages){finished}{last}", running.join(", ")))
    } else if !finished.is_empty() {
        ("ok", format!("listening{}{last}", finished.trim_start_matches(", and")))
    } else if !st.looked || st.pages_at.is_none() {
        ("unknown", "not looked yet".into())
    } else if !st.waiting.is_empty() {
        ("stopped", format!("nobody to listen for: {}", st.waiting.join("; ")))
    } else {
        (
            "stopped",
            format!(
                "nobody to listen for: none of the {} agents here has published a page in its current session",
                st.others
            ),
        )
    };
    let listeners = part("listeners", "listener", "Talk listeners", state, vec![], why);
    vec![account, listeners]
}

fn main() -> anyhow::Result<()> {
    let mut ext = Extension::new()?;
    let h = ext.handle();
    let status = Arc::new(Mutex::new(Status::default()));
    let posts = poster(h.clone(), status.clone());
    let lines = Lines::default();
    let talk = Arc::new(Mutex::new(Talk {
        h: h.clone(),
        next: Instant::now(),
        lines: lines.clone(),
        status: status.clone(),
        posts,
        groups: None,
        state: PathBuf::new(),
        remembered: Remembered::new(),
        checking: Arc::new(AtomicBool::new(false)),
        checked: None,
        scanning: Arc::new(AtomicBool::new(false)),
        scanned: None,
        look_now: Arc::new(AtomicBool::new(false)),
    }));

    let t = talk.clone();
    ext.every(TICK, move |_: &Handle| lock(&t).tick(), "talk");

    let st = status.clone();
    ext.on(move |_: HealthCheckRequest| Ok(HealthCheckResponse { parts: health(&lock(&st)), ..Default::default() }));
    let st = status.clone();
    ext.on(move |fix: HealthFixRequest| {
        let message = match (fix.part.as_str(), fix.action.as_known()) {
            ("account", Some(Action::ACTION_AUTH)) => {
                "Sign this machine in to Spacesheep yourself, in a terminal on it: \
                                                `npx -y spacesheep@latest login`, or `npx -y spacesheep@latest connect \
                                                <key>` where there's no browser, with a key from \
                                                https://spacesheep.dev/settings/api-keys (never posted)."
                    .to_string()
            }
            _ => {
                let parts = health(&lock(&st));
                let why = parts.into_iter().find(|p| p.id == fix.part).and_then(|p| p.why);
                format!("Nothing to fix from here{}", why.map(|w| format!(": {w}")).unwrap_or_default())
            }
        };
        Ok(HealthFixResponse { message, ..Default::default() })
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
    fn only_agents_whose_session_published_a_page_are_listened_for() {
        let answer = json!({"agents": [
            {"handle": "scene", "client": "claude", "session_id": "s-1", "thread_id": 5},
            {"handle": "quiet", "client": "claude", "session_id": "s-2", "thread_id": 6},
            {"handle": "fresh", "client": "claude", "session_id": null, "thread_id": 7},
            {"handle": "nowhere", "client": "claude", "session_id": "s-4", "thread_id": null},
            {"handle": "odd", "client": "claude", "session_id": "s 5", "thread_id": 8},
            {"handle": "twin", "client": "claude", "session_id": "s-1", "thread_id": 9},
        ]});
        // s-gone: docrules', here once; s-nixos: an agent alive on another node; s-james: James's own session
        let publishers: Vec<String> = ["s-1", "s-4", "s 5", "s-gone", "s-nixos", "s-james"].map(String::from).into();
        let remembered = Remembered::from([("s-gone".to_string(), ("docrules".to_string(), 41))]);
        let (agents, waiting, others) = wanted(&answer, &publishers, &remembered);
        assert_eq!(
            agents,
            [
                Agent { handle: "scene".into(), session: "s-1".into(), thread: 5, gone: false },
                Agent { handle: "docrules".into(), session: "s-gone".into(), thread: 41, gone: true },
            ],
            "not quiet: no page; s-gone: a finished agent's page, as remembered; not another node's or James's"
        );
        assert_eq!(
            waiting,
            [
                "@nowhere published a page but has no thread yet",
                "@odd's session id \"s 5\" isn't one Spacesheep takes",
                "@twin shares a session",
            ]
        );
        assert_eq!(others, 2, "quiet and fresh");
        assert!(wanted(&answer, &[], &Remembered::new()).0.is_empty());
        let many: Vec<String> = (0..20).map(|i| format!("gone-{i}")).collect();
        let all: Remembered = many.iter().map(|s| (s.clone(), ("old".to_string(), 7))).collect();
        let (agents, _, _) = wanted(&answer, &many, &all);
        assert_eq!(agents.len(), GONE_PAGES, "finished agents' pages, the newest {GONE_PAGES} only");
        assert!(agents.iter().all(|a| a.gone) && agents[0].session == "gone-0");
        assert!(wanted(&answer, &many, &Remembered::new()).0.is_empty(), "nobody here ever had them");
    }

    #[test]
    fn each_agents_session_and_thread_is_remembered() {
        let mut r = Remembered::new();
        let answer = json!({"agents": [
            {"handle": "scene", "session_id": "s-1", "thread_id": 5},
            {"handle": "fresh", "session_id": "s-2", "thread_id": null},
            {"handle": "Bad Name", "session_id": "s-3", "thread_id": 6},
        ]});
        assert!(remember(&mut r, &answer, Some(&[])));
        assert!(!remember(&mut r, &answer, Some(&[])), "nothing new the second time");
        assert_eq!(r, Remembered::from([("s-1".to_string(), ("scene".to_string(), 5))]));
        assert_eq!(remembered_from(&remembered_json(&r)), r);
        // scene moves on: its old session stays while it published a page, and goes once it's off the latest pages
        let moved = json!({"agents": [{"handle": "scene", "session_id": "s-9", "thread_id": 5}]});
        remember(&mut r, &moved, Some(&["s-1".to_string()]));
        assert!(r.contains_key("s-1") && r.contains_key("s-9"));
        // before the pages are read (a start), nothing is forgotten
        remember(&mut r, &moved, None);
        assert!(r.contains_key("s-1"), "kept until a page read says otherwise");
        assert!(remember(&mut r, &moved, Some(&[])));
        assert_eq!(r.keys().collect::<Vec<_>>(), ["s-9"]);
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
