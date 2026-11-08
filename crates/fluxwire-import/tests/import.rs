//! Fixture corpus tests: every importer's output parses cleanly, is canonical and matches its
//! snapshot, and every lossy conversion carries a reason.

use fluxwire_format::{is_canonical, parse_with};
use fluxwire_import::{ImportPlan, Options, PlannedKind, Source, apply, detect, plan};
use fluxwire_model::{FileKind, Severity};
use fluxwire_protocol::ProtocolRegistry;
use std::path::{Path, PathBuf};

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/import")
        .join(rel)
}

fn run(source: Source, rel: &str, opts: &Options) -> ImportPlan {
    plan(source, &fixture(rel), opts).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn kind_of(k: PlannedKind) -> Option<FileKind> {
    match k {
        PlannedKind::Request => Some(FileKind::Request),
        PlannedKind::Folder => Some(FileKind::Folder),
        PlannedKind::Collection => Some(FileKind::Collection),
        _ => None,
    }
}

fn check_valid(p: &ImportPlan) {
    let reg = ProtocolRegistry::builtin();
    for f in &p.files {
        let Some(kind) = kind_of(f.kind) else {
            continue;
        };
        let doc = parse_with(&f.contents, kind, &reg);
        let errs: Vec<_> = doc
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(
            errs.is_empty(),
            "{}: {:?}\n{}",
            f.path.display(),
            errs,
            f.contents
        );
        assert!(
            is_canonical(&f.contents, kind),
            "{} is not canonical",
            f.path.display()
        );
        if kind == FileKind::Request {
            assert!(
                doc.request().is_some(),
                "{} has no request",
                f.path.display()
            );
        }
    }
    for l in &p.report.lossy {
        assert!(!l.feature.is_empty() && !l.reason.is_empty());
    }
}

fn snapshot_text(p: &ImportPlan) -> String {
    let mut s = String::new();
    for f in &p.files {
        s.push_str(&format!("=== {} ===\n{}\n", f.path.display(), f.contents));
    }
    s.push_str(&p.report.to_markdown(&p.files));
    s
}

macro_rules! case {
    ($name:ident, $src:expr, $rel:expr) => {
        case!($name, $src, $rel, Options::default());
    };
    ($name:ident, $src:expr, $rel:expr, $opts:expr) => {
        #[test]
        fn $name() {
            let p = run($src, $rel, &$opts);
            check_valid(&p);
            insta::assert_snapshot!(snapshot_text(&p));
        }
    };
}

case!(
    postman_shop,
    Source::Postman,
    "postman/shop-tests.postman_collection.json"
);
case!(
    postman_orders,
    Source::Postman,
    "postman/orders.postman_collection.json"
);
case!(
    postman_env,
    Source::Postman,
    "postman/local.postman_environment.json"
);
case!(
    insomnia_v4,
    Source::Insomnia,
    "insomnia/bookstore.insomnia.json"
);
case!(
    insomnia_v5,
    Source::Insomnia,
    "insomnia/notes.insomnia.yaml"
);
case!(openapi_petstore, Source::OpenApi, "openapi/petstore.yaml");
case!(openapi_swagger2, Source::OpenApi, "openapi/swagger2.json");
case!(
    openapi_spec_link,
    Source::OpenApi,
    "openapi/petstore.yaml",
    Options {
        spec_link: true,
        ..Default::default()
    }
);
case!(hurl_login, Source::Hurl, "hurl/login-flow.hurl");
case!(hurl_health, Source::Hurl, "hurl/health.hurl");
case!(http_users, Source::Http, "http/users.http");
case!(http_rest_client, Source::Http, "http/rest-client.http");

#[test]
fn detects_sources() {
    let cases = [
        (
            "postman/shop-tests.postman_collection.json",
            Source::Postman,
        ),
        ("postman/local.postman_environment.json", Source::Postman),
        ("insomnia/bookstore.insomnia.json", Source::Insomnia),
        ("insomnia/notes.insomnia.yaml", Source::Insomnia),
        ("openapi/petstore.yaml", Source::OpenApi),
        ("openapi/swagger2.json", Source::OpenApi),
        ("hurl/login-flow.hurl", Source::Hurl),
        ("http/users.http", Source::Http),
    ];
    for (rel, want) in cases {
        let path = fixture(rel);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(detect(&path, &text), Some(want), "{rel}");
    }
}

#[test]
fn postman_report_lists_unsupported_features() {
    let p = run(
        Source::Postman,
        "postman/shop-tests.postman_collection.json",
        &Options::default(),
    );
    let features: Vec<&str> = p.report.lossy.iter().map(|l| l.feature.as_str()).collect();
    assert!(features.contains(&"pm.visualizer"), "{features:?}");
    assert!(features.contains(&"require('moment')"), "{features:?}");
    assert!(features.iter().any(|f| f.contains("awsv4")), "{features:?}");
}

#[test]
fn secrets_go_to_local_env_and_gitignore() {
    let p = run(
        Source::Postman,
        "postman/local.postman_environment.json",
        &Options::default(),
    );
    let local = p
        .files
        .iter()
        .find(|f| f.kind == PlannedKind::LocalEnv)
        .expect("local env");
    assert!(local.contents.contains("password=hunter2"));
    assert!(local.contents.contains("apiToken=tok_123"));
    let shared = p.files.iter().find(|f| f.kind == PlannedKind::Env).unwrap();
    assert!(!shared.contents.contains("hunter2"));
    assert!(shared.contents.contains("baseUrl"));
    assert!(!shared.contents.contains("off="));
    let toml = p
        .files
        .iter()
        .find(|f| f.kind == PlannedKind::Config)
        .unwrap();
    assert!(toml.contents.contains("password"));
    assert!(
        p.files
            .iter()
            .any(|f| f.kind == PlannedKind::Gitignore && f.contents.contains("*.local.env"))
    );
}

#[test]
fn apply_never_overwrites_without_confirmation() {
    let mut p = run(Source::Hurl, "hurl/health.hurl", &Options::default());
    let dir = tempfile::tempdir().unwrap();
    let written = apply(&mut p, dir.path(), &|_| false).unwrap();
    assert!(!written.is_empty());
    let req = p
        .files
        .iter()
        .find(|f| f.kind == PlannedKind::Request)
        .unwrap()
        .path
        .clone();
    std::fs::write(dir.path().join(&req), "mine").unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target\n").unwrap();
    let written = apply(&mut p, dir.path(), &|_| false).unwrap();
    assert!(!written.contains(&req));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(&req)).unwrap(),
        "mine"
    );
    assert!(p.report.skipped.contains(&req));
    let gi = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gi.starts_with("target\n") && gi.contains("*.local.env"));
    apply(&mut p, dir.path(), &|_| true).unwrap();
    assert_ne!(
        std::fs::read_to_string(dir.path().join(&req)).unwrap(),
        "mine"
    );
}

#[test]
fn bad_input_is_a_parse_error() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x.json");
    std::fs::write(&f, "{ nope").unwrap();
    for s in [Source::Postman, Source::Insomnia, Source::OpenApi] {
        let e = plan(s, &f, &Options::default()).unwrap_err();
        assert!(matches!(e, fluxwire_import::ImportError::Parse(_)), "{s:?}");
    }
}

fn only_request(p: &ImportPlan) -> &str {
    let r: Vec<_> = p
        .files
        .iter()
        .filter(|f| f.kind == PlannedKind::Request)
        .collect();
    assert_eq!(r.len(), 1, "{r:?}");
    &r[0].contents
}

fn assert_graphql_request(text: &str, query_has: &str) {
    assert!(text.contains("requires: [graphql]"), "{text}");
    assert!(text.contains("```graphql query\n"), "{text}");
    assert!(text.contains(query_has), "{text}");
    // the operation is not left in the http block as a JSON body
    assert!(!text.contains("\"query\""), "{text}");
    let doc = parse_with(text, FileKind::Request, &ProtocolRegistry::builtin());
    assert!(
        doc.diagnostics
            .iter()
            .all(|d| d.severity != Severity::Error),
        "{:?}",
        doc.diagnostics
    );
    assert!(is_canonical(text, FileKind::Request));
}

#[test]
fn postman_graphql_body_becomes_a_graphql_request() {
    let p = run(
        Source::Postman,
        "postman/graphql.postman_collection.json",
        &Options::default(),
    );
    check_valid(&p);
    let t = only_request(&p);
    assert_graphql_request(t, "country(code: $code)");
    assert!(
        t.contains("```json variables\n{\n  \"code\": \"NO\"\n}\n```"),
        "{t}"
    );
}

#[test]
fn insomnia_graphql_body_becomes_a_graphql_request() {
    let p = run(
        Source::Insomnia,
        "insomnia/graphql.insomnia.json",
        &Options::default(),
    );
    check_valid(&p);
    assert_graphql_request(only_request(&p), "continents {");
}

#[test]
fn hurl_graphql_body_becomes_a_graphql_request() {
    let p = run(Source::Hurl, "hurl/graphql.hurl", &Options::default());
    check_valid(&p);
    let t = only_request(&p);
    assert_graphql_request(t, "country(code: $code)");
    assert!(t.contains("```json variables"), "{t}");
    assert!(!t.contains("variables {"), "{t}");
}
