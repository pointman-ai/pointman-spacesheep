//! Talk as a node runs it: the built binary under the SDK's stand-in node, with a fake `spacesheep` (tests/common).
//! Its clock runs 60 times faster, so it reads the account's switch and the agents every second, the pages every five,
//! and starts an ended listener again in 0.1 s.

mod common;

use std::time::Duration;

use common::*;
use serde_json::json;

const MESSAGE: &str = r#"{"text": "Can you make the header blue?", "id": "m1"}"#;

fn comment(id: i64, key: &str, words: &str) -> String {
    let page = "https://spacesheep.dev/space/page";
    format!(
        "{}\n",
        json!({"id": id, "space_url": page, "asked_in": "a comment on the page",
        "message": format!("{words}\n\n---\nThe owner of the page “Title” ({page}?comment={key}) wrote this as a comment on it, to you. It is about the page as a whole.")})
    )
}

#[test]
fn followups_share_the_first_posts_side_thread_and_session_tab_messages_stay_separate() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let batch = format!(
        "{}{}{}{}{}",
        comment(1, "a", "first"),
        comment(2, "a", "followup"),
        comment(3, "b", "another comment"),
        "{\"message\":\"session one\"}\n",
        "{\"message\":\"session two\"}\n"
    );
    fake.prints("s-1", &batch);
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    let got = board.wait_posts(WITHIN, |p| p.len() >= 5);
    assert_eq!(got.len(), 5, "{}", node.log());
    assert!(got[0]["side_of"].is_null(), "{got:?}");
    assert_eq!(got[1]["side_of"]["id"], "post_1001");
    assert!(got[2..].iter().all(|post| post["side_of"].is_null()), "{got:?}");
}

#[test]
fn a_restart_reuses_the_comment_root_but_a_new_pointman_thread_gets_its_own() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    let state = tempfile::tempdir().unwrap();
    fake.published("s-1");
    let node = spacesheep(&fake, &board, on()).state_dir(state.path()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{}", node.log());
    fake.push("s-1", &comment(1, "a", "first"));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    let path = state.path().join("extensions/spacesheep/comment-threads.json");
    assert!(wait(WITHIN, || path.exists()), "root saved before restart");
    node.stop();

    let node = spacesheep(&fake, &board, on()).state_dir(state.path()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 2), "{}", node.log());
    fake.push("s-1", &comment(2, "a", "after restart"));
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    assert_eq!(got.len(), 2, "{}", node.log());
    assert_eq!(got[1]["side_of"]["id"], "post_1001");
    board.set_agents(vec![agent("scene", "s-1", 42)]);
    std::thread::sleep(Duration::from_millis(2500)); // the next agents.sessions look updates the listener
    fake.push("s-1", &comment(3, "a", "in a different Pointman thread"));
    let got = board.wait_posts(WITHIN, |p| p.len() >= 3);
    assert_eq!(got.len(), 3, "{}", node.log());
    assert_eq!(got[2]["thread_id"], 42);
    assert!(got[2]["side_of"].is_null());
}

#[test]
fn a_failed_delivery_does_not_establish_a_comment_root() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    *board.failures.lock().unwrap() = 3;
    fake.prints(
        "s-1",
        &format!("{}{}{}", comment(1, "a", "fails"), comment(2, "a", "first delivered"), comment(3, "a", "followup")),
    );
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    assert_eq!(got.len(), 2, "{}", node.log());
    assert!(got[0]["side_of"].is_null());
    assert_eq!(got[1]["side_of"]["id"], "post_1001");
}

#[test]
fn a_deleted_root_is_replaced_and_the_next_reply_uses_the_replacement() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{}", node.log());
    fake.push("s-1", &comment(1, "a", "first"));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    board.deleted_roots.lock().unwrap().push("post_1001".into());
    fake.push("s-1", &comment(2, "a", "after the root was deleted"));
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    assert_eq!(got.len(), 2, "{}", node.log());
    assert!(got[1]["side_of"].is_null(), "the replacement is in the main stream: {got:?}");
    fake.push("s-1", &comment(3, "a", "after the replacement"));
    let got = board.wait_posts(WITHIN, |p| p.len() >= 3);
    assert_eq!(got.len(), 3, "{}", node.log());
    assert_eq!(got[2]["side_of"]["id"], "post_1002");
}

#[test]
fn a_temporary_posting_failure_keeps_the_existing_root() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{}", node.log());
    fake.push("s-1", &comment(1, "a", "first"));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    *board.failures.lock().unwrap() = 1;
    fake.push("s-1", &comment(2, "a", "after a temporary failure"));
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    assert_eq!(got.len(), 2, "{}", node.log());
    assert_eq!(got[1]["side_of"]["id"], "post_1001");
}

#[test]
fn comments_without_pin_context_still_share_their_side_thread() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let message = json!({"message": "a comment without a pin", "asked_in": "a comment on the page",
        "space_url": "https://spacesheep.dev/space/page?comment=a"});
    fake.prints("s-1", &format!("{message}\n{message}\n"));
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    assert_eq!(got.len(), 2, "{}", node.log());
    assert!(got[0]["side_of"].is_null());
    assert_eq!(got[1]["side_of"]["id"], "post_1001");
}

#[test]
fn a_comment_keeps_its_words_and_moves_pin_and_reply_into_context() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.answers_named();
    fake.published("s-1");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{}", node.log());
    let words = "make this clearer\n\n---\nkeep my horizontal rule and @james here";
    let url = "https://spacesheep.dev/space/page?comment=comment-1";
    let message = format!("{words}\n\n---\nThe owner of the page “Title” ({url}) wrote this as a comment on it, to you. It is pinned to a block reading: @other ```quoted```.\nEarlier in that comment thread (quoted for context):\n- someone: old words\nYour answer is posted as a reply; use your own session.");
    fake.push("s-1", &format!("{}\n", json!({"message": message, "space_url": "https://spacesheep.dev/space/page", "asked_in": "a comment on the page", "id": 19})));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    let text = got[0]["text"].as_str().unwrap();
    assert!(text.contains(words), "{text}");
    assert!(!text.contains("The owner of the page") && !text.contains("talk reply"), "{text}");
    let context = got[0]["blocks"][0]["text"]["markdown"].as_str().unwrap();
    assert!(
        context.contains("It is pinned to a block reading: @other ```quoted```.")
            && context.contains("--session s-1 --to 19"),
        "{context}"
    );
    assert!(!context.contains("old words") && !context.contains("use your own session"), "{context}");
    assert_eq!(mentioned(&format!("{text}\n{context}")), ["scene"]);
    assert_eq!(got[0]["links"], json!([url]));
}

#[test]
fn session_tab_words_that_look_like_a_comment_footer_are_kept() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let message = "example\n\n---\nThe owner of the page “example” wrote this as a comment on it, to you. Keep this.";
    fake.prints("s-1", &format!("{}\n", json!({"message": message})));
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert!(got[0]["text"].as_str().unwrap().contains(message));
}

#[test]
fn a_message_becomes_one_post_in_the_agents_thread_fenced() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    fake.prints("s-1", &format!("{MESSAGE}\n"));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 41);
    assert_eq!(got[0]["text"], "@scene\n\n```\nCan you make the header blue?\n```");
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
    assert_eq!(part(&health, "account")["state"], "STATE_OK", "{health}");
    let listeners = part(&health, "listeners");
    assert_eq!(listeners["state"], "STATE_OK", "{health}");
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
    assert!(text.contains(&format!("\n`````\n{words}\n`````")), "fenced longer than its longest run: {text}");
    assert_eq!(mentioned(text), ["scene"], "only the agent is woken: {text}");
}

#[test]
fn a_message_starting_with_handles_wakes_those_agents_in_the_pages_thread() {
    let (fake, board) = (Fake::new(), Board::with(vec![agent("scene", "s-1", 41)]));
    fake.published("s-1");
    let words = "@pm.editor @Scene2, cut the intro and tell @james";
    fake.prints("s-1", &format!("{}\n", json!({"text": words})));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 41, "the page's thread");
    let text = got[0]["text"].as_str().unwrap();
    assert!(text.starts_with("@editor @scene2\n\n"), "{text}");
    assert_eq!(mentioned(text), ["editor", "scene2"], "those two, not @scene or @james: {text}");
    assert!(got[0]["blocks"][0]["text"]["markdown"]
        .as_str()
        .unwrap()
        .ends_with("`spacesheep talk reply --session s-1 \"…\"`"));
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
    assert!(got[0]["text"].as_str().unwrap().starts_with("@quiet\n\n"), "{}", got[0]["text"]);
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
    assert_eq!(part(&health, "listeners")["state"], "STATE_OK", "listening again: {health}");
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
    assert_eq!(listeners["state"], "STATE_FAILING", "{health}");
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
    assert_eq!(account["state"], "STATE_STOPPED", "{health}");
    let why = account["why"].as_str().unwrap();
    assert!(why.starts_with("Talk is off for the Spacesheep account, so a page's Session tab can't send"), "{why}");
    let listeners = part(&health, "listeners");
    assert_eq!(listeners["state"], "STATE_STOPPED", "{health}");
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

    // a new session: its listener starts, and the old session's page is still listened on, as a finished agent's
    let old = fake.pids("s-1")[0];
    board.set_agents(vec![agent("scene", "s-2", 52)]);
    assert!(wait(WITHIN, || fake.listens("s-2") == 1), "{:?}\n{}", fake.calls(), node.log());
    std::thread::sleep(Duration::from_secs(2));
    assert!(alive(old) && fake.listens("s-1") == 1, "the same listener goes on: {:?}", fake.calls());
}

#[test]
fn a_finished_agents_page_reaches_only_the_agents_it_calls() {
    let board = Board::with(vec![agent("docrules", "s-d", 41), agent("editor", "s-e", 50)]);
    let fake = Fake::new();
    fake.published("s-d");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.pids("s-d").len() == 1), "{}", node.log());

    // docrules is despawned: the node stops listing it, and its page is still listened on, as remembered
    board.set_agents(vec![agent("editor", "s-e", 50)]);
    assert!(wait(WITHIN, || node.log().contains("listening on @docrules's page (finished)")), "{}", node.log());
    let message = json!({"message": "@pm.editor fix the table", "asked_in": "a comment on the page"});
    fake.push("s-d", &format!("{message}\n"));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 41, "the page's own thread, remembered");
    let text = got[0]["text"].as_str().unwrap();
    let header = "@editor\n\n";
    assert!(text.starts_with(header), "{text}");
    assert_eq!(mentioned(text), ["editor"], "{text}");

    // calling nobody, or only docrules: no post, and a reply on the page saying how to ask
    fake.push("s-d", &format!("{}\n", json!({"message": "@pm.docrules hello"})));
    assert!(
        wait(WITHIN, || fake.calls().iter().any(|c| c.starts_with("talk reply --session s-d docrules has finished"))),
        "{:?}\n{}",
        fake.calls(),
        node.log()
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(board.posts().len(), 1, "nothing posted for a finished agent");
}

#[test]
fn a_restart_keeps_the_finished_agents_it_remembers() {
    // agents.json from a run before: docrules once had s-d here, and is gone now
    let state = tempfile::tempdir().unwrap();
    let dir = state.path().join("extensions").join("spacesheep");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("agents.json"), json!({"s-d": {"handle": "docrules", "thread": 41}}).to_string()).unwrap();
    let (fake, board) = (Fake::new(), Board::with(vec![agent("editor", "s-e", 50)]));
    fake.published("s-d");
    let node = spacesheep(&fake, &board, on()).state_dir(state.path()).start().unwrap();

    // the first looks come before the pages are read: they mustn't forget s-d
    assert!(wait(WITHIN, || node.log().contains("listening on @docrules's page (finished)")), "{}", node.log());
    let kept = std::fs::read_to_string(dir.join("agents.json")).unwrap();
    assert!(kept.contains("s-d"), "{kept}");
}

#[test]
fn the_reply_names_the_message_it_answers_once_the_cli_can() {
    // EXT-73: Spacesheep answers a session's newest message, so with comments waiting on two pages a reply that names
    // none lands under the other one. A CLI with `talk reply --to` says which.
    for named in [false, true] {
        let board = Board::with(vec![agent("scene", "s-1", 41)]);
        let fake = Fake::new();
        if named {
            fake.answers_named();
        }
        fake.published("s-1");
        let node = spacesheep(&fake, &board, on()).start().unwrap();
        assert!(wait(WITHIN, || fake.calls().iter().any(|c| c == "--help")), "{}", node.log());
        assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{}", node.log());
        fake.push("s-1", &format!("{}\n", json!({"message": "make it blue", "id": 19})));
        let got = board.wait_posts(WITHIN, |p| !p.is_empty());
        let text = got[0]["blocks"][0]["text"]["markdown"].as_str().unwrap();
        let line = if named { "talk reply --session s-1 --to 19 \"…\"" } else { "talk reply --session s-1 \"…\"" };
        assert!(text.ends_with(&format!("`spacesheep {line}`")), "named {named}: {text}");
        if named {
            // and the extension's own answer on a finished agent's page names its message too
            board.set_agents(vec![]);
            assert!(wait(WITHIN, || node.log().contains("listening on @scene's page (finished)")), "{}", node.log());
            fake.push("s-1", &format!("{}\n", json!({"message": "hello", "id": 20})));
            let reply = "talk reply --session s-1 --to 20 scene has finished";
            assert!(wait(WITHIN, || fake.calls().iter().any(|c| c.starts_with(reply))), "{:?}", fake.calls());
        }
    }
}

#[test]
fn a_message_sent_again_says_so() {
    // EXT-80: a new listener gets back the messages no `talk reply` answered, which may have been answered another way
    let board = Board::with(vec![agent("scene", "s-1", 41)]);
    let fake = Fake::new();
    fake.published("s-1");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 1), "{}", node.log());
    let first = json!({"message": "first time", "id": 7});
    let again = json!({"message": "again", "id": 6, "redelivered": true});
    fake.push("s-1", &format!("{first}\n{again}\n"));
    let got = board.wait_posts(WITHIN, |p| p.len() >= 2);
    let texts: Vec<&str> = got.iter().map(|p| p["blocks"][0]["text"]["markdown"].as_str().unwrap()).collect();
    assert_eq!(texts.len(), 2, "{}", node.log());
    assert!(!texts[0].contains("Sent again"), "{}", texts[0]);
    assert!(texts[1].contains("Sent again: no `talk reply` has answered it yet."), "{}", texts[1]);
    // this CLI's reply can't name the message, so it would answer the newest one waiting
    assert!(texts[1].contains("and it's the only message waiting,"), "{}", texts[1]);
}

#[test]
fn a_page_pushed_out_of_the_latest_list_still_reaches_its_agent() {
    // `list` is the account's latest 50: a burst of other pages (core's docs job, 48 in a minute) pushes one out
    let board = Board::with(vec![agent("scene", "s-1", 41), agent("busy", "s-2", 42)]);
    let fake = Fake::new();
    fake.published("s-1");
    fake.published("s-2");
    let node = spacesheep(&fake, &board, on()).start().unwrap();
    assert!(wait(WITHIN, || fake.listens("s-1") == 1 && fake.listens("s-2") == 1), "{}", node.log());

    fake.listed_only(&["s-2"]);
    let lists = || fake.calls().iter().filter(|c| *c == "list --json").count();
    let before = lists();
    assert!(wait(WITHIN, || lists() >= before + 2), "two more reads of the pages: {}", node.log());
    assert!(!node.log().contains("not listening for @scene"), "{}", node.log());

    fake.push("s-1", &format!("{MESSAGE}\n"));
    let got = board.wait_posts(WITHIN, |p| !p.is_empty());
    assert_eq!(got.len(), 1, "{}", node.log());
    assert_eq!(got[0]["thread_id"], 41);
    assert_eq!(fake.listens("s-1"), 1, "the same listener all along: {:?}", fake.calls());
}

#[test]
fn pages_no_agent_here_ever_had_are_left_to_their_own_node() {
    // every node reads the account's pages: s-nixos is an agent alive on another node, s-james James's own session
    let (fake, board) = (Fake::new(), Board::with(vec![agent("editor", "s-e", 50)]));
    fake.published("s-nixos");
    fake.published("s-james");
    fake.prints("s-nixos", &format!("{}\n", json!({"text": "@pm.editor look at this"})));
    let node = spacesheep(&fake, &board, on()).start().unwrap();

    assert!(wait(WITHIN, || fake.calls().iter().any(|c| c.starts_with("versions page-s-james"))), "{}", node.log());
    std::thread::sleep(Duration::from_secs(3)); // looks after the pages were read
    assert_eq!((fake.listens("s-nixos"), fake.listens("s-james")), (0, 0), "{:?}", fake.calls());
    assert!(board.posts().is_empty());
    assert!(!fake.calls().iter().any(|c| c.starts_with("talk reply")), "no 'has finished' on pages that aren't ours");
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
