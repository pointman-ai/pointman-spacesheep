//! A machine that isn't signed in signs itself in with the vault's key (EXT-132): a key of its own, named after the
//! machine, minted with the vault's key in the CLI's environment and kept from its stdin. Never a word of a command.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;

const VAULT_KEY: &str = "ss_0123456789abcdef0123456789abcdef";

/// What the node hands an extension for a vault item: the vault's whole record, not the bare key.
fn record(key: &str) -> String {
    format!(r#"{{"kind":"env","vars":{{"spacesheep.key":"{key}"}}}}"#)
}

#[test]
fn a_vault_record_with_no_ss_key_is_said_and_never_reaches_the_cli() {
    let fake = Fake::new();
    fake.signed_out();
    let board = Board::default();
    let node =
        spacesheep(&fake, &board, on()).secret("spacesheep.key", record("Bearer 0123456789abcdef")).start().unwrap();
    let mut health = json!(null);
    assert!(
        wait(WITHIN, || {
            health = node.call("health.check", json!({})).unwrap();
            part(&health, "account")["why"].as_str().is_some_and(|w| w.contains("holds no key starting with `ss_`"))
        }),
        "{health}"
    );
    assert_eq!(part(&health, "account")["state"], "STATE_FAILING", "{health}");
    assert!(fake.calls().iter().all(|c| !c.contains("keys ")), "{:?}", fake.calls());
    assert_eq!(fake.parent_key(), None);
}

#[test]
fn a_signed_out_machine_signs_itself_in_with_the_vaults_key() {
    let fake = Fake::new();
    fake.signed_out();
    let board = Board::default();
    let node =
        spacesheep(&fake, &board, on()).machine("mac-m4").secret("spacesheep.key", record(VAULT_KEY)).start().unwrap();

    assert!(wait(WITHIN, || fake.saved_key().is_some()), "kept a key: {:?}", fake.calls());
    assert_eq!(fake.parent_key().as_deref(), Some(VAULT_KEY), "minted with the vault's key, in SPACESHEEP_KEY");
    let saved = fake.saved_key().unwrap();
    assert!(saved.trim().starts_with("ss_minted") && saved.ends_with('\n'), "the minted key, on stdin: {saved:?}");
    let calls = fake.calls();
    assert!(calls.iter().any(|c| c.ends_with("keys create --name mac-m4 --json")), "{calls:?}");
    assert!(calls.iter().any(|c| c.ends_with("keys save --name mac-m4")), "{calls:?}");
    assert!(calls.iter().all(|c| !c.contains("ss_")), "no key is ever a word of a command: {calls:?}");

    assert!(
        wait(WITHIN, || {
            let health = node.call("health.check", json!({})).unwrap();
            part(&health, "account")["state"] == "STATE_OK"
        }),
        "signed in, the account reads"
    );
    let health = node.call("health.check", json!({})).unwrap();
    let why = part(&health, "account")["why"].as_str().unwrap().to_string();
    assert!(why.contains("signed in with the vault's spacesheep.key at"), "{why}");
    assert!(!why.contains(VAULT_KEY) && !why.contains("ss_minted"), "{why}");
    let creates = fake.calls().iter().filter(|c| c.contains("keys create")).count();
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(fake.calls().iter().filter(|c| c.contains("keys create")).count(), creates, "signed in: once");
}

#[test]
fn a_signed_in_machine_never_touches_the_vaults_key() {
    let fake = Fake::new();
    let board = Board::default();
    let node = spacesheep(&fake, &board, on()).secret("spacesheep.key", record(VAULT_KEY)).start().unwrap();
    assert!(wait(WITHIN, || {
        let health = node.call("health.check", json!({})).unwrap();
        part(&health, "account")["state"] == "STATE_OK"
    }));
    std::thread::sleep(Duration::from_secs(2));
    assert!(fake.calls().iter().all(|c| !c.contains("keys ")), "{:?}", fake.calls());
    assert_eq!(fake.parent_key(), None);
}

#[test]
fn with_no_key_in_the_vault_health_asks_for_spacesheep_add() {
    let fake = Fake::new();
    fake.signed_out();
    let board = Board::default();
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    let mut health = json!(null);
    assert!(
        wait(WITHIN, || {
            health = node.call("health.check", json!({})).unwrap();
            part(&health, "account")["why"].as_str().is_some_and(|w| w.contains("/spacesheep add"))
        }),
        "{health}"
    );
    let account = part(&health, "account");
    assert_eq!(account["state"], "STATE_FAILING", "{health}");
    assert_eq!(account["needs"], json!(["ACTION_AUTH"]), "{health}");
    assert!(fake.calls().iter().all(|c| !c.contains("keys ")), "{:?}", fake.calls());
    let fix = node.call("health.fix", json!({"part": "account", "action": "ACTION_AUTH"})).unwrap();
    assert!(fix["message"].as_str().unwrap().starts_with("Run `/spacesheep add` once"), "{fix}");
}

#[test]
fn a_minted_key_that_isnt_kept_is_saved_at_the_next_try_and_no_other_is_minted() {
    let fake = Fake::new();
    fake.signed_out();
    fake.save_fails(true);
    let board = Board::default();
    // ten times faster than the others: the next try, SIGN_IN_AGAIN, is 3 s here
    let node = spacesheep(&fake, &board, on())
        .env("POINTMAN_SPACESHEEP_FASTER", "600")
        .machine("mac-m4")
        .secret("spacesheep.key", record(VAULT_KEY))
        .start()
        .unwrap();
    let mut health = json!(null);
    assert!(
        wait(WITHIN, || {
            health = node.call("health.check", json!({})).unwrap();
            part(&health, "account")["why"].as_str().is_some_and(|w| w.contains("keeping it failed"))
        }),
        "{health}\n{:?}",
        fake.calls()
    );
    let why = part(&health, "account")["why"].as_str().unwrap().to_string();
    assert!(why.contains("minted it a key, \"mac-m4\" at"), "{why}");
    assert!(why.contains("exited 1: could not check ss_…: fetch failed"), "the CLI's words, with the key cut: {why}");
    assert!(!why.contains("ss_minted") && !why.contains(VAULT_KEY), "{why}");
    let unsaved = fake.unsaved_keys();
    assert!(unsaved.len() == 1 && unsaved[0].starts_with("ss_minted"), "{unsaved:?}");

    fake.save_fails(false);
    assert!(wait(Duration::from_secs(45), || fake.saved_key().is_some()), "{:?}", fake.calls());
    assert_eq!(fake.saved_key().unwrap().trim(), unsaved[0], "the key minted first is the one kept");
    let creates = fake.calls().iter().filter(|c| c.contains("keys create")).count();
    assert_eq!(creates, 1, "no other key minted: {:?}", fake.calls());
    assert!(wait(WITHIN, || {
        part(&node.call("health.check", json!({})).unwrap(), "account")["state"] == "STATE_OK"
    }));
    assert!(!node.log().contains("ss_minted") && !node.log().contains(VAULT_KEY), "{}", node.log());
}

#[test]
fn a_refused_key_is_said_and_not_tried_again_and_again() {
    let fake = Fake::new();
    fake.signed_out();
    fake.refuses();
    let board = Board::default();
    let node = spacesheep(&fake, &board, on()).secret("spacesheep.key", record(VAULT_KEY)).start().unwrap();
    let mut health = json!(null);
    assert!(
        wait(WITHIN, || {
            health = node.call("health.check", json!({})).unwrap();
            part(&health, "account")["why"].as_str().is_some_and(|w| w.contains("the server rejected that key"))
        }),
        "{health}"
    );
    let why = part(&health, "account")["why"].as_str().unwrap().to_string();
    assert!(why.contains("signing it in with the vault's spacesheep.key failed"), "{why}");
    assert!(!why.contains(VAULT_KEY), "{why}");
    // the account is read every second here, and a refused key waits SIGN_IN_AGAIN (30 s here) before its next try
    std::thread::sleep(Duration::from_secs(4));
    let creates = fake.calls().iter().filter(|c| c.contains("keys create")).count();
    assert_eq!(creates, 1, "{:?}", fake.calls());
    assert_eq!(fake.saved_key(), None);
}
