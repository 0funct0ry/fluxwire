//! `fluxwire import` end to end: write, validate, report, no-overwrite and exit codes.

use assert_cmd::Command;
use std::path::PathBuf;

fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/import")
        .join(rel)
}

fn fluxwire() -> Command {
    Command::cargo_bin("fluxwire-cli").unwrap()
}

#[test]
fn imports_validates_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let report = dir.path().join("report.md");
    fluxwire()
        .args(["import", "openapi"])
        .arg(fixture("openapi/petstore.yaml"))
        .arg("--into")
        .arg(&out)
        .arg("--report")
        .arg(&report)
        .assert()
        .success();
    fluxwire().arg("validate").arg(&out).assert().success();
    let md = std::fs::read_to_string(&report).unwrap();
    assert!(md.starts_with("# Import report"));
    assert!(md.contains("pets/list-pets.flux.md"));
}

#[test]
fn existing_files_are_kept_without_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let args = |c: &mut Command| {
        c.args(["import", "hurl"])
            .arg(fixture("hurl/health.hurl"))
            .arg("--into")
            .arg(dir.path());
    };
    let mut c = fluxwire();
    args(&mut c);
    c.assert().success();
    let f = dir.path().join("get-health.flux.md");
    std::fs::write(&f, "mine").unwrap();
    let mut c = fluxwire();
    args(&mut c);
    let out = c.assert().success().get_output().stdout.clone();
    assert!(String::from_utf8_lossy(&out).contains("skipped (exists)"));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "mine");
    let mut c = fluxwire();
    args(&mut c);
    c.arg("--overwrite").assert().success();
    assert_ne!(std::fs::read_to_string(&f).unwrap(), "mine");
}

#[test]
fn exit_codes() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, "{ nope").unwrap();
    fluxwire()
        .args(["import", "postman"])
        .arg(&bad)
        .arg("--into")
        .arg(dir.path().join("o"))
        .assert()
        .code(65);
    fluxwire()
        .args(["import", "nonsense"])
        .arg(&bad)
        .arg("--into")
        .arg(dir.path().join("o"))
        .assert()
        .code(64);
    fluxwire()
        .args(["import", "auto"])
        .arg(fixture("http/users.http"))
        .arg("--into")
        .arg(dir.path().join("auto"))
        .assert()
        .success();
}
