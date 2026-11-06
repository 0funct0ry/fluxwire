//! `fluxwire openapi` end to end: convergence of one pull and one push, and exit code 4.

use assert_cmd::Command;
use std::path::Path;

const SPEC: &str = "openapi: 3.0.3\n# keep this comment\npaths:\n  /pets/{id}:\n    get:\n      operationId: getPet\n      summary: Get a pet\n      parameters:\n        - name: id\n          in: path\n          required: true\n          description: Pet id\n          example: p1\n      responses:\n        '200': {description: ok}\n";
const REQ: &str = "---\nname: Get a pet\noperation: getPet\npath:\n  id: p1\n---\n\n# Get a pet\n\n```http\nGET {{baseUrl}}/pets/:id\n```\n\n```http expect\nHTTP 200\n```\n";

fn fw(root: &Path) -> Command {
    let mut c = Command::cargo_bin("fluxwire-cli").unwrap();
    c.args(["openapi"]);
    c.current_dir(root);
    c
}

fn setup() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    std::fs::create_dir_all(r.join("openapi")).unwrap();
    std::fs::create_dir_all(r.join("pets")).unwrap();
    std::fs::write(
        r.join("fluxwire.toml"),
        "[collection]\nformat = 1\nname = \"t\"\n\n[[openapi]]\nname = \"p\"\nsource = \"openapi/p.yaml\"\nfolder = \"pets\"\nmode = \"two-way\"\n",
    )
    .unwrap();
    std::fs::write(r.join("openapi/p.yaml"), SPEC).unwrap();
    std::fs::write(r.join("pets/get-pet.flux.md"), REQ).unwrap();
    // The initial snapshot is the spec as projected (what the importer writes).
    std::fs::create_dir_all(r.join(".fluxwire/sync")).unwrap();
    let st = fluxwire_openapi::link::load(
        r,
        &fluxwire_format::load_config(r).unwrap().openapi[0],
        None,
    )
    .unwrap();
    std::fs::write(r.join(".fluxwire/sync/p.base.json"), st.spec.to_base_json()).unwrap();
    d
}

#[test]
fn pull_and_push_converge_touching_only_the_edited_lines() {
    let d = setup();
    let r = d.path();
    std::fs::write(
        r.join("openapi/p.yaml"),
        SPEC.replace("Pet id", "The pet id"),
    )
    .unwrap();
    std::fs::write(
        r.join("pets/get-pet.flux.md"),
        REQ.replace("name: Get a pet", "name: Fetch pet"),
    )
    .unwrap();
    assert!(status(r).contains("1 incoming, 1 outgoing, 0 conflicts"));
    fw(r).args(["pull", "--yes"]).assert().success();
    fw(r).args(["push", "--yes"]).assert().success();
    let spec = std::fs::read_to_string(r.join("openapi/p.yaml")).unwrap();
    assert_eq!(
        spec,
        SPEC.replace("Pet id", "The pet id")
            .replace("Get a pet", "Fetch pet")
    );
    assert!(status(r).contains("0 incoming, 0 outgoing, 0 conflicts"));
}

#[test]
fn conflicts_block_pull_and_push_with_exit_4() {
    let d = setup();
    let r = d.path();
    std::fs::write(
        r.join("openapi/p.yaml"),
        SPEC.replace("summary: Get a pet", "summary: Spec"),
    )
    .unwrap();
    std::fs::write(
        r.join("pets/get-pet.flux.md"),
        REQ.replace("name: Get a pet", "name: App"),
    )
    .unwrap();
    fw(r).arg("status").assert().code(4);
    fw(r).args(["pull", "--yes"]).assert().code(4);
    fw(r).args(["push", "--yes"]).assert().code(4);
}

#[test]
fn unknown_link_is_a_usage_error() {
    let d = setup();
    fw(d.path())
        .args(["status", "--link", "nope"])
        .assert()
        .code(64);
}

fn status(r: &Path) -> String {
    let out = fw(r)
        .arg("status")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8_lossy(&out).into_owned()
}

#[test]
fn a_fresh_spec_link_import_is_in_sync() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("c");
    let spec =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/import/openapi/petstore.yaml");
    Command::cargo_bin("fluxwire-cli")
        .unwrap()
        .args(["import", "openapi"])
        .arg(&spec)
        .arg("--into")
        .arg(&out)
        .arg("--spec-link")
        .assert()
        .success();
    let s = status(&out);
    assert!(s.contains("0 incoming, 0 outgoing, 0 conflicts"), "{s}");
}

#[test]
fn spec_link_import_into_an_existing_collection_is_recorded() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("fluxwire.toml"),
        "[collection]\nname = \"brightcart\"\nformat = 1\n",
    )
    .unwrap();
    let spec =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/import/openapi/petstore.yaml");
    Command::cargo_bin("fluxwire-cli")
        .unwrap()
        .args(["import", "openapi"])
        .arg(&spec)
        .arg("--into")
        .arg(d.path())
        .arg("--spec-link")
        .assert()
        .success();
    let toml = std::fs::read_to_string(d.path().join("fluxwire.toml")).unwrap();
    assert!(
        toml.contains("[[openapi]]") && toml.contains("brightcart"),
        "{toml}"
    );
    assert!(status(d.path()).contains("0 incoming, 0 outgoing, 0 conflicts"));
}

#[test]
fn a_yml_source_is_written_as_yaml_and_matches_the_link() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("petstore.yml");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/import/openapi/petstore.yaml"),
        &src,
    )
    .unwrap();
    let out = d.path().join("c");
    Command::cargo_bin("fluxwire-cli")
        .unwrap()
        .args(["import", "openapi"])
        .arg(&src)
        .arg("--into")
        .arg(&out)
        .arg("--spec-link")
        .assert()
        .success();
    assert!(
        out.join("petstore/petstore.yaml").is_file() && !out.join("petstore/petstore.yml").exists()
    );
    assert!(status(&out).contains("0 incoming, 0 outgoing, 0 conflicts"));
}
