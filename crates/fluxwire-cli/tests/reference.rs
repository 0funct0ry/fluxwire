//! `fluxwire reference` feeds the docs site generators (M15), so the pages cannot drift.

use assert_cmd::Command;

fn out(args: &[&str]) -> String {
    let o = Command::cargo_bin("fluxwire-cli")
        .unwrap()
        .arg("reference")
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success());
    String::from_utf8(o.stdout).unwrap()
}

#[test]
fn cli_reference_lists_every_command() {
    let md = out(&["cli"]);
    for c in [
        "run",
        "send",
        "fmt",
        "validate",
        "env list",
        "import",
        "openapi pull",
        "docs export",
        "mcp serve",
    ] {
        assert!(md.contains(&format!("## `fluxwire {c}`")), "missing {c}");
    }
    assert!(!md.contains("`fluxwire reference"), "hidden command leaked");
}

#[test]
fn protocol_reference_is_json_with_roles_and_reserved() {
    let v: serde_json::Value = serde_json::from_str(&out(&["protocols"])).unwrap();
    assert!(
        v["protocols"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["id"] == "http")
    );
    assert!(
        v["reserved_roles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "query")
    );
    assert!(
        v["reserved_request_languages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "grpc")
    );
    assert!(
        v["script_roles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "pre")
    );
}

#[test]
fn exit_codes_reference_has_all_codes() {
    let v: serde_json::Value = serde_json::from_str(&out(&["exit-codes"])).unwrap();
    let codes: Vec<u64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["code"].as_u64().unwrap())
        .collect();
    assert_eq!(codes, vec![0, 1, 2, 3, 4, 64, 65, 66]);
}
