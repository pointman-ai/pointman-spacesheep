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

/// Signs this machine in with the vault's `key`: a key of its own named `name`, minted and kept in the CLI's login
/// file. Why not, in the CLI's words, when it can't.
pub fn sign_in(cli: &Cli, key: &str, name: &str, timeout: Duration) -> Result<(), String> {
    let created =
        cli::run_with(cli, &["keys", "create", "--name", name, "--json"], &[("SPACESHEEP_KEY", key)], None, timeout)
            .map_err(|e| e.to_string())?;
    let minted = minted(&created)?;
    let saved = cli::run_with(cli, &["keys", "save", "--name", name], &[], Some(&format!("{minted}\n")), timeout)
        .map_err(|e| e.to_string())?;
    match saved.code {
        Some(0) => Ok(()),
        code => Err(failed("keys save", code, &saved)),
    }
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

/// A run that failed, in the CLI's words: its exit and its stderr's last words (the CLI's `✗` line), never stdout.
fn failed(what: &str, code: Option<i32>, ran: &Ran) -> String {
    format!(
        "`spacesheep {what}` exited {}{}",
        code.map(|c| c.to_string()).unwrap_or_else(|| "by a signal".into()),
        if ran.stderr.is_empty() { String::new() } else { format!(": {}", ran.stderr) }
    )
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
