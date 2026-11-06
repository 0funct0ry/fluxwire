use fluxwire_format::*;
use fluxwire_model::{BlockKind, FileKind, FluxDoc, FmValue, apply_edits};
use proptest::prelude::*;
use std::fmt::Write;

fn corpus() -> Vec<(String, String)> {
    let mut v: Vec<_> = std::fs::read_dir("tests/corpus")
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

fn summary(d: &FluxDoc) -> String {
    let mut s = String::new();
    writeln!(
        s,
        "runnable: {}  unsupported: {:?}  kind: {:?}",
        d.runnable(),
        d.unsupported,
        d.kind
    )
    .unwrap();
    writeln!(s, "front matter:").unwrap();
    for e in &d.front.entries {
        writeln!(s, "  {} = {:?}", e.key, e.value).unwrap();
    }
    writeln!(s, "blocks:").unwrap();
    for (b, data) in d.blocks.iter().zip(&d.data) {
        writeln!(
            s,
            "  {:?} lang={} role={:?} label={:?} closed={}",
            b.kind, b.info.language, b.info.role, b.info.label, b.closed
        )
        .unwrap();
        match data {
            fluxwire_model::BlockData::Request(r) => {
                writeln!(
                    s,
                    "    request {} {} version={:?}",
                    r.method, r.target, r.version
                )
                .unwrap();
                for q in &r.query {
                    writeln!(
                        s,
                        "    query enabled={} {}={:?}",
                        q.enabled, q.name, q.value
                    )
                    .unwrap();
                }
                for h in &r.headers {
                    writeln!(
                        s,
                        "    header enabled={} {}: {}",
                        h.enabled, h.name, h.value
                    )
                    .unwrap();
                }
                writeln!(
                    s,
                    "    body {:?}",
                    match &r.body {
                        fluxwire_model::Body::None => "none".to_string(),
                        fluxwire_model::Body::Raw(sp) => format!("raw {:?}", sp.slice(&d.source)),
                        fluxwire_model::Body::File { path, .. } => format!("file {path}"),
                    }
                )
                .unwrap();
            }
            fluxwire_model::BlockData::Expect(x) => {
                writeln!(
                    s,
                    "    expect {:?} {:?} headers={}",
                    x.status,
                    x.version,
                    x.headers.len()
                )
                .unwrap();
            }
            fluxwire_model::BlockData::Kv(k) => writeln!(s, "    kv lines={}", k.len()).unwrap(),
            fluxwire_model::BlockData::None => {}
        }
    }
    writeln!(s, "diagnostics:").unwrap();
    for x in &d.diagnostics {
        let (l, c) = fluxwire_model::line_col(&d.source, x.span.start);
        writeln!(
            s,
            "  {} {:?} {}:{} {} fix={:?}",
            x.code,
            x.severity,
            l + 1,
            c + 1,
            x.message,
            x.fix.as_ref().map(|f| &f.label)
        )
        .unwrap();
    }
    s
}

#[test]
fn snapshots_over_corpus() {
    for (name, src) in corpus() {
        let d = parse(&src);
        insta::with_settings!({snapshot_suffix => name.clone()}, {
            insta::assert_snapshot!("parse", summary(&d));
        });
    }
}

#[test]
fn unmodified_roundtrip_is_byte_exact() {
    for (_, src) in corpus() {
        assert_eq!(parse(&src).source, src);
    }
}

#[test]
fn fmt_idempotent_on_corpus() {
    for (name, src) in corpus() {
        let once = format(&src, FileKind::Request);
        let twice = format(&once, FileKind::Request);
        assert_eq!(once, twice, "not idempotent: {name}");
    }
}

#[test]
fn spec_example_is_canonical() {
    let src = std::fs::read_to_string("tests/corpus/create-payment.flux.md").unwrap();
    assert!(is_canonical(&src, FileKind::Request));
}

#[test]
fn unknown_content_preserved_by_fmt() {
    let src = std::fs::read_to_string("tests/corpus/future-grpc.flux.md").unwrap();
    assert!(
        is_canonical(&src, FileKind::Request),
        "{}",
        format(&src, FileKind::Request)
    );
    let d = parse(&src);
    assert!(!d.runnable());
    assert!(d.diagnostics.iter().any(|x| x.code == "F401"));
    assert!(d.blocks.iter().any(|b| b.kind == BlockKind::Reserved));
    assert!(d.blocks.iter().any(|b| b.kind == BlockKind::Extension));
    assert_eq!(d.front.str("x-owner"), Some("payments-team"));
}

#[test]
fn fmt_normalises_agent_style() {
    let src = std::fs::read_to_string("tests/corpus/agent-written.flux.md").unwrap();
    let out = format(&src, FileKind::Request);
    assert!(
        out.contains("GET {{baseUrl}}/v1/payments/:paymentId HTTP/1.1\nAccept: application/json")
    );
    assert!(out.contains("Fetches a payment by id.\n\n```http\n"));
}

#[test]
fn fmt_converts_crlf() {
    let src = std::fs::read_to_string("tests/corpus/crlf.flux.md").unwrap();
    assert!(!format(&src, FileKind::Request).contains('\r'));
}

#[test]
fn fmt_reorders_front_matter_keeping_comments() {
    let src = "---\nseq: 2\n# about the name\nname: X\nx-foo: 1\nrequires: []\n---\n";
    let out = format(src, FileKind::Request);
    assert!(
        out.starts_with("---\nrequires: []\n# about the name\nname: X"),
        "{out}"
    );
    assert!(out.contains("# about the name\nname: X"));
    assert!(out.find("seq").unwrap() > out.find("name").unwrap());
    assert!(out.find("x-foo").unwrap() > out.find("seq").unwrap());
}

#[test]
fn span_edits_touch_only_their_lines() {
    let src = std::fs::read_to_string("tests/corpus/create-payment.flux.md").unwrap();
    let d = parse(&src);
    let mut edits = set_request_line(&d, "PUT", "{{baseUrl}}/v1/payments");
    edits.extend(set_header(&d, 0, true, "Content-Type", "text/plain"));
    let out = apply_edits(&src, &edits).unwrap();
    let diff: Vec<_> = src
        .lines()
        .zip(out.lines())
        .filter(|(a, b)| a != b)
        .collect();
    assert_eq!(diff.len(), 2, "{diff:?}");
    assert_eq!(src.lines().count(), out.lines().count());
}

#[test]
fn header_add_remove_and_body_edits() {
    let src = "---\nname: A\n---\n\n# A\n\n```http\nGET /a\nX-A: 1\n```\n";
    let d = parse(src);
    let out = apply_edits(src, &add_header(&d, "X-B", "2")).unwrap();
    assert!(out.contains("X-A: 1\nX-B: 2\n```"));
    let d2 = parse(&out);
    let out = apply_edits(&out, &remove_header(&d2, 0)).unwrap();
    assert!(!out.contains("X-A"));
    let d3 = parse(&out);
    let out = apply_edits(&out, &set_body(&d3, "{\"a\":1}")).unwrap();
    assert!(out.contains("X-B: 2\n\n{\"a\":1}\n```"), "{out}");
    let d4 = parse(&out);
    assert!(matches!(
        d4.request().unwrap().body,
        fluxwire_model::Body::Raw(_)
    ));
    let out = apply_edits(&out, &set_body(&d4, "{\"a\":2}")).unwrap();
    assert!(out.contains("{\"a\":2}\n```"));
}

#[test]
fn query_edits() {
    let src = "---\nname: A\n---\n\n```http\nGET /a\n```\n";
    let d = parse(src);
    let out = apply_edits(src, &add_query(&d, "x", Some("1"))).unwrap();
    assert!(out.contains("GET /a\n    ?x=1\n```"));
    let d = parse(&out);
    let out = apply_edits(&out, &add_query(&d, "y", None)).unwrap();
    assert!(out.contains("    ?x=1\n    &y\n```"));
}

#[test]
fn insert_block_default_positions() {
    let src = "---\nname: A\n---\n\n# A\n\n```http\nGET /a\n```\n\nprose\n\n```ts test\nx\n```\n";
    let d = parse(src);
    let out = apply_edits(
        src,
        &insert_block(&d, BlockKind::Expect, "http expect", "HTTP 200"),
    )
    .unwrap();
    let e = out.find("http expect").unwrap();
    assert!(e > out.find("GET /a").unwrap() && e < out.find("ts test").unwrap());
    let d = parse(&out);
    let out = apply_edits(&out, &insert_block(&d, BlockKind::Pre, "ts pre", "y")).unwrap();
    assert!(out.find("ts pre").unwrap() < out.find("ts test").unwrap());
    assert!(is_canonical(&out, FileKind::Request), "{out}");
}

#[test]
fn front_matter_set_inserts_in_order() {
    let src = "---\nname: A\nsettings:\n  timeout_ms: 5\n---\n";
    let d = parse(src);
    let out = apply_edits(
        src,
        &set_front_matter(&d, "seq", &FmValue::Scalar("3".into())),
    )
    .unwrap();
    assert_eq!(
        out,
        "---\nname: A\nseq: 3\nsettings:\n  timeout_ms: 5\n---\n"
    );
    let d = parse(&out);
    let out = apply_edits(
        &out,
        &set_front_matter(&d, "tags", &FmValue::List(vec!["a".into(), "b c".into()])),
    )
    .unwrap();
    assert!(out.contains("seq: 3\ntags: [a, b c]\nsettings"));
}

#[test]
fn templates_are_canonical_and_runnable() {
    let r = new_request("Get thing", "Fetches a thing.", "GET", "{{baseUrl}}/things");
    assert!(is_canonical(&r, FileKind::Request));
    assert!(parse(&r).runnable());
    let w = new_websocket("Stream", "", "wss://x/y");
    assert!(is_canonical(&w, FileKind::Request));
    assert!(parse(&w).is_websocket());
}

proptest! {
    #[test]
    fn fmt_is_idempotent(s in "[ -~\\n]{0,300}") {
        let src = format!("---\nname: T\n---\n\n{s}\n");
        let once = format(&src, FileKind::Request);
        prop_assert_eq!(format(&once, FileKind::Request), once);
    }

    #[test]
    fn parse_never_changes_bytes_or_panics(s in "\\PC{0,400}") {
        let d = parse(&s);
        prop_assert_eq!(d.source, s);
    }

    #[test]
    fn header_edits_never_change_prose(
        prose in "PROSE[a-z]{1,20}",
        name in "[A-Za-z][A-Za-z0-9-]{0,8}",
        value in "[a-z0-9]{0,8}",
    ) {
        let src = format!("---\nname: T\n---\n\n# T\n\n{prose}\n\n```http\nGET /a\nX-A: 1\n```\n\nafter {prose}\n");
        let d = parse(&src);
        let out = apply_edits(&src, &add_header(&d, &name, &value)).unwrap();
        let prose_lines = |s: &str| -> Vec<String> {
            s.lines().filter(|l| l.contains(&prose)).map(str::to_string).collect()
        };
        prop_assert_eq!(prose_lines(&src), prose_lines(&out));
        let d2 = parse(&out);
        prop_assert_eq!(d2.request().unwrap().headers.len(), 2);
    }

    #[test]
    fn fmt_never_changes_prose_lines(
        a in "[A-Za-z][A-Za-z ,.]{0,30}",
        b in "[A-Za-z][A-Za-z ,.]{0,30}",
    ) {
        let src = format!("---\nname: T\n---\n{a}\n```http\nGET   /x\n```\n\n\n{b}\n");
        let out = format(&src, FileKind::Request);
        for p in [&a, &b] {
            prop_assert!(out.lines().any(|l| l == p.as_str()), "lost prose line");
        }
    }
}

#[test]
fn saved_messages_are_typed_with_labels_and_file_refs() {
    let src = std::fs::read_to_string("tests/corpus/websocket-messages.flux.md").unwrap();
    let m = parse(&src).messages();
    let got: Vec<_> = m
        .iter()
        .map(|m| (m.label.as_str(), m.language.as_str(), m.file.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            ("subscribe", "json", None),
            ("legacy", "xml", None),
            ("blob", "binary", Some("fixtures/frame.bin")),
        ]
    );
    assert_eq!(m[0].frame_kind(), "text");
    assert_eq!(m[2].frame_kind(), "binary");
}

#[test]
fn query_rows_cover_url_and_continuation_lines() {
    let src = "---\nname: A\n---\n\n```http\nGET /a?x=1&y=2#top\n    &z=3\n    # &off=4\n```\n";
    let d = parse(src);
    let rows = query_rows(&d);
    let names: Vec<_> = rows
        .iter()
        .map(|r| (r.name.as_str(), r.enabled, r.in_url))
        .collect();
    assert_eq!(
        names,
        [
            ("x", true, true),
            ("y", true, true),
            ("z", true, false),
            ("off", false, false)
        ]
    );

    // Editing a URL row rewrites only that pair of the target.
    let out = apply_edits(src, &set_query(&d, 0, true, "x", Some("9"))).unwrap();
    assert!(out.contains("GET /a?x=9&y=2#top\n    &z=3\n"), "{out}");
    // Disabling a URL row moves it to a commented continuation line, after the existing ones.
    let out = apply_edits(src, &set_query(&d, 1, false, "y", Some("2"))).unwrap();
    assert!(
        out.contains("GET /a?x=1#top\n    &z=3\n    # &off=4\n    # &y=2\n```"),
        "{out}"
    );
    // Removing the only URL pair drops the `?`.
    let one = "---\nname: A\n---\n\n```http\nGET /a?x=1\n```\n";
    let d1 = parse(one);
    let out = apply_edits(one, &remove_query(&d1, 0)).unwrap();
    assert!(out.contains("GET /a\n```"), "{out}");
    // A continuation row keeps working with the shifted index.
    let out = apply_edits(src, &remove_query(&d, 2)).unwrap();
    assert!(!out.contains("&z=3") && out.contains("# &off=4"), "{out}");
    let out = apply_edits(src, &set_query(&d, 2, true, "z", Some("7"))).unwrap();
    assert!(out.contains("    &z=7\n"), "{out}");
    // Adding appends a continuation line with `&` because the URL already has a query.
    let out = apply_edits(src, &add_query(&d, "n", Some("1"))).unwrap();
    assert!(out.contains("# &off=4\n    &n=1\n```"), "{out}");
    // Variables are opaque, and an untouched target stays byte for byte.
    let v = "---\nname: A\n---\n\n```http\nGET {{base}}/a?q={{a&b}}&t=1\n```\n";
    let dv = parse(v);
    assert_eq!(query_rows(&dv).len(), 2);
    let out = apply_edits(v, &set_query(&dv, 1, true, "t", Some("2"))).unwrap();
    assert!(out.contains("GET {{base}}/a?q={{a&b}}&t=2\n"), "{out}");
}

#[test]
fn path_param_names_follow_the_url_path_only() {
    assert_eq!(
        path_param_names("https://example.com/v1/users/:id/posts/:post_id?x=:no#:frag"),
        ["id", "post_id"]
    );
    assert_eq!(
        path_param_names("http://localhost:8080/a/:id/b/:id"),
        ["id"]
    );
    assert!(path_param_names("http://localhost:8080").is_empty());
    assert!(path_param_names("{{base}}/users/{{id}}").is_empty());
    assert_eq!(path_param_names("{{base}}/users/:id"), ["id"]);
    assert!(path_param_names("https://x.test/a/:/b/:a.b").is_empty());
}

#[test]
fn set_path_param_edits_only_the_path_map() {
    let src = "---\nname: X\nx-keep: 1\n---\n\n```http\nGET https://x.test/u/:id/p/:pid\n```\n";
    let d = parse(src);
    assert_eq!(path_values(&d), []);
    let s1 = apply_edits(src, &set_path_param(&d, "id", "42")).unwrap();
    assert!(s1.contains("path:\n  id: 42\n"), "{s1}");
    assert!(s1.contains("x-keep: 1"));
    let d1 = parse(&s1);
    let s2 = apply_edits(&s1, &set_path_param(&d1, "pid", "{{p}}")).unwrap();
    assert_eq!(
        path_values(&parse(&s2)),
        [("id".into(), "42".into()), ("pid".into(), "{{p}}".into())]
    );
    let s3 = apply_edits(&s2, &set_path_param(&parse(&s2), "id", "")).unwrap();
    assert_eq!(path_values(&parse(&s3)), [("pid".into(), "{{p}}".into())]);
    let s4 = apply_edits(&s3, &set_path_param(&parse(&s3), "pid", "")).unwrap();
    assert!(!s4.contains("path:"), "{s4}");
    assert_eq!(s4, src);
}

#[test]
fn path_param_ranges_cover_each_segment() {
    let t = "{{base}}/u/:id/p/:id?x=1";
    let r = path_param_ranges(t);
    assert_eq!(r.len(), 2);
    assert_eq!(&t[r[0].1.clone()], ":id");
    assert_eq!(r[1].1, 17..20);
}

#[test]
fn url_bar_text_includes_enabled_query_lines_and_syncs_back() {
    let src = "```http\nGET https://x.test/u\n    ?page=2\n    &limit=10\n    # &off=1\n```\n";
    let d = parse(src);
    assert_eq!(display_url(&d), "https://x.test/u?page=2&limit=10");
    assert!(set_url_text(&d, "GET", "https://x.test/u?page=2&limit=10").is_empty());
    // Removing a parameter in the bar removes it from the table.
    let s = apply_edits(src, &set_url_text(&d, "GET", "https://x.test/u?page=2")).unwrap();
    let rows = query_rows(&parse(&s));
    assert_eq!(rows.len(), 2, "{s}");
    assert_eq!(rows[0].name, "page");
    assert!(!rows[1].enabled && rows[1].name == "off", "{s}");
    // Adding one in the bar adds it to the table.
    let s = apply_edits(
        &s,
        &set_url_text(&parse(&s), "GET", "https://x.test/u?page=2&q=z"),
    )
    .unwrap();
    let names: Vec<_> = query_rows(&parse(&s)).into_iter().map(|q| q.name).collect();
    assert_eq!(names, ["page", "q", "off"], "{s}");
}

#[test]
fn remove_block_takes_one_block_and_its_blank_line() {
    let src =
        "```http\nGET /a\n```\n\n```ts pre\n1\n```\n\n```ts pre\n2\n```\n\n```ts pre\n3\n```\n";
    let d = parse(src);
    let idx: Vec<usize> = d
        .blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| b.kind == BlockKind::Pre)
        .map(|(i, _)| i)
        .collect();
    let out = apply_edits(src, &remove_block(&d, idx[1])).unwrap();
    assert_eq!(
        out,
        "```http\nGET /a\n```\n\n```ts pre\n1\n```\n\n```ts pre\n3\n```\n"
    );
    let out = apply_edits(src, &remove_block(&d, idx[2])).unwrap();
    assert!(
        out.ends_with("2\n```\n\n") || out.ends_with("2\n```\n"),
        "{out:?}"
    );
}

#[test]
fn block_title_is_set_replaced_and_removed_in_the_info_string_only() {
    let src = "```http\nGET /a\n```\n\n```ts pre\nx\n```\n";
    let d = parse(src);
    let pre = d
        .blocks
        .iter()
        .position(|b| b.kind == BlockKind::Pre)
        .unwrap();
    let s1 = apply_edits(src, &set_block_title(&d, pre, "Store auth in header")).unwrap();
    assert!(
        s1.contains("```ts pre title=\"Store auth in header\"\nx\n```"),
        "{s1}"
    );
    let d1 = parse(&s1);
    assert_eq!(block_title(&d1.blocks[pre]), Some("Store auth in header"));
    assert_eq!(d1.blocks[pre].kind, BlockKind::Pre);
    let s2 = apply_edits(&s1, &set_block_title(&d1, pre, "Say \"hi\"")).unwrap();
    assert!(
        s2.contains("title=\"Say  hi\"") || s2.contains("title=\"Say hi\""),
        "{s2}"
    );
    assert_eq!(s2.matches("title=").count(), 1);
    let s3 = apply_edits(&s2, &set_block_title(&parse(&s2), pre, "  ")).unwrap();
    assert_eq!(s3, src);
    assert!(set_block_title(&parse(src), pre, "").is_empty());
}
