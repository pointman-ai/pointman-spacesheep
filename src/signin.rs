//! Signing a machine in from the vault (EXT-132). `/spacesheep add` seals one Spacesheep API key in the vault as
//! `spacesheep.key`. A machine that isn't signed in uses it to mint a key of its own, named after the machine, and
//! keeps that one: `spacesheep keys create`, then `spacesheep keys save`. So one add signs in every node, a new one
//! too, and each machine's key can be revoked on spacesheep.dev without touching the others'.
//!
//! The vault's key goes only into the first run's environment (`SPACESHEEP_KEY`, which the CLI reads before its
//! login file), and the new key only onto the second run's stdin: neither is ever a word of a command, which anyone
//! on the machine can read in `ps`, nor in the log, a post or health. What the CLI prints on stdout is never repeated.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use crate::cli::{self, Cli, Ran};
use crate::words::now;

/// The vault item `/spacesheep add` seals (extension.toml's `[[credentials]]`).
pub const ITEM: &str = "spacesheep.key";

/// A key's fingerprint, so a key changed in the vault is tried at once and a refused one isn't tried again and again.
/// Never the key, and never logged.
pub fn fingerprint(key: &str) -> u64 {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish()
}

/// The name a machine's key goes by on spacesheep.dev (Settings › API keys) and in the CLI's `whoami`: the
/// machine's, as the CLI's own `connect` names it, clipped as the CLI clips one.
pub fn key_name(machine: &str) -> String {
    let name: String = machine.trim().chars().take(60).collect();
    if name.is_empty() {
        "Pointman node".into()
    } else {
        name
    }
}

/// A key minted for this machine that `keys save` didn't keep: held in memory only, never written down, and saved at
/// the next try instead of minting another, so a save that fails leaves no key after key unused on spacesheep.dev.
pub struct Unsaved {
    /// The fingerprint of the vault key it was minted with: a different vault key mints its own.
    pub from: u64,
    /// Its name on spacesheep.dev.
    pub name: String,
    /// When it was minted, in seconds since the epoch.
    pub at: f64,
    key: String,
}

impl std::fmt::Debug for Unsaved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Unsaved").field("name", &self.name).field("at", &self.at).finish_non_exhaustive()
    }
}

/// Why a machine isn't signed in yet, in the CLI's words with any key in them cut (`scrub`).
#[derive(Debug, Clone, PartialEq)]
pub enum NotYet {
    /// No key was minted: the vault's key was refused, or the CLI couldn't run.
    Mint(String),
    /// A key was minted and not kept: it's in `unsaved`, for the next try.
    Save(String),
}

/// Signs this machine in with the vault's `key`: a key of its own named `name`, minted and kept in the CLI's login
/// file. A key minted with this vault key before and not kept (`unsaved`) is saved now instead of minting another;
/// one this try mints and can't keep is left there.
pub fn sign_in(
    cli: &Cli,
    key: &str,
    name: &str,
    unsaved: &mut Option<Unsaved>,
    timeout: Duration,
) -> Result<(), NotYet> {
    let from = fingerprint(key);
    let new = match unsaved.take() {
        Some(kept) if kept.from == from => kept,
        _ => {
            let created = cli::run_with(
                cli,
                &["keys", "create", "--name", name, "--json"],
                &[("SPACESHEEP_KEY", key)],
                None,
                timeout,
            )
            .map_err(|e| NotYet::Mint(scrub(&e.to_string())))?;
            Unsaved { from, name: name.into(), at: now(), key: minted(&created).map_err(NotYet::Mint)? }
        }
    };
    let saved =
        cli::run_with(cli, &["keys", "save", "--name", &new.name], &[], Some(&format!("{}\n", new.key)), timeout)
            .map_err(|e| scrub(&e.to_string()))
            .and_then(|saved| match saved.code {
                Some(0) => Ok(()),
                code => Err(failed("keys save", code, &saved)),
            });
    saved.map_err(|why| {
        *unsaved = Some(new);
        NotYet::Save(why)
    })
}

/// The key `keys create --json` minted, or why there isn't one.
pub fn minted(ran: &Ran) -> Result<String, String> {
    if ran.code != Some(0) {
        return Err(failed("keys create", ran.code, ran));
    }
    let printed: serde_json::Value = serde_json::from_str(ran.stdout.trim())
        .map_err(|_| "`spacesheep keys create --json` didn't print JSON".to_string())?;
    printed["key"]
        .as_str()
        .map(str::trim)
        .filter(|k| k.starts_with("ss_") && k.len() > 3)
        .map(String::from)
        .ok_or_else(|| "`spacesheep keys create --json` printed no key".into())
}

/// A run that failed, in the CLI's words: its exit and its stderr's last words (the CLI's `✗` line), never stdout,
/// and any key in them cut.
fn failed(what: &str, code: Option<i32>, ran: &Ran) -> String {
    format!(
        "`spacesheep {what}` exited {}{}",
        code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into()),
        if ran.stderr.is_empty() { String::new() } else { format!(": {}", scrub(&ran.stderr)) }
    )
}

/// `text` with each Spacesheep key in it (`ss_` and what follows, at the start of a word) cut to `ss_…`, so an error
/// that repeats a key never reaches health, the log or a post.
pub fn scrub(text: &str) -> String {
    let word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("ss_") {
        let after = &rest[i + 3..];
        let tail = after.find(|c: char| !word(c)).unwrap_or(after.len());
        let starts_word = !rest[..i].chars().next_back().or_else(|| out.chars().next_back()).is_some_and(word);
        out.push_str(&rest[..i + 3]);
        if starts_word && tail > 0 {
            out.push('…');
            rest = &after[tail..];
        } else {
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ran(code: i32, stdout: &str, stderr: &str) -> Ran {
        Ran { code: Some(code), stdout: stdout.into(), stderr: stderr.into(), npm: String::new() }
    }

    #[test]
    fn the_minted_key_is_read_from_keys_creates_json() {
        let printed = r#"{"key": "ss_0123abcd", "name": "mac-m4", "scope": "full"}"#;
        assert_eq!(minted(&ran(0, printed, "")).unwrap(), "ss_0123abcd");
    }

    #[test]
    fn a_failed_create_says_the_clis_words_and_never_its_stdout() {
        let why = minted(&ran(1, "ss_must_not_show", "the server rejected that key")).unwrap_err();
        assert_eq!(why, "`spacesheep keys create` exited 1: the server rejected that key");
        let why = minted(&ran(0, "ss_must_not_show", "")).unwrap_err();
        assert!(!why.contains("ss_must_not_show"), "{why}");
        assert!(minted(&ran(0, r#"{"key": "nope"}"#, "")).is_err());
        assert!(minted(&ran(0, r#"{"key": "ss_"}"#, "")).is_err());
    }

    #[test]
    fn keys_in_the_clis_words_are_cut() {
        assert_eq!(scrub("✗ could not check ss_0123abcdEF: fetch failed"), "✗ could not check ss_…: fetch failed");
        assert_eq!(scrub("\"ss_a-b_c\" and ss_x."), "\"ss_…\" and ss_….");
        let words = "process_exit pass_ss_x class_y ss_ nothing";
        assert_eq!(scrub(words), words, "only a word that starts ss_ is a key");
        let why = minted(&ran(1, "", "the server rejected ss_0123456789abcdef")).unwrap_err();
        assert_eq!(why, "`spacesheep keys create` exited 1: the server rejected ss_…");
    }

    #[test]
    fn an_unsaved_key_never_prints() {
        let kept = Unsaved { from: 1, name: "mac-m4".into(), at: 0.0, key: "ss_must_not_show".into() };
        assert!(!format!("{kept:?}").contains("ss_must_not_show"));
    }

    #[test]
    fn a_keys_name_is_the_machines() {
        assert_eq!(key_name("mac-m4"), "mac-m4");
        assert_eq!(key_name("  "), "Pointman node");
        assert_eq!(key_name(&"x".repeat(80)).len(), 60);
    }

    #[test]
    fn a_fingerprint_tells_keys_apart_without_being_one() {
        assert_eq!(fingerprint("ss_a"), fingerprint("ss_a"));
        assert_ne!(fingerprint("ss_a"), fingerprint("ss_b"));
    }
}
