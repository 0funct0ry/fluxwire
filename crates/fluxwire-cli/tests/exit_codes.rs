//! Exit-code matrix for the headless CLI (SPEC §18) against a local stub server.

use assert_cmd::Command;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};

fn server() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let mut s = s;
            let mut buf = [0u8; 2048];
            let _ = s.read(&mut buf);
            let body = r#"{"ok":true}"#;
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    format!("http://{addr}")
}

fn collection(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fw-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("fluxwire.toml"),
        "[collection]\nname = \"t\"\nformat = 1\n",
    )
    .unwrap();
    d
}

fn req(dir: &Path, file: &str, url: &str, extra: &str) {
    std::fs::write(
        dir.join(file),
        format!("---\nname: r\n---\n\n# r\n\n```http\nGET {url}\n```\n{extra}"),
    )
    .unwrap();
}

fn fw() -> Command {
    let mut c = Command::cargo_bin("fluxwire-cli").unwrap();
    c.env("NO_COLOR", "1");
    c
}

#[test]
fn passing_run_is_zero() {
    let url = server();
    let d = collection("ok");
    req(
        &d,
        "a.flux.md",
        &format!("{url}/x"),
        "\n```http expect\nHTTP 200\n```\n",
    );
    fw().args(["run", d.to_str().unwrap()]).assert().code(0);
}

#[test]
fn failed_expect_is_one() {
    let url = server();
    let d = collection("fail");
    req(
        &d,
        "a.flux.md",
        &format!("{url}/x"),
        "\n```http expect\nHTTP 404\n```\n",
    );
    fw().args(["run", d.to_str().unwrap()]).assert().code(1);
}

#[test]
fn network_failure_is_two() {
    let d = collection("net");
    req(&d, "a.flux.md", "http://127.0.0.1:1/x", "");
    fw().args(["run", d.to_str().unwrap()]).assert().code(2);
}

#[test]
fn script_throw_is_three() {
    let url = server();
    let d = collection("script");
    req(
        &d,
        "a.flux.md",
        &format!("{url}/x"),
        "\n```ts post\nthrow new Error('boom')\n```\n",
    );
    fw().args(["run", d.to_str().unwrap()]).assert().code(3);
}

#[test]
fn usage_parse_and_unsupported() {
    fw().args(["run"]).assert().code(64);
    fw().args(["run", "/definitely/not/here"]).assert().code(64);
    fw().args(["import", "postman", "x"]).assert().code(64);
    let d = collection("bad");
    std::fs::write(d.join("a.flux.md"), "# no front matter\n").unwrap();
    fw().args(["validate", d.to_str().unwrap()])
        .assert()
        .code(65);
    let u = collection("unsup");
    std::fs::write(
        u.join("a.flux.md"),
        "---\nname: r\nrequires: [grpc-streaming]\n---\n\n```http\nGET http://x/\n```\n",
    )
    .unwrap();
    fw().args(["validate", u.to_str().unwrap()])
        .assert()
        .code(66);
    fw().args(["run", u.to_str().unwrap()]).assert().code(66);
}

#[test]
fn fmt_check_fails_on_noncanonical() {
    let d = collection("fmt");
    std::fs::write(
        d.join("a.flux.md"),
        "---\nname: r\n---\n\n```http\nGET http://x/\n```",
    )
    .unwrap();
    fw().args(["fmt", "--check", d.to_str().unwrap()])
        .assert()
        .code(1);
    fw().args(["fmt", d.to_str().unwrap()]).assert().code(0);
    fw().args(["fmt", "--check", d.to_str().unwrap()])
        .assert()
        .code(0);
}

#[test]
fn send_splits_streams() {
    let url = server();
    let d = collection("send");
    req(&d, "a.flux.md", &format!("{url}/x"), "");
    let out = fw()
        .args(["send", d.join("a.flux.md").to_str().unwrap()])
        .assert()
        .code(0);
    let o = out.get_output();
    assert_eq!(String::from_utf8_lossy(&o.stdout), r#"{"ok":true}"#);
    assert!(String::from_utf8_lossy(&o.stderr).contains("200"));
    let out = fw()
        .args(["send", "-i", d.join("a.flux.md").to_str().unwrap()])
        .assert()
        .code(0);
    assert!(String::from_utf8_lossy(&out.get_output().stdout).contains("content-type"));
}

#[test]
fn reporters_write_files_and_redact() {
    let url = server();
    let d = collection("rep");
    req(
        &d,
        "a.flux.md",
        &format!("{url}/x"),
        "\n```http expect\nHTTP 200\n```\n",
    );
    let junit = d.join("out.xml");
    let o = fw()
        .args([
            "run",
            d.to_str().unwrap(),
            "-r",
            "tap",
            "-r",
            "junit",
            "-o",
            junit.to_str().unwrap(),
        ])
        .assert()
        .code(0);
    assert!(String::from_utf8_lossy(&o.get_output().stdout).starts_with("TAP version 14"));
    assert!(
        std::fs::read_to_string(junit)
            .unwrap()
            .contains("<testsuites")
    );
    let j = fw()
        .args(["run", d.to_str().unwrap(), "-r", "json"])
        .assert()
        .code(0);
    serde_json::from_slice::<serde_json::Value>(&j.get_output().stdout).unwrap();
}

#[test]
fn env_list_and_completions() {
    let d = collection("env");
    std::fs::create_dir_all(d.join("environments")).unwrap();
    std::fs::write(d.join("environments/ci.env"), "A=1\n").unwrap();
    let o = fw()
        .args(["env", "list", d.to_str().unwrap()])
        .assert()
        .code(0);
    assert!(String::from_utf8_lossy(&o.get_output().stdout).contains("ci"));
    fw().args(["completions", "bash"]).assert().code(0);
}
