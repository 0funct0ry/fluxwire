//! Default layouts for new files (SPEC §7.10).

pub fn new_request(name: &str, description: &str, method: &str, url: &str) -> String {
    let mut s = format!(
        "---\nname: {}\n---\n\n# {name}\n\n",
        crate::encode_scalar(name)
    );
    if !description.trim().is_empty() {
        s.push_str(description.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("```http\n{method} {url}\n```\n"));
    s
}

pub fn new_websocket(name: &str, description: &str, url: &str) -> String {
    let mut s = format!(
        "---\nname: {}\n---\n\n# {name}\n\n",
        crate::encode_scalar(name)
    );
    if !description.trim().is_empty() {
        s.push_str(description.trim());
        s.push_str("\n\n");
    }
    s.push_str(&format!("```ws\nGET {url}\n```\n"));
    s
}

pub fn new_folder(name: &str) -> String {
    format!(
        "---\nname: {}\n---\n\n# {name}\n",
        crate::encode_scalar(name)
    )
}

pub fn new_collection_file(name: &str) -> String {
    format!(
        "---\nname: {}\n---\n\n# {name}\n",
        crate::encode_scalar(name)
    )
}

pub fn new_collection_toml(name: &str) -> String {
    format!("[collection]\nname = {}\nformat = 1\n", toml_string(name))
}

fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub const GITIGNORE: &str = "*.local.env\nfluxwire.local.toml\n.fluxwire/cache/\n";
