//! Talk as a node runs it: the built binary under the SDK's stand-in node, with a fake `spacesheep` (tests/common).
//! Its clock runs 60 times faster, so it reads the account's switch and the agents every second, the pages every five,
//! and starts an ended listener again in 0.1 s.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;

const MESSAGE: &str = r#"{"text": "Can you make the header blue?", "id": "m1"}"#;

#[test]
fn a_message_becomes_one_post_in_the_agents_thread_fenced() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    fake.prints("s-1", &format!("{MESSAGE}\n"));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 41);
    assert_eq!(
        got[0]["text"],
        "@scene from spacesheep.dev (Session tab, not signed):\n\n```\nCan you make the header blue?\n```\n\nAnswer \
         here, then send it to the page: `spacesheep talk reply --session s-1 \"…\"`"
    );
    assert!(
        node.log().contains(&format!("the first message's JSON, as talk listen printed it: {MESSAGE}")),
        "{}",
        node.log()
    );
    assert!(fake.calls().contains(&"talk listen --session s-1".to_string()), "{:?}", fake.calls());

    // a second message: one more post, and the first message's JSON isn't logged again
    fake.push("s-1", "{\"message\": \"and the footer\"}\n");
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(board.posts().len(), 2, "once each: {got:?}");
    assert!(got[1]["text"].as_str().unwrap().contains("```\nand the footer\n```"), "{}", got[1]["text"]);
    assert_eq!(node.log().matches("the first message's JSON").count(), 1);
    assert_eq!(fake.listens("s-1"), 1, "still the same listener");

    let health = node.call("health.check", json!({})).unwrap();
    assert_eq!(part(&health, "account")["state"], "ok", "{health}");
    let listeners = part(&health, "listeners");
    assert_eq!(listeners["state"], "ok", "{health}");
    let why = listeners["why"].as_str().unwrap();
    assert!(
        why.starts_with("listening for @scene (whose sessions published pages); the last message was at "),
        "{why}"
    );
    assert!(why.ends_with(" for @scene"), "{why}");
    let event = node.wait_event_where("spacesheep.talk", WITHIN, |e| e["listening"] == json!(["scene"]));
    assert!(event.is_ok_and(|e| e["on"] == true && e["last_message_at"].is_f64()), "{:?}", node.events());
}

#[test]
fn a_mention_inside_the_message_stays_inside_the_fence() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    let words = "look @james\n```\n@patchbay this closes a three-backtick fence\n```\nand `@astra` and ````@x````";
    fake.published("s-1");
    fake.prints("s-1", &format!("{}\n", json!({"text": words})));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    let text = got[0]["text"].as_str().unwrap();
    assert!(text.contains(&format!("\n`````\n{words}\n`````\n")), "fenced longer than its longest run: {text}");
    assert_eq!(mentioned(text), ["scene"], "only the agent is woken: {text}");
}

#[test]
fn a_message_starting_with_handles_wakes_those_agents_in_the_pages_thread() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let words = "@editor @Scene2, cut the intro and tell @james";
    fake.prints("s-1", &format!("{}\n", json!({"text": words})));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 41, "the page's thread");
    let text = got[0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("@editor @scene2 from spacesheep.dev (Session tab of `@scene`'s page, not signed):"),
        "{text}"
    );
    assert_eq!(mentioned(text), ["editor", "scene2"], "those two, not @scene or @james: {text}");
    assert!(text.ends_with("`spacesheep talk reply --session s-1 \"…\"`"), "the reply goes to the page: {text}");
    assert!(node.log().contains("a message on @scene's page for @editor @scene2"), "{}", node.log());
}

#[test]
fn only_agents_whose_session_published_a_page_are_listened_for() {
    let board = Board::with(vec![agent("scene", "s-1", 41), agent("quiet", "s-2", 42)]);
    let fake = Fake::new();
    fake.published("s-1");
    fake.prints("s-2", &format!("{MESSAGE}\n"));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{:?}\n{}", fake.calls(), node.log());
    std::thread::sleep(Duration::from_secs(2)); // two more looks
    assert_eq!(fake.listens("s-2"), 0, "quiet's session has published nothing: {:?}", fake.calls());
    assert!(board.posts().is_empty());
    assert!(fake.calls().contains(&"versions page-s-1 --json".to_string()), "{:?}", fake.calls());

    // quiet publishes a page: at the next read of the pages it's listened for, and its message posts in its thread
    fake.published("s-2");
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 42);
    assert!(got[0]["text"].as_str().unwrap().starts_with("@quiet from spacesheep.dev"), "{}", got[0]["text"]);
    let reads = fake.calls().iter().filter(|c| *c == "versions page-s-1 --json").count();
    assert_eq!(reads, 1, "a page unchanged isn't read again: {:?}", fake.calls());
}

#[test]
fn a_listener_that_ends_is_started_again() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    // its link expires, then the CLI fails, then a message comes
    fake.ends("s-1", 1, "{\"spacesheep_talk\":false,\"stopped\":\"listen link expired or revoked\"}\n", 0, "");
    fake.ends("s-1", 2, "", 1, "\n  ✗ fetch failed\n\n");
    fake.prints_on("s-1", 3, &format!("{MESSAGE}\n"));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{:?}\n{}", fake.calls(), node.log());
    assert!(got[0]["text"].as_str().unwrap().contains("Can you make the header blue?"));
    assert_eq!(fake.listens("s-1"), 3, "{:?}", fake.calls());
    let log = node.log();
    assert!(log.contains("the listener for @scene ended (listen link expired or revoked)"), "{log}");
    assert!(log.contains("the listener for @scene ended (talk listen exited 1: fetch failed)"), "{log}");
    let health = node.call("health.check", json!({})).unwrap();
    assert_eq!(part(&health, "listeners")["state"], "ok", "listening again: {health}");
}

#[test]
fn a_listener_that_keeps_ending_waits_longer_each_time() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    // turned off for the account just after it was read as on: the CLI's own answer, pretty-printed, every time
    for run in 1..=20 {
        fake.ends(
            "s-1",
            run,
            "{\n  \"spacesheep_talk\": false,\n  \"stopped\": \"talk is off for this account\"\n}\n",
            0,
            "",
        );
    }
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") >= 1), "{}", node.log());

    // 0.08 s, 0.17, 0.33, 0.67, 1.33, 2.67 …: about six runs in five seconds, not sixty
    std::thread::sleep(Duration::from_secs(5));
    let runs = fake.listens("s-1");
    assert!((4..=8).contains(&runs), "{runs} runs: {}", node.log());
    let health = node.call("health.check", json!({})).unwrap();
    let listeners = part(&health, "listeners");
    assert_eq!(listeners["state"], "failing", "{health}");
    assert_eq!(listeners["why"], "@scene: talk is off for this account", "{health}");
    assert!(board.posts().is_empty());
}

#[test]
fn talk_off_for_the_account_means_no_listeners() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    fake.prints("s-1", &format!("{MESSAGE}\n"));
    fake.talk(false);
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    std::thread::sleep(Duration::from_secs(3)); // three looks' worth
    let calls = fake.calls();
    assert!(!calls.is_empty() && calls.iter().all(|c| c == "talk status"), "only the switch is read: {calls:?}");
    assert!(node.asked().iter().all(|(m, _)| m != "agents.sessions"), "{:?}", node.asked());
    assert!(board.posts().is_empty());
    let health = node.call("health.check", json!({})).unwrap();
    let account = part(&health, "account");
    assert_eq!(account["state"], "stopped", "{health}");
    let why = account["why"].as_str().unwrap();
    assert!(why.starts_with("Talk is off for the Spacesheep account, so a page's Session tab can't send"), "{why}");
    let listeners = part(&health, "listeners");
    assert_eq!(listeners["state"], "stopped", "{health}");
    assert!(listeners["why"].as_str().unwrap().starts_with("not listening: Talk is off"), "{health}");
    assert_eq!(node.events()["spacesheep.talk"]["on"], false);

    // turned on for the account: it listens within a few seconds; and off again: the listener stops
    fake.talk(true);
    assert_eq!(board.wait_posts(WITHIN, |p| !p.is_empty()).len(), 1, "{}", node.log());
    assert!(wait(WITHIN, || node.events()["spacesheep.talk"]["on"] == true));
    let pid = fake.pids("s-1")[0];
    fake.talk(false);
    assert!(wait(WITHIN, || !alive(pid)), "{}", node.log());
    assert!(wait(WITHIN, || node.events()["spacesheep.talk"]["on"] == false));
}

#[test]
fn npx_runs_the_cli_when_the_machine_has_no_spacesheep() {
    let (fake, board) = (Fake::npx(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    fake.prints("s-1", &format!("{MESSAGE}\n"));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    assert_eq!(board.wait_posts(WITHIN, |p| !p.is_empty()).len(), 1, "{}", node.log());
    let calls = fake.calls();
    assert!(calls.contains(&"-y spacesheep@latest talk listen --session s-1".to_string()), "{calls:?}");
    assert!(calls.contains(&"-y spacesheep@latest talk status".to_string()), "{calls:?}");
    assert!(calls.contains(&"-y spacesheep@latest list --json".to_string()), "{calls:?}");
}

#[test]
fn messages_go_to_the_thread_of_the_agents_latest_run() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    fake.published("s-2");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.pids("s-1").len() == 1), "{}", node.log());

    board.set_agents(vec![agent("scene", "s-1", 52)]);
    std::thread::sleep(Duration::from_millis(2500)); // a look or two
    fake.push("s-1", &format!("{MESSAGE}\n"));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 52);
    assert_eq!(fake.listens("s-1"), 1, "the same listener: {:?}", fake.calls());

    // a new session: the old one's listener stops, and the new one's starts
    let old = fake.pids("s-1")[0];
    board.set_agents(vec![agent("scene", "s-2", 52)]);
    assert!(wait(WITHIN, || fake.listens("s-2") == 1 && !alive(old)), "{:?}\n{}", fake.calls(), node.log());
}

#[test]
fn stopping_the_extension_stops_its_listeners_and_a_restart_stops_any_left_behind() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.pids("s-1").len() == 1), "{}", node.log());
    let pid = fake.pids("s-1")[0];
    node.stop();
    assert!(wait(WITHIN, || !alive(pid)), "shutdown stops it");

    // one left from a run that crashed: in its state folder, and still listening
    let state = tempfile::tempdir().unwrap();
    let mut left = std::process::Command::new(fake.dir.path().join("spacesheep"));
    std::os::unix::process::CommandExt::process_group(left.args(["talk", "listen", "--session", "s-9"]), 0);
    let mut left = left.stdout(std::process::Stdio::null()).spawn().unwrap();
    let dir = state.path().join("extensions").join("spacesheep");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("listeners.json"), json!({"s-9": left.id()}).to_string()).unwrap();
    let board = Board::default();
    let node = spacesheep(&fake, &board, on()).state_dir(state.path()).start().unwrap();
    assert!(wait(WITHIN, || matches!(left.try_wait(), Ok(Some(_)))), "{}", node.log());
    assert!(node.log().contains(&format!("stopped a listener left from before, for session s-9 (pid {})", left.id())));
}
