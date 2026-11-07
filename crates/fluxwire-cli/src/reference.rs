//! Hidden `fluxwire reference <cli|protocols|exit-codes>`: machine-readable reference data that
//! the docs site generators (`site/scripts/`) consume, so the published pages cannot drift.

use crate::{Cli, exit};
use clap::{Args, CommandFactory, Subcommand};
use fluxwire_protocol::{
    CORE_OTHER_ROLES, ProtocolRegistry, RESERVED_REQUEST_LANGUAGES, RESERVED_ROLES,
    SCRIPT_LANGUAGES, SCRIPT_ROLES,
};
use serde_json::json;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ReferenceArgs {
    #[command(subcommand)]
    pub what: What,
}

#[derive(Subcommand, Debug)]
pub enum What {
    /// The clap command tree as Markdown.
    Cli,
    /// Protocol roles, request languages and reserved names as JSON.
    Protocols,
    /// The exit-code table as JSON.
    ExitCodes,
}

pub fn run(a: ReferenceArgs) -> ExitCode {
    match a.what {
        What::Cli => print!("{}", cli_markdown()),
        What::Protocols => println!("{}", serde_json::to_string_pretty(&protocols()).unwrap()),
        What::ExitCodes => println!("{}", serde_json::to_string_pretty(&exit_codes()).unwrap()),
    }
    ExitCode::SUCCESS
}

pub fn cli_markdown() -> String {
    let mut out = String::new();
    let cmd = Cli::command();
    walk(&cmd, &mut vec!["fluxwire".to_string()], &mut out);
    out
}

fn walk(cmd: &clap::Command, path: &mut Vec<String>, out: &mut String) {
    for sub in cmd.get_subcommands().filter(|s| !s.is_hide_set()) {
        path.push(sub.get_name().to_string());
        let has_subs = sub.get_subcommands().any(|s| !s.is_hide_set());
        if !has_subs {
            let mut c = sub.clone().bin_name(path.join(" "));
            out.push_str(&format!("## `{}`\n\n", path.join(" ")));
            if let Some(about) = sub.get_about() {
                out.push_str(&format!("{about}\n\n"));
            }
            let help = c.render_long_help().to_string();
            // Drop the repeated about paragraph; the heading and summary already carry it.
            let help = help
                .split_once("\n\n")
                .map_or(help.as_str(), |(_, rest)| rest);
            out.push_str("```text\n");
            out.push_str(help.trim_end());
            out.push_str("\n```\n\n");
        } else {
            walk(sub, path, out);
        }
        path.pop();
    }
}

pub fn protocols() -> serde_json::Value {
    let reg = ProtocolRegistry::builtin();
    let ps: Vec<_> = reg
        .protocols()
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "feature": p.feature,
                "request_languages": p.request_languages,
                "interaction": format!("{:?}", p.interaction),
                "roles": p.roles.iter().map(|r| json!({
                    "role": r.role,
                    "language": r.language,
                    "content": format!("{:?}", r.content),
                    "cardinality": format!("{:?}", r.cardinality),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "protocols": ps,
        "script_roles": SCRIPT_ROLES,
        "script_languages": SCRIPT_LANGUAGES,
        "core_roles": CORE_OTHER_ROLES,
        "reserved_roles": RESERVED_ROLES,
        "reserved_request_languages": RESERVED_REQUEST_LANGUAGES,
    })
}

pub fn exit_codes() -> serde_json::Value {
    json!([
        {"code": 0, "name": "OK", "meaning": "Everything passed."},
        {"code": exit::TESTS_FAILED, "name": "TESTS_FAILED", "meaning": "A test, expectation or request failed."},
        {"code": exit::NETWORK, "name": "NETWORK", "meaning": "A network failure prevented a request from completing."},
        {"code": exit::SCRIPT, "name": "SCRIPT", "meaning": "A script threw."},
        {"code": exit::CONFLICTS, "name": "CONFLICTS", "meaning": "OpenAPI conflicts block a pull or push."},
        {"code": exit::USAGE, "name": "USAGE", "meaning": "Invalid command-line usage."},
        {"code": exit::PARSE, "name": "PARSE", "meaning": "A file could not be parsed."},
        {"code": exit::UNSUPPORTED, "name": "UNSUPPORTED", "meaning": "A file requires a feature this version does not support."},
    ])
}
