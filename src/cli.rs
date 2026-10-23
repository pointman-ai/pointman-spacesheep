//! Spacesheep's CLI: where it is, and running it. The machine's own `spacesheep` when it has one, else
//! `npx -y spacesheep@latest`, signed in with the machine's own key (~/.config/spacesheep, the CLI's). Each run is
//! the leader of a process group of its own, so stopping one stops npx's node under it too.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Where a CLI is looked for after the node's PATH: Homebrew's two folders (a launchd PATH may have neither).
pub const PLACES: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    /// `spacesheep`, or `npx`.
    pub program: PathBuf,
    /// What goes before the CLI's own words: `-y spacesheep@latest` for npx.
    pub prefix: Vec<String>,
    /// The PATH it runs with: the node's, with the program's own folder first, so its `node` is found.
    pub path: OsString,
}

impl Cli {
    /// What health says it is.
    pub fn describe(&self) -> String {
        match self.prefix.is_empty() {
            true => format!("spacesheep at {}", self.program.display()),
            false => format!("npx -y spacesheep@latest ({})", self.program.display()),
        }
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
    let (program, prefix) = match found("spacesheep") {
        Some(p) => (p, vec![]),
        None => (found("npx")?, vec!["-y".to_string(), "spacesheep@latest".to_string()]),
    };
    let mut path: Vec<PathBuf> = program.parent().map(Path::to_path_buf).into_iter().collect();
    path.extend(path_var.map(|p| std::env::split_paths(p).collect::<Vec<_>>()).unwrap_or_default());
    path.dedup();
    let path = std::env::join_paths(path).unwrap_or_else(|_| path_var.unwrap_or_default().to_owned());
    Some(Cli { program, prefix, path })
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

/// A short run's end: its exit code, stdout, and its stderr's last words.
#[derive(Debug, Clone, Default)]
pub struct Ran {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs the CLI once and waits up to `timeout` for it (`talk status`).
pub fn run(cli: &Cli, args: &[&str], timeout: Duration) -> std::io::Result<Ran> {
    let mut child = cli.command(args).spawn()?;
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
    Ok(Ran { code, stdout, stderr: last_words(&stderr) })
}

/// Whether this CLI's `talk reply` takes `--to`, the message it answers (CLI pull 55: its help says
/// `[--to MESSAGE_ID]`). Without it Spacesheep answers the session's newest message.
pub fn answers_named(help: &Ran) -> bool {
    help.stdout.contains("--to MESSAGE_ID") || help.stderr.contains("--to MESSAGE_ID")
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
        assert_eq!((npx.program, npx.prefix), (a.join("npx"), vec!["-y".to_string(), "spacesheep@latest".to_string()]));
        make(&b.join("spacesheep"));
        let own = find(Some(&path), &[], None).unwrap();
        assert_eq!((own.program.clone(), own.prefix.is_empty()), (b.join("spacesheep"), true));
        assert_eq!(own.path, std::env::join_paths([&b, &a, &b]).unwrap(), "its own folder first, for its node");
        // this extension's binary is called spacesheep too: never that one
        let skipped = find(Some(&path), &[], Some(&b.join("spacesheep"))).unwrap();
        assert_eq!(skipped.program, a.join("npx"));
        assert!(find(Some(&std::env::join_paths([&b]).unwrap()), &[], Some(&b.join("spacesheep"))).is_none());
    }
}
