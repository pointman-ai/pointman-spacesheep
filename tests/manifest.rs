//! extension.toml within what core's manifest check takes (Pointman's src/messageboard/extensions/
//! manifest.py): core refuses a release whose manifest breaks one.

#[test]
fn the_manifest_keeps_to_cores_limits() {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/extension.toml")).unwrap();
    let m: toml::Table = toml::from_str(&text).unwrap();
    let chars = |v: &toml::Value| v.as_str().unwrap().chars().count();
    let ext = &m["extension"];
    assert_eq!(ext["version"].as_str(), Some(env!("CARGO_PKG_VERSION")), "extension.toml's version is Cargo's");
    assert!((1..=60).contains(&chars(&ext["name"])) && (1..=200).contains(&chars(&ext["summary"])));
    assert_eq!(
        m["run"]["command"].as_array().unwrap()[0].as_str(),
        Some("bin/spacesheep"),
        "a binary runs as bin/<id>"
    );
    assert!(!ext["platforms"].as_array().unwrap().is_empty(), "a process says where it runs");
    let files: Vec<&str> = m["behavior"]["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, ["references"], "the guide, checker and template go to every run's folder");
    for f in ["references/pages.md", "references/check.py", "references/template.html"] {
        assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(f).is_file(), "{f} ships with the behaviour");
    }
    let skill = m["behavior"]["skill"].as_str().unwrap();
    assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(skill).is_file(), "{skill}");
    assert!(m.get("settings").is_none(), "no settings: the account's Talk switch is the one switch");
    let p = &m["permissions"];
    assert_eq!(
        (p["agents"].as_bool(), p["post"].as_bool()),
        (Some(true), Some(true)),
        "agents.sessions and board.post"
    );
    for e in m["events"].as_array().unwrap() {
        assert!(e["name"].as_str().unwrap().starts_with("spacesheep."), "{e}");
    }
    // `/spacesheep add`: the one key that signs each node in, which only the extension may open (EXT-132)
    let credentials = m["credentials"].as_array().unwrap();
    assert_eq!(credentials.len(), 1);
    let key = &credentials[0];
    assert_eq!(key["item"].as_str(), Some("spacesheep.key"));
    let secrets: Vec<&str> = p["secrets"].as_array().unwrap().iter().map(|s| s.as_str().unwrap()).collect();
    assert_eq!(secrets, ["spacesheep.key"], "only the key it signs in with");
    assert_eq!(key["needed"].as_integer(), Some(0), "a machine signed in by hand needs no key in the vault");
    let value = fancy_regex::Regex::new(&format!("^(?:{})$", key["value"].as_str().unwrap())).unwrap();
    assert!(value.is_match("ss_0123456789abcdef0123456789abcdef").unwrap(), "a key as spacesheep.dev makes one");
    assert!(!value.is_match("ssc_0123456789abcdef").unwrap(), "a one-time sign-in code isn't a key");
    // the slash command that opens that form: without it there's no `/spacesheep add` to type
    let commands = m["commands"].as_array().unwrap();
    let add = commands.iter().find(|c| c["kind"].as_str() == Some("spacesheep.add")).expect("a spacesheep.add command");
    assert_eq!(add["slash"].as_str(), Some("add"));
    assert_eq!(add["form"].as_str(), key["id"].as_str(), "it opens the key's form");
    assert!(add["params"]["properties"].as_table().unwrap().is_empty(), "no name: only spacesheep.key is opened");
    assert_eq!(add["params"]["additionalProperties"].as_bool(), Some(false), "nor a name passed through");
}
