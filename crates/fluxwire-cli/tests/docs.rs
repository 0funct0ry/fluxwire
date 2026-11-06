//! `fluxwire docs export` (SPEC §16).

use assert_cmd::Command;

fn fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/payments-api")
}

#[test]
fn exports_the_payments_fixture_without_javascript() {
    let out = tempfile::tempdir().unwrap();
    Command::cargo_bin("fluxwire-cli")
        .unwrap()
        .args(["docs", "export"])
        .arg(fixture())
        .arg("--out")
        .arg(out.path())
        .assert()
        .success();
    let html = std::fs::read_to_string(out.path().join("index.html")).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert!(html.contains("Create payment"));
    assert!(html.contains("prefers-color-scheme:dark"));
    assert!(!html.contains("<script"));
}

#[test]
fn missing_path_is_a_usage_error() {
    let out = tempfile::tempdir().unwrap();
    Command::cargo_bin("fluxwire-cli")
        .unwrap()
        .args(["docs", "export", "/no/such/path", "--out"])
        .arg(out.path())
        .assert()
        .code(64);
}
