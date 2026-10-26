//! Native operations behind the JS prelude: one `__native(op, json) -> json` entry point.

use std::cell::RefCell;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

use crate::{ConsoleLine, LogLevel, RunEnv, ScriptState, TestOutcome, VarEntry};

/// All JSONPath matches of `expr` in `value`.
pub fn query_json(value: &Value, expr: &str) -> Result<Vec<Value>, String> {
    let path = serde_json_path::JsonPath::parse(expr).map_err(|e| format!("bad JSONPath: {e}"))?;
    Ok(path.query(value).all().into_iter().cloned().collect())
}

/// Validates `value` against a JSON Schema; returns error messages (empty = valid).
pub fn validate_schema(value: &Value, schema: &Value) -> Result<Vec<String>, String> {
    let validator = jsonschema::validator_for(schema).map_err(|e| format!("bad schema: {e}"))?;
    Ok(validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect())
}

fn s(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn encode(bytes: &[u8], enc: &str) -> String {
    if enc == "base64" {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    } else {
        hex::encode(bytes)
    }
}

fn crypto(arg: &Value) -> Result<Value, String> {
    let alg = s(arg, "alg");
    let data = s(arg, "data");
    let enc = s(arg, "enc");
    let out = match alg.as_str() {
        "sha256" => Sha256::digest(data.as_bytes()).to_vec(),
        "sha512" => Sha512::digest(data.as_bytes()).to_vec(),
        "hmac-sha256" => {
            let mut m = Hmac::<Sha256>::new_from_slice(s(arg, "key").as_bytes())
                .map_err(|e| e.to_string())?;
            m.update(data.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        "hmac-sha512" => {
            let mut m = Hmac::<Sha512>::new_from_slice(s(arg, "key").as_bytes())
                .map_err(|e| e.to_string())?;
            m.update(data.as_bytes());
            m.finalize().into_bytes().to_vec()
        }
        other => {
            return Err(format!(
                "unsupported crypto algorithm '{other}' (sha256, sha512)"
            ));
        }
    };
    Ok(Value::String(encode(&out, &enc)))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn opt(v: &Value, key: &str) -> Option<String> {
    match v.get(key) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Dispatches one native op. Errors become JS exceptions in the prelude.
pub fn dispatch(
    state: &RefCell<ScriptState>,
    env: &RunEnv,
    op: &str,
    arg: &Value,
) -> Result<Value, String> {
    match op {
        "vars.set" => {
            let name = s(arg, "name");
            let entry = VarEntry {
                value: s(arg, "value"),
                secret: arg.get("secret").and_then(Value::as_bool).unwrap_or(false),
            };
            let mut st = state.borrow_mut();
            st.vars.insert(name.clone(), entry.clone());
            st.vars_changed.insert(name, Some(entry));
            Ok(Value::Null)
        }
        "vars.unset" => {
            let name = s(arg, "name");
            let mut st = state.borrow_mut();
            st.vars.remove(&name);
            st.vars_changed.insert(name, None);
            Ok(Value::Null)
        }
        "env.set" => {
            let (name, value) = (s(arg, "name"), s(arg, "value"));
            let mut st = state.borrow_mut();
            st.env_vars.insert(name.clone(), value.clone());
            st.env_set.insert(name, value);
            Ok(Value::Null)
        }
        "env.persist" => {
            state.borrow_mut().persist.push(s(arg, "name"));
            Ok(Value::Null)
        }
        "req.op" => {
            state.borrow_mut().request_ops.push(arg.clone());
            Ok(Value::Null)
        }
        "skip" => {
            state.borrow_mut().skip = Some(s(arg, "reason"));
            Ok(Value::Null)
        }
        "log" => {
            let level = match s(arg, "level").as_str() {
                "debug" => LogLevel::Debug,
                "info" => LogLevel::Info,
                "warn" => LogLevel::Warn,
                "error" => LogLevel::Error,
                _ => LogLevel::Log,
            };
            state.borrow_mut().console.push(ConsoleLine {
                level,
                text: s(arg, "text"),
                ts_ms: now_ms(),
            });
            Ok(Value::Null)
        }
        "test.record" => {
            state.borrow_mut().tests.push(TestOutcome {
                name: s(arg, "name"),
                passed: arg.get("passed").and_then(Value::as_bool).unwrap_or(false),
                message: opt(arg, "message"),
                expected: opt(arg, "expected"),
                actual: opt(arg, "actual"),
            });
            Ok(Value::Null)
        }
        "crypto" => crypto(arg),
        "uuid" => Ok(Value::String(uuid::Uuid::new_v4().to_string())),
        "jsonpath" => {
            let hits = query_json(arg.get("value").unwrap_or(&Value::Null), &s(arg, "expr"))?;
            Ok(Value::Array(hits))
        }
        "schema" => {
            let errors = validate_schema(
                arg.get("value").unwrap_or(&Value::Null),
                arg.get("schema").unwrap_or(&Value::Null),
            )?;
            Ok(json!({ "valid": errors.is_empty(), "errors": errors }))
        }
        "sleep" => {
            let ms = arg.as_u64().unwrap_or(0).min(60_000);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            Ok(Value::Null)
        }
        "send" => {
            let host = env
                .host
                .as_ref()
                .ok_or("fw.sendRequest is not available in this context")?;
            let vars = arg
                .get("vars")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                v.as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| v.to_string()),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            host.send_request(arg.get("target").unwrap_or(&Value::Null), &vars)
        }
        other => Err(format!("unknown native op '{other}'")),
    }
}
