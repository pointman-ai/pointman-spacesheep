//! Spacesheep's real CLI, as `npx -y spacesheep@latest`, on a machine that isn't signed in (an empty HOME), so it
//! never reaches spacesheep.dev: health says the machine needs signing in. Runs when SPACESHEEP_REAL_CLI is the
//! folder with Node.js's npx (CI sets it); otherwise it says so and passes.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;

#[test]
fn the_real_cli_on_a_machine_not_signed_in_says_so_in_health() {
    let Some(npx) = std::env::var_os("SPACESHEEP_REAL_CLI") else {
        eprintln!("skipped: set SPACESHEEP_REAL_CLI to the folder with npx to run Spacesheep's real CLI");
        return;
    };
    let home = tempfile::tempdir().unwrap();
    let board = Board::with(vec![agent("scene", "s-1", 41)]);
    let path_var = format!("{}:/usr/bin:/bin", npx.to_string_lossy());
    let node = spacesheep_on(&path_var, &board, on()).env("HOME", home.path()).start().unwrap();

    // npx fetches the CLI into the empty HOME first
    let signed_out = wait(Duration::from_secs(180), || {
        let health = node.call("health.check", json!({})).unwrap();
        part(&health, "account")["needs"] == json!(["ACTION_AUTH"])
    });
    let health = node.call("health.check", json!({})).unwrap();
    assert!(signed_out, "{health}\n{}", node.log());
    let account = part(&health, "account");
    assert_eq!(account["state"], "STATE_FAILING", "{health}");
    assert!(
        account["why"].as_str().unwrap().starts_with("this machine isn't signed in to Spacesheep: not signed in"),
        "{health}"
    );
    // nothing is listened for on a machine that can't sign in
    let listeners = part(&health, "listeners");
    assert_eq!(listeners["state"], "STATE_STOPPED", "{health}");
    assert_eq!(listeners["why"], "not listening: this machine isn't signed in to Spacesheep", "{health}");
    assert!(node.log().contains("talk status: not signed in"), "{}", node.log());
    assert!(!node.log().contains("listening for @scene"), "{}", node.log());
    let fix = node.call("health.fix", json!({"part": "account", "action": "ACTION_AUTH"})).unwrap();
    assert!(fix["message"].as_str().unwrap().contains("npx -y spacesheep@latest login"), "{fix}");
    assert!(board.posts().is_empty());
}
