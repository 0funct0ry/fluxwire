use std::collections::BTreeMap;
use std::sync::Arc;

use fluxwire_script::*;
use serde_json::{Value, json};

fn block(role: Role, lang: Lang, src: &str) -> ScriptBlock {
    ScriptBlock {
        role,
        lang,
        source: src.into(),
        file: "r.flux.md".into(),
        first_line: 10,
    }
}

fn state() -> ScriptState {
    ScriptState {
        request: Some(
            json!({"method":"POST","url":"http://x/v1","headers":[["Content-Type","application/json"]],"body":"{\"a\":1}","name":"Create","path":"payments/create.flux.md"}),
        ),
        response: Some(
            json!({"status":201,"statusText":"Created","headers":[["Location","/p/1"]],"text":"{\"id\":\"p_1\",\"n\":3}","time":12,"size":20,"timings":{}}),
        ),
        env_name: "staging".into(),
        ..Default::default()
    }
}

fn run(src: &str) -> (ScriptState, Vec<ScriptError>) {
    run_with(Role::Test, Lang::Ts, src, &RunEnv::default())
}

fn run_with(role: Role, lang: Lang, src: &str, env: &RunEnv) -> (ScriptState, Vec<ScriptError>) {
    run_blocks(&[block(role, lang, src)], state(), env)
}

#[test]
fn tests_and_expect_matchers() {
    let (st, errs) = run(r#"
        fw.test("ok", () => fw.expect(fw.response.json()).toHaveProperty("id", "p_1"));
        fw.test("neg", () => fw.expect(3).not.toBe(4));
        fw.test("len", () => fw.expect([1,2]).toHaveLength(2));
        fw.test("fails", () => fw.expect(1).toBe(2));
        fw.test("schema", () => fw.expect({id:"x"}).toMatchSchema({type:"object", required:["id"]}));
        fw.test("schema bad", () => fw.expect({}).toMatchSchema({type:"object", required:["id"]}));
        fw.test("async", async () => { await fw.sleep(1); fw.expect("abc").toMatch(/b/); });
        fw.test("oneOf", () => fw.expect(2).toBeOneOf([1,2]));
        fw.test("cmp", () => { fw.expect(5).toBeGreaterThan(4); fw.expect(5).toBeLessThan(6); });
    "#);
    assert!(errs.is_empty(), "{errs:?}");
    let by: BTreeMap<_, _> = st.tests.iter().map(|t| (t.name.as_str(), t)).collect();
    assert!(by["ok"].passed && by["neg"].passed && by["len"].passed && by["async"].passed);
    assert!(by["schema"].passed && by["oneOf"].passed && by["cmp"].passed);
    assert!(!by["fails"].passed);
    assert_eq!(by["fails"].expected.as_deref(), Some("2"));
    assert_eq!(by["fails"].actual.as_deref(), Some("1"));
    assert!(!by["schema bad"].passed);
}

#[test]
fn vars_env_and_request_edits() {
    let (st, errs) = run_with(
        Role::Pre,
        Lang::Ts,
        r#"
        fw.vars.set("idemKey", crypto.randomUUID());
        fw.vars.setSecret("tok", "s3");
        fw.env.set("x", "1");
        fw.env.persist("x");
        fw.request.headers.set("X-Sig", "abc");
        fw.request.headers.remove("Content-Type");
        fw.request.body.set({ b: 2 });
        console.log("hello", { a: 1 });
        console.warn("careful");
        const t: string = fw.crypto.sha256("abc");
        fw.vars.set("sha", t);
        fw.vars.set("hm", fw.crypto.hmac("sha256", "k", "d"));
        fw.vars.set("env", fw.env.name);
    "#,
        &RunEnv::default(),
    );
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(st.vars_changed["idemKey"].as_ref().unwrap().value.len(), 36);
    assert!(st.vars_changed["tok"].as_ref().unwrap().secret);
    assert_eq!(st.env_set["x"], "1");
    assert_eq!(st.persist, vec!["x"]);
    assert_eq!(st.request_ops.len(), 3);
    assert_eq!(st.request_ops[2], json!({"op":"body","value":"{\"b\":2}"}));
    assert_eq!(st.console[0].text, "hello {\"a\":1}");
    assert_eq!(
        st.vars_changed["sha"].as_ref().unwrap().value,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(st.vars_changed["env"].as_ref().unwrap().value, "staging");
}

#[test]
fn skip_from_pre() {
    let (st, errs) = run_with(
        Role::Pre,
        Lang::Js,
        "fw.skip('not today')",
        &RunEnv::default(),
    );
    assert!(errs.is_empty());
    assert_eq!(st.skip.as_deref(), Some("not today"));
}

#[test]
fn error_maps_to_original_line() {
    // first_line = 10, throw is on the 3rd source line => line 12 even after TS stripping.
    let src = "type A = { x: number };\nconst a: A = { x: 1 };\nthrow new Error('boom');\n";
    let (_, errs) = run_with(Role::Post, Lang::Ts, src, &RunEnv::default());
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::Throw);
    assert_eq!(errs[0].message, "boom");
    assert_eq!(
        (errs[0].file.as_str(), errs[0].line),
        ("r.flux.md", 12),
        "{:?}",
        errs[0]
    );
}

#[test]
fn syntax_error_is_reported() {
    let (_, errs) = run_with(Role::Post, Lang::Ts, "let = ;", &RunEnv::default());
    assert_eq!(errs[0].kind, ErrorKind::Syntax);
}

#[test]
fn imports_only_from_lib() {
    let dir = std::env::temp_dir().join(format!("fw-script-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/sign.ts"),
        "export function sign(a: string, k: string): string { return a + ':' + k; }\n",
    )
    .unwrap();
    std::fs::write(dir.join("secret.ts"), "export const x = 1;\n").unwrap();
    let env = RunEnv {
        lib_dir: Some(dir.join("lib")),
        ..Default::default()
    };

    let (st, errs) = run_with(
        Role::Pre,
        Lang::Ts,
        "import { sign } from 'lib/sign.ts';\nfw.vars.set('s', sign('a', 'b'));",
        &env,
    );
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(st.vars_changed["s"].as_ref().unwrap().value, "a:b");

    for bad in [
        "../secret.ts",
        "lib/../secret.ts",
        "/etc/passwd",
        "secret.ts",
    ] {
        let (_, errs) = run_with(Role::Pre, Lang::Ts, &format!("import '{bad}';"), &env);
        assert_eq!(errs.len(), 1, "{bad}");
        assert_eq!(errs[0].kind, ErrorKind::Import, "{bad}: {:?}", errs[0]);
    }
    let (_, errs) = run_with(
        Role::Pre,
        Lang::Ts,
        "import { sign } from 'lib/sign.ts'; sign('a','b');",
        &RunEnv::default(),
    );
    assert_eq!(errs[0].kind, ErrorKind::Import);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn error_inside_lib_maps_to_lib_file() {
    let dir = std::env::temp_dir().join(format!("fw-script-lib-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(
        dir.join("lib/bad.ts"),
        "export function f(): void {\n  throw new Error('inside');\n}\n",
    )
    .unwrap();
    let env = RunEnv {
        lib_dir: Some(dir.join("lib")),
        ..Default::default()
    };
    let (_, errs) = run_with(
        Role::Pre,
        Lang::Ts,
        "import { f } from 'lib/bad.ts';\nf();",
        &env,
    );
    assert_eq!(errs[0].message, "inside");
    assert_eq!(
        (errs[0].file.as_str(), errs[0].line),
        ("lib/bad.ts", 2),
        "{:?}",
        errs[0]
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn sandbox_infinite_loop_times_out() {
    let env = RunEnv {
        limits: Limits {
            timeout_ms: 200,
            ..Default::default()
        },
        ..Default::default()
    };
    let t = std::time::Instant::now();
    let (_, errs) = run_with(Role::Pre, Lang::Js, "while (true) {}", &env);
    assert_eq!(errs[0].kind, ErrorKind::Timeout, "{errs:?}");
    assert!(t.elapsed().as_secs() < 3);
}

#[test]
fn sandbox_memory_bomb() {
    let env = RunEnv {
        limits: Limits {
            memory_mb: 16,
            timeout_ms: 5000,
            ..Default::default()
        },
        ..Default::default()
    };
    let (_, errs) = run_with(
        Role::Pre,
        Lang::Js,
        "const a = []; while (true) { a.push(new Array(100000).fill('x'.repeat(100))); }",
        &env,
    );
    assert_eq!(errs[0].kind, ErrorKind::Memory, "{errs:?}");
}

#[test]
fn sandbox_deep_recursion_is_an_error_not_a_crash() {
    let (_, errs) = run_with(
        Role::Pre,
        Lang::Js,
        "function f(n) { return f(n + 1) + 1; } f(0);",
        &RunEnv::default(),
    );
    assert_eq!(errs.len(), 1);
    assert_eq!(errs[0].kind, ErrorKind::Throw);
}

#[test]
fn sandbox_has_no_fs_process_or_network() {
    let (st, errs) = run(r#"
        for (const n of ["fetch","XMLHttpRequest","process","require","std","os","Deno","Bun","WebSocket","setTimeout"])
          fw.test("no " + n, () => fw.expect(typeof (globalThis as any)[n]).toBe("undefined"));
    "#);
    assert!(errs.is_empty(), "{errs:?}");
    for t in &st.tests {
        assert!(t.passed, "{} leaked: {:?}", t.name, t.message);
    }
}

struct Host;
impl ScriptHost for Host {
    fn send_request(
        &self,
        target: &Value,
        vars: &BTreeMap<String, String>,
    ) -> Result<Value, String> {
        Ok(
            json!({"status":200,"statusText":"OK","headers":[],"text":format!("{{\"t\":{target},\"v\":{vars:?}}}"),"time":1,"size":1,"timings":{}}),
        )
    }
}

#[test]
fn send_request_goes_through_host() {
    let env = RunEnv {
        host: Some(Arc::new(Host)),
        ..Default::default()
    };
    let (st, errs) = run_with(
        Role::Post,
        Lang::Ts,
        r#"
        const r = await fw.sendRequest("payments/get.flux.md", { vars: { id: "p1" } });
        fw.vars.set("got", String(r.status) + r.json().t);
    "#,
        &env,
    );
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(
        st.vars_changed["got"].as_ref().unwrap().value,
        "200payments/get.flux.md"
    );
    let (_, errs) = run_with(
        Role::Post,
        Lang::Ts,
        "await fw.sendRequest('x')",
        &RunEnv::default(),
    );
    assert_eq!(errs.len(), 1);
}

#[test]
fn utilities_url_encoding_jsonpath() {
    let (st, errs) = run(r#"
        const u = new URL("https://a.example:8443/p/q?x=1&y=2#h");
        fw.test("url", () => { fw.expect(u.hostname).toBe("a.example"); fw.expect(u.port).toBe("8443"); fw.expect(u.searchParams.get("y")).toBe("2"); });
        fw.test("b64", () => { fw.expect(btoa("hello")).toBe("aGVsbG8="); fw.expect(atob("aGVsbG8=")).toBe("hello"); });
        fw.test("te", () => fw.expect(new TextDecoder().decode(new TextEncoder().encode("héllo"))).toBe("héllo"));
        fw.test("jp", () => fw.expect(fw.jsonpath({a:[{b:1},{b:2}]}, "$.a[*].b")).toEqual([1,2]));
        fw.test("sp", () => fw.expect(new URLSearchParams({a:"1 2"}).toString()).toBe("a=1%202"));
    "#);
    assert!(errs.is_empty(), "{errs:?}");
    for t in &st.tests {
        assert!(t.passed, "{}: {:?}", t.name, t.message);
    }
}

#[test]
fn pm_shim_runs_postman_snippets() {
    let (st, errs) = run_with(
        Role::Test,
        Lang::Js,
        r#"
        pm.test("Status code is 201", function () { pm.response.to.have.status(201); });
        pm.test("Body has id", function () {
          var jsonData = pm.response.json();
          pm.expect(jsonData.id).to.eql("p_1");
          pm.expect(jsonData).to.have.property("n").that.is.a("number");
          pm.expect(jsonData.n).to.be.above(2);
          pm.expect(pm.response.responseTime).to.be.below(1000);
          pm.expect(pm.response.headers.get("Location")).to.include("/p/");
        });
        pm.test("negation", function () { pm.expect(1).to.not.equal(2); pm.expect(true).to.be.true; });
        pm.environment.set("paymentId", pm.response.json().id);
        pm.test("fails", function () { pm.expect(1).to.equal(2); });
    "#,
        &RunEnv::default(),
    );
    assert!(errs.is_empty(), "{errs:?}");
    eprintln!("{:?}", st.tests);
    let passed: Vec<_> = st
        .tests
        .iter()
        .map(|t| (t.name.as_str(), t.passed))
        .collect();
    assert_eq!(
        passed,
        vec![
            ("Status code is 201", true),
            ("Body has id", true),
            ("negation", true),
            ("fails", false)
        ]
    );
    assert_eq!(st.vars_changed["paymentId"].as_ref().unwrap().value, "p_1");
}

#[test]
fn pm_unsupported_members_throw_clearly() {
    let (_, errs) = run_with(
        Role::Post,
        Lang::Js,
        "pm.visualizer.set('<p>x</p>', {});",
        &RunEnv::default(),
    );
    assert_eq!(errs[0].kind, ErrorKind::Unsupported);
    assert!(
        errs[0]
            .message
            .contains("Unsupported in Fluxwire: pm.visualizer.set"),
        "{}",
        errs[0].message
    );
    let (_, errs) = run_with(
        Role::Post,
        Lang::Js,
        "const m = require('moment');",
        &RunEnv::default(),
    );
    assert_eq!(errs[0].kind, ErrorKind::Unsupported);
}

#[test]
fn blocks_run_in_order_and_share_nothing_but_state() {
    let blocks = [
        block(
            Role::Test,
            Lang::Ts,
            "fw.vars.set('a', '1'); fw.test('one', () => {});",
        ),
        block(
            Role::Test,
            Lang::Ts,
            "fw.test('two', () => fw.expect(fw.vars.get('a')).toBe('1'));",
        ),
    ];
    let (st, errs) = run_blocks(&blocks, state(), &RunEnv::default());
    assert!(errs.is_empty());
    assert_eq!(
        st.tests.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert!(st.tests[1].passed);
}
