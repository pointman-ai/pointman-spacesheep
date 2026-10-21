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
}
