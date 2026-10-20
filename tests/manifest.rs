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
    let skill = m["behavior"]["skill"].as_str().unwrap();
    assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(skill).is_file(), "{skill}");
    let agents = &m["settings"]["properties"]["agents"];
    assert_eq!(agents["type"].as_str(), Some("array"));
    assert_eq!(agents["default"].as_array().map(Vec::len), Some(0), "Talk is off by default: nobody listed");
    assert_eq!(agents["items"]["type"].as_str(), Some("string"));
    let p = &m["permissions"];
    assert_eq!(
        (p["agents"].as_bool(), p["post"].as_bool()),
        (Some(true), Some(true)),
        "agents.sessions and board.post"
    );
    for e in m["events"].as_array().unwrap() {
        assert!(e["name"].as_str().unwrap().starts_with("spacesheep."), "{e}");
    }
}
