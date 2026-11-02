//! A small MCP client that runs a scenario file against a live `dev-mcp` app over its unix
//! socket (`fluxwire mcp scenario`). CI uses it so a smoke job needs no agent.

use crate::scenario::Scenario;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock, JsonObject};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The outcome of a remote run.
pub struct Outcome {
    pub report: Value,
    /// PNG bytes with their labels.
    pub images: Vec<(String, Vec<u8>)>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.report["passed"] == true
    }

    /// A short, human-readable summary with the failing step first.
    pub fn summary(&self) -> String {
        let mut out = format!(
            "{} {}\n",
            if self.passed() { "PASS" } else { "FAIL" },
            self.report["name"].as_str().unwrap_or("scenario")
        );
        for s in self.report["steps"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  {:>2} {:<14} {:<7} {} ms{}\n",
                s["n"],
                s["step"].as_str().unwrap_or("?"),
                s["status"].as_str().unwrap_or("?"),
                s["ms"],
                s["error"]
                    .as_str()
                    .map(|e| format!("  {e}"))
                    .unwrap_or_default()
            ));
        }
        if let Some(e) = self.report["failure"]["error"].as_str() {
            out.push_str(&format!("failure: {e}\n"));
        }
        out
    }
}

/// Parses `file`, runs it through `ui.run_scenario` on the app at `socket`.
#[cfg(unix)]
pub async fn run_file(file: &Path, socket: &Path, capture: bool) -> Result<Outcome, String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut scenario = serde_json::to_value(Scenario::parse(&src)?).map_err(|e| e.to_string())?;
    // A relative workspace in a scenario file is relative to the repository that holds it.
    if let Some(ws) = scenario["workspace"].as_str().map(PathBuf::from)
        && ws.is_relative()
        && let Some(repo) = file.canonicalize().ok().and_then(|f| {
            f.ancestors()
                .find(|a| a.join("fixtures").is_dir())
                .map(Path::to_path_buf)
        })
    {
        scenario["workspace"] = json!(repo.join(ws).to_string_lossy());
    }
    let stream = tokio::net::UnixStream::connect(socket)
        .await
        .map_err(|e| format!("cannot connect to {}: {e}", socket.display()))?;
    let client = ().serve(stream).await.map_err(|e| format!("MCP handshake failed: {e}"))?;
    let mut args = JsonObject::new();
    args.insert("scenario".into(), scenario);
    args.insert("capture".into(), json!(capture));
    let r = client
        .peer()
        .call_tool(CallToolRequestParams::new("ui.run_scenario").with_arguments(args))
        .await
        .map_err(|e| format!("ui.run_scenario failed: {e}"))?;
    if r.is_error == Some(true) {
        let text: Vec<String> = r
            .content
            .iter()
            .filter_map(|c| match c {
                ContentBlock::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect();
        return Err(text.join(" "));
    }
    let mut images = vec![];
    let mut label = String::from("image");
    for c in &r.content {
        match c {
            ContentBlock::Text(t) => label = t.text.clone(),
            ContentBlock::Image(i) => {
                if let Ok(b) = crate::image::from_base64(&i.data) {
                    images.push((std::mem::take(&mut label), b));
                }
            }
            _ => {}
        }
    }
    let _ = client.cancel().await;
    Ok(Outcome {
        report: r.structured_content.unwrap_or(Value::Null),
        images,
    })
}

#[cfg(not(unix))]
pub async fn run_file(_: &Path, _: &Path, _: bool) -> Result<Outcome, String> {
    Err("scenario runs need the unix socket transport".into())
}
