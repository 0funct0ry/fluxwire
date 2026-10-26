use fluxwire_format::*;
use fluxwire_model::{CollectionConfig, FileKind, TreeNode};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("fluxwire-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn config_format_rules() {
    assert!(parse_config("[collection]\nname = \"x\"\nformat = 1\n").is_ok());
    assert_eq!(
        parse_config("[collection]\nname = \"x\"\n"),
        Err(OpenError::MissingFormat)
    );
    assert_eq!(parse_config(""), Err(OpenError::MissingFormat));
    match parse_config("[collection]\nformat = 2\n") {
        Err(OpenError::FormatTooNew {
            found: 2,
            supported: 1,
        }) => {}
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        parse_config("[collection"),
        Err(OpenError::Config(_))
    ));
    let _: CollectionConfig = parse_config("[collection]\nformat = 1\n[future]\nx = 1\n").unwrap();
}

#[test]
fn scans_payments_api_fixture() {
    let c = scan(&fixture("payments-api")).unwrap();
    let names: Vec<_> = c.tree.iter().map(|n| n.name().to_string()).collect();
    assert_eq!(
        names,
        ["Auth", "Payments", "refunds", "events", "README.md"].map(String::from),
        "{names:?}"
    );
    let TreeNode::Folder { children, .. } = &c.tree[1] else {
        panic!()
    };
    let reqs: Vec<_> = children.iter().map(|n| n.name().to_string()).collect();
    assert_eq!(reqs, ["Create payment", "Get payment", "List payments"]);
    assert_eq!(c.readme.as_deref(), Some("README.md"));
}

#[test]
fn every_fixture_request_parses_without_errors() {
    for e in walk(&fixture("payments-api")) {
        if is_request_file(&e) || e.ends_with("folder.flux.md") || e.ends_with("collection.flux.md")
        {
            let src = std::fs::read_to_string(&e).unwrap();
            let d = parse_with(
                &src,
                file_kind_for(&e),
                &fluxwire_protocol::ProtocolRegistry::builtin(),
            );
            let errs: Vec<_> = d
                .diagnostics
                .iter()
                .filter(|x| x.severity == fluxwire_model::Severity::Error)
                .collect();
            assert!(errs.is_empty(), "{}: {errs:?}", e.display());
            assert!(
                is_canonical(&src, d.kind),
                "{} is not canonical",
                e.display()
            );
        }
    }
}

fn walk(p: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in std::fs::read_dir(p).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(walk(&p))
        } else {
            v.push(p)
        }
    }
    v
}

#[test]
fn discovery_finds_nested_collections() {
    let root = tmp("discover");
    create_collection(&root.join("a"), "A").unwrap();
    create_collection(&root.join("deep/b"), "B").unwrap();
    std::fs::create_dir_all(root.join("node_modules/c")).unwrap();
    create_collection(&root.join("node_modules/c"), "C").unwrap();
    let found = find_collections(&root);
    assert_eq!(found.len(), 2, "{found:?}");
}

#[test]
fn create_collection_layout() {
    let root = tmp("create");
    create_collection(&root, "My API").unwrap();
    assert!(root.join("environments").is_dir());
    assert_eq!(
        std::fs::read_to_string(root.join(".gitignore")).unwrap(),
        GITIGNORE
    );
    let c = scan(&root).unwrap();
    assert_eq!(c.config.collection.name, "My API");
    assert!(c.tree.is_empty());
    assert!(is_canonical(
        &std::fs::read_to_string(root.join("collection.flux.md")).unwrap(),
        FileKind::Collection
    ));
}

#[test]
fn file_operations() {
    let root = tmp("ops");
    create_collection(&root, "X").unwrap();
    let f = create_folder(&root, "Orders").unwrap();
    let r = create_request(&f, "List orders", "Lists.", false).unwrap();
    assert!(r.ends_with("orders/list-orders.flux.md"));
    let r2 = create_request(&f, "List orders", "", true).unwrap();
    assert!(r2.ends_with("list-orders-2.flux.md"));

    let renamed = rename(&r, "Fetch orders").unwrap();
    assert!(renamed.ends_with("fetch-orders.flux.md"));
    let text = std::fs::read_to_string(&renamed).unwrap();
    assert!(text.contains("name: Fetch orders"));
    assert!(text.contains("# List orders"), "prose must be untouched");

    let dup = duplicate(&renamed).unwrap();
    assert!(
        std::fs::read_to_string(&dup)
            .unwrap()
            .contains("name: Fetch orders copy")
    );

    let other = create_folder(&root, "Archive").unwrap();
    let moved = move_to(&dup, &other).unwrap();
    assert!(moved.exists() && !dup.exists());
    assert!(move_to(&f, &f.join("nested")).is_err());

    let rf = rename(&f, "Sales").unwrap();
    assert!(rf.ends_with("sales"));
    assert!(
        std::fs::read_to_string(rf.join("folder.flux.md"))
            .unwrap()
            .contains("name: Sales")
    );

    let c = scan(&root).unwrap();
    assert_eq!(c.tree.len(), 2);
}

#[test]
fn watcher_reports_external_edit() {
    let root = tmp("watch");
    create_collection(&root, "W").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let _h = watch(&root, tx).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let p = root.join("a.flux.md");
    let start = std::time::Instant::now();
    std::fs::write(&p, new_request("A", "", "GET", "/")).unwrap();
    let batch = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("no event");
    assert!(
        batch.iter().any(|b| b.file_name() == p.file_name()),
        "{batch:?}"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_millis(1000),
        "{:?}",
        start.elapsed()
    );
}
