//! Spacesheep's CLI: where it is, and running it. The machine's own `spacesheep` when it has one, else npx's copy of
//! `spacesheep@latest` run with node once npx has said where it is (`copy_from`), else `npx -y spacesheep@latest`;
//! signed in with the machine's own key (~/.config/spacesheep, the CLI's; signin.rs makes one from the vault's). Each run is the leader of a process group
//! of its own, so stopping one stops npx's node under it too.

use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Where a CLI is looked for after the node's PATH: Homebrew's two folders (a launchd PATH may have neither).
pub const PLACES: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// How the CLI runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// The machine's own `spacesheep`.
    Own,
    /// `npx -y spacesheep@latest`: an `npm exec` (~100 MB) stays under each run, for as long as it runs.
    Npx,
    /// npx's copy of `spacesheep@latest`, run with node: only the CLI's own node runs (EXT-127).
    Copy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub via: Via,
    /// `spacesheep`, `npx`, or `node` for npx's copy.
    pub program: PathBuf,
    /// What goes before the CLI's own words: `-y spacesheep@latest` for npx, the copy's script for node.
    pub prefix: Vec<String>,
    /// The PATH it runs with: the node's, with the program's own folder first, so its `node` is found.
    pub path: OsString,
    /// npx's copy's version, from its package.json: npx brings the copy up to date where it is, so a new version
    /// is a new `Cli`, and the log says so.
    pub version: Option<String>,
}

impl Cli {
    /// What health says it is.
    pub fn describe(&self) -> String {
        match self.via {
            Via::Own => format!("spacesheep at {}", self.program.display()),
            Via::Npx => format!("npx -y spacesheep@latest ({})", self.program.display()),
            Via::Copy => format!(
                "npx's spacesheep@latest{} at {}, run with {}",
                self.version.as_ref().map(|v| format!(" ({v})")).unwrap_or_default(),
                self.prefix.first().map(String::as_str).unwrap_or_default(),
                self.program.display()
            ),
        }
    }

    /// Whether it can still start: npx's copy goes when npm's cache is cleaned.
    pub fn there(&self) -> bool {
        executable(&self.program)
            && (self.via != Via::Copy || self.prefix.first().is_some_and(|s| Path::new(s).is_file()))
    }

    /// The CLI with these words, in a process group of its own, its stdin closed and the rest piped.
    pub fn command<S: AsRef<OsStr>>(&self, args: &[S]) -> Command {
        let mut c = Command::new(&self.program);
        c.args(&self.prefix).args(args).env("PATH", &self.path).process_group(0);
        c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }
}

fn executable(p: &Path) -> bool {
    p.is_absolute() && p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `spacesheep` on the PATH or in `places`, else `npx` there. Never `own`, this extension's binary, which has the
/// same name.
pub fn find(path_var: Option<&OsStr>, places: &[PathBuf], own: Option<&Path>) -> Option<Cli> {
    find_with(path_var, places, own, true)
}

/// As `find`; with `installed` false the machine's own `spacesheep` is passed over (it is too old, see `too_old`).
pub fn find_with(path_var: Option<&OsStr>, places: &[PathBuf], own: Option<&Path>, installed: bool) -> Option<Cli> {
    let dirs: Vec<PathBuf> = path_var
        .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(places.iter().cloned())
        .collect();
    let own = own.and_then(|p| p.canonicalize().ok());
    let found = |name: &str| {
        dirs.iter().map(|d| d.join(name)).find(|p| executable(p) && (own.is_none() || p.canonicalize().ok() != own))
    };
    let (via, program, prefix) = match found("spacesheep").filter(|_| installed) {
        Some(p) => (Via::Own, p, vec![]),
        None => (Via::Npx, found("npx")?, vec!["-y".to_string(), "spacesheep@latest".to_string()]),
    };
    let mut path: Vec<PathBuf> = program.parent().map(Path::to_path_buf).into_iter().collect();
    path.extend(path_var.map(|p| std::env::split_paths(p).collect::<Vec<_>>()).unwrap_or_default());
    path.dedup();
    let path = std::env::join_paths(path).unwrap_or_else(|_| path_var.unwrap_or_default().to_owned());
    Some(Cli { via, program, prefix, path, version: None })
}

/// npx's copy of `spacesheep@latest`, fetched or brought up to date first as any npx run does: the CLI's script, as
/// npx's `.bin` names it, run with the `node` npx's own runs would find (its PATH, npx's folder first). The same
/// CLI as npx runs, with no `npm exec` left under each run.
pub fn copy_from(npx: &Cli, timeout: Duration) -> Result<Cli, String> {
    let ask = Cli { prefix: vec![], ..npx.clone() };
    let ran = run(&ask, &["-y", "-p", "spacesheep@latest", "-c", "command -v spacesheep"], timeout)
        .map_err(|e| e.to_string())?;
    let script = ran.stdout.lines().map(str::trim).rfind(|l| l.starts_with('/')).map(PathBuf::from);
    let script = match (ran.code, script) {
        (Some(0), Some(s)) if s.is_file() => s,
        (Some(0), Some(s)) => return Err(format!("npx named {}, which isn't there", s.display())),
        (Some(0), None) => return Err("npx didn't name its copy".into()),
        (code, _) => {
            let code = code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into());
            let said = [&ran.stderr, &ran.npm].into_iter().find(|s| !s.is_empty());
            return Err(format!("npx exited {code}{}", said.map(|s| format!(": {s}")).unwrap_or_default()));
        }
    };
    let program = std::env::split_paths(&npx.path)
        .map(|d| d.join("node"))
        .find(|p| executable(p))
        .ok_or_else(|| "no node beside npx or on the node's PATH".to_string())?;
    // .bin/spacesheep links to the package's bin/spacesheep.js; its package.json is beside bin
    let version = script
        .canonicalize()
        .ok()
        .and_then(|s| Some(s.parent()?.parent()?.join("package.json")))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["version"].as_str().map(String::from));
    Ok(Cli {
        via: Via::Copy,
        program,
        prefix: vec![script.to_string_lossy().into_owned()],
        path: npx.path.clone(),
        version,
    })
}

/// Stops a run and everything it started: SIGTERM to its group, then SIGKILL if its leader is still there after
/// `grace`. Only signalled before the leader is reaped, so the group's id is never anyone else's.
pub fn stop(child: &mut Child, grace: Duration) {
    let group = child.id() as libc::pid_t;
    // SAFETY: kill(2) with a negative pid signals that process group; the group is the child's own
    unsafe { libc::kill(-group, libc::SIGTERM) };
    let until = Instant::now() + grace;
    while Instant::now() < until {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    unsafe { libc::kill(-group, libc::SIGKILL) };
    let _ = child.wait();
}

/// Stops a group left from before a restart: only when its leader is still a `talk listen`, so a pid used again
/// since is never touched.
pub fn stop_leftover(pid: u32) -> bool {
    let Ok(out) = Command::new("ps").args(["-o", "command=", "-p", &pid.to_string()]).output() else {
        return false;
    };
    let command = String::from_utf8_lossy(&out.stdout);
    if !(command.contains("spacesheep") && command.contains("talk listen")) {
        return false;
    }
    // SAFETY: as in stop: the group a run of ours led
    unsafe { libc::kill(-(pid as libc::pid_t), libc::SIGTERM) };
    true
}

/// A short run's end: its exit code, stdout, and its stderr's last words; and npm's own last error, for a run of npx
/// itself (`last_words` leaves npm's lines out).
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub npm: String,
}

/// Runs the CLI once and waits up to `timeout` for it (`talk status`).
pub fn run(cli: &Cli, args: &[&str], timeout: Duration) -> std::io::Result<Ran> {
    run_with(cli, args, &[], None, timeout)
}

/// As `run`, with these in its environment and this on its stdin: how a key reaches the CLI without ever being one
/// of its words, which anyone on the machine can read in `ps` (EXT-132).
pub fn run_with(
    cli: &Cli,
    args: &[&str],
    env: &[(&str, &str)],
    input: Option<&str>,
    timeout: Duration,
) -> std::io::Result<Ran> {
    let mut command = cli.command(args);
    command.envs(env.iter().copied());
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    let mut child = command.spawn()?;
    if let Some(text) = input {
        // a line far smaller than a pipe's buffer, so this never waits on the CLI; closed after, so it reads its end
        let mut stdin = child.stdin.take().expect("piped");
        let _ = stdin.write_all(text.as_bytes());
    }
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let reading = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = out.read_to_end(&mut s);
        s
    });
    let erring = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = err.read_to_end(&mut s);
        s
    });
    let until = Instant::now() + timeout;
    let code = loop {
        match child.try_wait()? {
            Some(status) => break status.code(),
            None if Instant::now() >= until => {
                stop(&mut child, Duration::from_secs(2));
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("`spacesheep {}` didn't answer in {} s", args.join(" "), timeout.as_secs()),
                ));
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    };
    let stdout = String::from_utf8_lossy(&reading.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&erring.join().unwrap_or_default()).into_owned();
    let npm = stderr.lines().map(str::trim).rfind(|l| l.starts_with("npm error")).unwrap_or_default().to_string();
    Ok(Ran { code, stdout, stderr: last_words(&stderr), npm })
}

/// The oldest CLI the extension is written for: 1.36 reads the server's own table of calls and falls back to
/// today's tools without it, so a machine's older `spacesheep` is replaced by npx's current one.
pub const MIN_CLI: (u32, u32, u32) = (1, 36, 0);

/// The version a `--version` answer names (`1.36.0`, `spacesheep 1.36.0`): the first three dotted numbers.
pub fn version_in(text: &str) -> Option<(u32, u32, u32)> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|w| {
        let mut n = w.split('.').map(|p| p.parse::<u32>().ok());
        Some((n.next()??, n.next()??, n.next()??))
    })
}

/// Whether this installed CLI is older than `MIN_CLI`. A version it can't read is not too old: that is a CLI
/// the extension knows nothing about, and health says what it ran.
pub fn too_old(version_answer: &Ran) -> bool {
    let text = if version_answer.stdout.trim().is_empty() { &version_answer.stderr } else { &version_answer.stdout };
    version_in(text).is_some_and(|v| v < MIN_CLI)
}

/// Whether this CLI's `talk reply` takes `--to`, the message it answers (CLI pull 55: its help says
/// `[--to MESSAGE_ID]`). Without it Spacesheep answers the session's newest message.
pub fn answers_named(help: &Ran) -> bool {
    help.stdout.contains("--to MESSAGE_ID") || help.stderr.contains("--to MESSAGE_ID")
}

/// What the CLI's help prints when it doesn't know a command word (it shows its own top-level usage, at exit 0 or 1).
const NO_SUCH_COMMAND: &str = "publish web pages to spacesheep.dev";

/// Whether the server's table names this command (`spaces list`), which is what the CLI's own help says: the
/// command's usage when it's there, its top-level usage when it isn't. Only that decides: a run that can't be made or
/// that doesn't answer is an error, never "absent", so a server having trouble is not read as one without the call.
pub fn serves(cli: &Cli, command: &[&str], timeout: Duration) -> Result<bool, String> {
    let mut args = command.to_vec();
    args.push("--help");
    let ran = run(cli, &args, timeout).map_err(|e| e.to_string())?;
    // The top-level help is the CLI's answer for a command it doesn't know, at whatever exit status it gives: 1 for
    // `spaces list` (no such command), 0 for `versions list` (the old `versions <space>` takes it). Only that text says
    // "absent"; other failures, which print none of it, stay errors.
    if ran.stdout.contains(NO_SUCH_COMMAND) && matches!(ran.code, Some(0 | 1)) {
        return Ok(false);
    }
    match ran.code {
        Some(0) => Ok(true),
        _ => Err(format!("`spacesheep {} --help` failed: {}", command.join(" "), last_words(&ran.stderr))),
    }
}

/// A failed run's reason: the ErrorInfo the new commands print on stdout with `--json` (`{code, message}`, exit 1,
/// also for a 401), else the CLI's own error on stderr.
pub fn refusal(ran: &Ran) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(ran.stdout.trim()) {
        let e = if v["error"].is_object() { &v["error"] } else { &v };
        if let Some(m) = e["message"].as_str().filter(|m| !m.is_empty()) {
            return match e["code"].as_str() {
                Some(c) if !c.is_empty() => format!("{c}: {m}"),
                _ => m.to_string(),
            };
        }
    }
    let said = last_words(&ran.stderr);
    if said.is_empty() {
        format!("exit {}", ran.code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into()))
    } else {
        said
    }
}

/// What a CLI said on stderr worth repeating, clipped: its error (`✗ not signed in — …`), else its last line. npx's
/// own notices (`npm notice …`, `npm warn …`) come after it, so they're never it.
pub fn last_words(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with("npm ")).collect();
    let line = lines.iter().rfind(|l| l.starts_with('✗')).or(lines.last()).copied().unwrap_or("");
    let line = line.trim_start_matches('✗').trim();
    match line.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_words_are_the_clis_error_not_npms_notices() {
        let stderr = "\n  ✗ not signed in — run `spacesheep login`\n\nnpm notice\nnpm notice New major version of npm available!\n";
        assert_eq!(last_words(stderr), "not signed in — run `spacesheep login`");
        assert_eq!(last_words("  listening for messages…\n  fetch failed\n"), "fetch failed");
        assert_eq!(last_words(""), "");
    }

    #[test]
    fn spacesheep_is_found_before_npx_and_never_this_binary() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let make = |p: &Path| {
            std::fs::write(p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        make(&a.join("npx"));
        let path = std::env::join_paths([&a, &b]).unwrap();
        let npx = find(Some(&path), &[], None).unwrap();
        assert_eq!(npx.via, Via::Npx);
        assert_eq!((npx.program, npx.prefix), (a.join("npx"), vec!["-y".to_string(), "spacesheep@latest".to_string()]));
        make(&b.join("spacesheep"));
        let own = find(Some(&path), &[], None).unwrap();
        assert_eq!((own.via, own.program.clone(), own.prefix.is_empty()), (Via::Own, b.join("spacesheep"), true));
        assert_eq!(own.path, std::env::join_paths([&b, &a, &b]).unwrap(), "its own folder first, for its node");
        // this extension's binary is called spacesheep too: never that one
        let skipped = find(Some(&path), &[], Some(&b.join("spacesheep"))).unwrap();
        assert_eq!(skipped.program, a.join("npx"));
        assert!(find(Some(&std::env::join_paths([&b]).unwrap()), &[], Some(&b.join("spacesheep"))).is_none());
    }

    #[test]
    fn an_installed_cli_older_than_the_minimum_is_too_old() {
        let said = |s: &str| Ran { stdout: s.into(), ..Ran::default() };
        assert_eq!(version_in("1.36.0\n"), Some((1, 36, 0)));
        assert_eq!(version_in("spacesheep 1.26.0"), Some((1, 26, 0)));
        assert_eq!(version_in("v2.0.1-beta"), Some((2, 0, 1)));
        assert_eq!(version_in("no version here"), None);
        assert!(too_old(&said("1.35.0")) && too_old(&said("spacesheep 1.26.0")));
        assert!(!too_old(&said("1.36.0")) && !too_old(&said("1.40.2")) && !too_old(&said("2.0.0")));
        assert!(!too_old(&said("")), "unreadable is not too old");
        assert!(too_old(&Ran { stderr: "1.9.0".into(), ..Ran::default() }), "a version on stderr counts");
    }

    #[test]
    fn a_too_old_installed_cli_is_passed_over_for_npx() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["spacesheep", "npx"] {
            let p = dir.path().join(name);
            std::fs::write(&p, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(find_with(Some(&path), &[], None, true).unwrap().via, Via::Own);
        let npx = find_with(Some(&path), &[], None, false).unwrap();
        assert_eq!((npx.via, npx.prefix), (Via::Npx, vec!["-y".to_string(), "spacesheep@latest".to_string()]));
    }

    #[test]
    fn npxs_copy_is_there_only_while_its_script_is() {
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("node");
        std::fs::write(&node, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        let script = dir.path().join("spacesheep.js");
        std::fs::write(&script, "").unwrap();
        let copy = Cli {
            via: Via::Copy,
            program: node,
            prefix: vec![script.to_string_lossy().into_owned()],
            path: dir.path().as_os_str().to_owned(),
            version: Some("1.35.0".into()),
        };
        assert!(copy.there());
        assert!(copy.describe().starts_with("npx's spacesheep@latest (1.35.0) at "), "{}", copy.describe());
        std::fs::remove_file(&script).unwrap();
        assert!(!copy.there(), "npm's cache cleaned");
    }

    /// `serves()` against spacesheep 1.36.0 itself and an old server (every path a 404, so no doors table): `spaces
    /// list --help` is the top-level help at exit 1, `versions list --help` the same at exit 0, and both are "not
    /// served". Runs when SPACESHEEP_CLI_1_36 is the CLI's bin/spacesheep.js and node is on the PATH (CI sets it).
    #[test]
    fn an_old_server_serves_neither_command_whatever_the_helps_exit_status() {
        use std::io::{Read, Write};
        let Some(script) = std::env::var_os("SPACESHEEP_CLI_1_36") else {
            eprintln!("skipped: set SPACESHEEP_CLI_1_36 to spacesheep 1.36.0's bin/spacesheep.js");
            return;
        };
        let node = std::env::var_os("SPACESHEEP_NODE").map(PathBuf::from).unwrap_or_else(|| {
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                .map(|d| d.join("node"))
                .find(|p| p.is_file())
                .expect("node on the PATH, or SPACESHEEP_NODE")
        });
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for mut conn in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    let _ = conn.read(&mut buf);
                    let body = r#"{"code":"not_found","message":"no such path"}"#;
                    let _ = write!(
                        conn,
                        "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        let home = tempfile::tempdir().unwrap();
        let cli = Cli {
            via: Via::Copy,
            program: node.clone(),
            prefix: vec![PathBuf::from(&script).to_string_lossy().into_owned()],
            path: node.parent().unwrap().into(),
            version: None,
        };
        let env: Vec<(&str, String)> = vec![
            ("HOME", home.path().to_string_lossy().into_owned()),
            ("SPACESHEEP_CONFIG_DIR", home.path().join("c").to_string_lossy().into_owned()),
            ("SPACESHEEP_ORIGIN", origin.clone()),
            ("SPACESHEEP_APP_ORIGIN", origin.clone()),
            ("SPACESHEEP_CONTENT_ORIGIN", origin),
            ("SPACESHEEP_NO_UPDATE_CHECK", "1".into()),
            ("SPACESHEEP_KEY", "synthetic-good-key-0000".into()),
        ];
        // serves() has no env of its own: set them for this process (this test is the only one reading them)
        for (k, v) in &env {
            std::env::set_var(k, v);
        }
        assert_eq!(serves(&cli, &["spaces", "list"], Duration::from_secs(60)), Ok(false));
        assert_eq!(serves(&cli, &["versions", "list"], Duration::from_secs(60)), Ok(false));
    }

    #[test]
    fn a_failed_help_that_is_not_the_top_level_help_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let sh = dir.path().join("spacesheep");
        std::fs::write(&sh, "#!/bin/sh\necho '✗ fetch failed' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cli = Cli { via: Via::Own, program: sh, prefix: vec![], path: "/usr/bin:/bin".into(), version: None };
        let err = serves(&cli, &["spaces", "list"], Duration::from_secs(10)).unwrap_err();
        assert!(err.contains("fetch failed"), "{err}");
        // the top-level help at exit 1 (what 1.36.0 does for `spaces`) is "not served"
        std::fs::write(
            cli.program.clone(),
            "#!/bin/sh\necho 'spacesheep 1.36.0 — publish web pages to spacesheep.dev'\nexit 1\n",
        )
        .unwrap();
        assert_eq!(serves(&cli, &["spaces", "list"], Duration::from_secs(10)), Ok(false));
    }
}
