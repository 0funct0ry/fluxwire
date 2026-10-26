//! The QuickJS driver: runs a list of blocks as ES modules inside one sandboxed runtime.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use rquickjs::loader::{Loader, Resolver};
use rquickjs::{
    CatchResultExt, Context, Ctx, Error, Function, Module, Persistent, Promise, Runtime,
    Value as JsValue,
};
use serde_json::{Value, json};

use crate::native::dispatch;
use crate::transpile::{Transpiled, transpile};
use crate::{ErrorKind, Lang, RunEnv, ScriptBlock, ScriptError, ScriptState};

const PRELUDE: &str = include_str!("prelude.js");
const PM: &str = include_str!("pm.js");

struct ModInfo {
    map: Transpiled,
    file: String,
    first_line: u32,
}

type Maps = Rc<RefCell<HashMap<String, ModInfo>>>;

/// Resolves imports lexically; only `lib/…` under the collection is allowed.
struct LibResolver {
    enabled: bool,
}

fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for c in Path::new(path).components() {
        match c {
            Component::Normal(s) => parts.push(s.to_str()?),
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

impl Resolver for LibResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attrs: Option<rquickjs::loader::ImportAttributes<'js>>,
    ) -> rquickjs::Result<String> {
        let deny = |why: &str| Error::new_resolving_message(base, name, why.to_string());
        if !self.enabled {
            return Err(deny("imports are disabled (no collection)"));
        }
        let joined = if name.starts_with("./") || name.starts_with("../") {
            match base.strip_prefix("lib/") {
                Some(rest) => {
                    let dir = Path::new(rest).parent().unwrap_or(Path::new(""));
                    format!("lib/{}", dir.join(name).display())
                }
                None => name.to_string(),
            }
        } else {
            name.to_string()
        };
        match normalize(&joined) {
            Some(p) if p.starts_with("lib/") => Ok(p),
            _ => Err(deny(
                "imports are only allowed from the collection's lib/ directory",
            )),
        }
    }
}

struct LibLoader {
    lib_dir: Option<PathBuf>,
    maps: Maps,
}

impl Loader for LibLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attrs: Option<rquickjs::loader::ImportAttributes<'js>>,
    ) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        let fail = |why: String| Error::new_loading_message(name, why);
        let lib = self
            .lib_dir
            .as_ref()
            .ok_or_else(|| fail("imports are disabled".into()))?;
        let root = lib.parent().unwrap_or(lib);
        let full = root.join(name);
        let canon = full
            .canonicalize()
            .map_err(|e| fail(format!("{}: {e}", name)))?;
        let canon_lib = lib.canonicalize().map_err(|e| fail(e.to_string()))?;
        if !canon.starts_with(&canon_lib) {
            return Err(fail("path escapes lib/".into()));
        }
        let source = std::fs::read_to_string(&canon).map_err(|e| fail(e.to_string()))?;
        let lang = if name.ends_with(".ts") {
            Lang::Ts
        } else {
            Lang::Js
        };
        let t = transpile(&source, lang, name, 1).map_err(|e| fail(e.message))?;
        let js = t.js.clone();
        self.maps.borrow_mut().insert(
            name.to_string(),
            ModInfo {
                map: t,
                file: name.to_string(),
                first_line: 1,
            },
        );
        Module::declare(ctx.clone(), name, js)
    }
}

fn exception_parts(ctx: &Ctx<'_>, caught: JsValue<'_>) -> (String, String) {
    let _ = ctx;
    if let Some(obj) = caught.as_object() {
        let message: Option<String> = obj.get("message").ok();
        let stack: Option<String> = obj.get("stack").ok();
        let name: Option<String> = obj.get("name").ok();
        let msg = message.unwrap_or_default();
        let msg = match name {
            Some(n) if n != "Error" && !msg.is_empty() && !msg.starts_with(&n) => {
                format!("{n}: {msg}")
            }
            _ => msg,
        };
        return (msg, stack.unwrap_or_default());
    }
    if caught.is_undefined() || caught.is_null() {
        // An exhausted heap leaves no readable exception behind.
        return ("out of memory".into(), String::new());
    }
    let text = caught
        .as_string()
        .and_then(|s| s.to_string().ok())
        .unwrap_or_else(|| "uncaught exception".into());
    (text, String::new())
}

/// Finds the first stack frame in a known module and maps it to the `.flux.md` file.
fn locate(stack: &str, maps: &Maps, fallback: (&str, u32)) -> (String, u32, u32) {
    let maps = maps.borrow();
    for frame in stack.lines() {
        let Some(open) = frame.rfind('(') else {
            continue;
        };
        let inner = frame[open + 1..].trim_end_matches(')');
        let mut it = inner.rsplitn(3, ':');
        let (col, line, name) = match (it.next(), it.next(), it.next()) {
            (Some(c), Some(l), Some(n)) => (c, l, n),
            _ => continue,
        };
        if let (Some(info), Ok(l), Ok(c)) =
            (maps.get(name), line.parse::<u32>(), col.parse::<u32>())
        {
            let src = info.map.source_line(l);
            return (info.file.clone(), info.first_line + src, c);
        }
    }
    (fallback.0.to_string(), fallback.1, 0)
}

fn classify(message: &str, timed_out: bool) -> ErrorKind {
    if timed_out || message == "interrupted" {
        ErrorKind::Timeout
    } else if message.contains("out of memory") {
        ErrorKind::Memory
    } else if message.contains("Unsupported in Fluxwire") {
        ErrorKind::Unsupported
    } else if message.contains("lib/")
        && (message.contains("resolv") || message.contains("loading"))
    {
        ErrorKind::Import
    } else {
        ErrorKind::Throw
    }
}

fn js_err(
    ctx: &Ctx<'_>,
    e: Error,
    block: &ScriptBlock,
    maps: &Maps,
    deadline: &Cell<Instant>,
) -> ScriptError {
    let timed_out = Instant::now() > deadline.get();
    let (message, stack) = if matches!(e, Error::Exception) {
        let caught = ctx.catch();
        exception_parts(ctx, caught)
    } else {
        (e.to_string(), String::new())
    };
    let kind = classify(&message, timed_out);
    let (file, line, col) = locate(&stack, maps, (&block.file, block.first_line));
    let message = if kind == ErrorKind::Timeout {
        "script exceeded its time limit".to_string()
    } else if kind == ErrorKind::Memory {
        "script exceeded its memory limit".to_string()
    } else {
        message
    };
    ScriptError {
        kind,
        message,
        file,
        line,
        col,
        stack,
    }
}

fn init_json(state: &ScriptState, block: &ScriptBlock) -> String {
    json!({
        "role": block.role.name(),
        "request": state.request,
        "response": state.response,
        "vars": state.vars.iter().map(|(k, v)| (k.clone(), Value::String(v.value.clone()))).collect::<serde_json::Map<_, _>>(),
        "env": state.env_vars,
        "envName": state.env_name,
        "globals": state.globals,
    })
    .to_string()
}

/// Runs `blocks` in order inside one sandboxed runtime. Stops at the first error.
/// Returns the final state and the errors (empty on success).
pub fn run_blocks(
    blocks: &[ScriptBlock],
    state: ScriptState,
    env: &RunEnv,
) -> (ScriptState, Vec<ScriptError>) {
    if blocks.is_empty() {
        return (state, Vec::new());
    }
    let state = Rc::new(RefCell::new(state));
    let mut errors = Vec::new();
    let maps: Maps = Rc::new(RefCell::new(HashMap::new()));
    let deadline = Rc::new(Cell::new(
        Instant::now() + Duration::from_millis(env.limits.timeout_ms),
    ));

    let fail = |msg: String, b: &ScriptBlock| ScriptError {
        kind: ErrorKind::Throw,
        message: msg,
        file: b.file.clone(),
        line: b.first_line,
        col: 0,
        stack: String::new(),
    };

    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            errors.push(fail(
                format!("could not start script runtime: {e}"),
                &blocks[0],
            ));
            return (
                Rc::try_unwrap(state)
                    .map(RefCell::into_inner)
                    .unwrap_or_default(),
                errors,
            );
        }
    };
    rt.set_memory_limit(env.limits.memory_mb * 1024 * 1024);
    rt.set_max_stack_size(env.limits.stack_kb * 1024);
    {
        let deadline = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > deadline.get())));
    }
    rt.set_loader(
        LibResolver {
            enabled: env.lib_dir.is_some(),
        },
        LibLoader {
            lib_dir: env.lib_dir.clone(),
            maps: maps.clone(),
        },
    );
    let ctx = match Context::full(&rt) {
        Ok(c) => c,
        Err(e) => {
            errors.push(fail(
                format!("could not start script context: {e}"),
                &blocks[0],
            ));
            return (
                Rc::try_unwrap(state)
                    .map(RefCell::into_inner)
                    .unwrap_or_default(),
                errors,
            );
        }
    };

    let needs_pm = blocks
        .iter()
        .any(|b| b.source.contains("pm.") || b.source.contains("require("));
    let counters: RefCell<HashMap<&'static str, usize>> = RefCell::new(HashMap::new());

    for block in blocks {
        deadline.set(Instant::now() + Duration::from_millis(env.limits.timeout_ms));
        let n = {
            let mut c = counters.borrow_mut();
            let e = c.entry(block.role.name()).or_insert(0);
            *e += 1;
            *e
        };
        let name = format!("{}#{}-{}", block.file, block.role.name(), n);
        let transpiled = match transpile(&block.source, block.lang, &block.file, block.first_line) {
            Ok(t) => t,
            Err(e) => {
                errors.push(e);
                break;
            }
        };
        let js = transpiled.js.clone();
        maps.borrow_mut().insert(
            name.clone(),
            ModInfo {
                map: transpiled,
                file: block.file.clone(),
                first_line: block.first_line,
            },
        );

        let init = init_json(&state.borrow(), block);
        let setup: Result<Persistent<Promise<'static>>, ScriptError> = ctx.with(|ctx| {
            let st = state.clone();
            let env2 = env.clone();
            let native = Function::new(ctx.clone(), move |op: String, arg: String| -> String {
                let parsed: Value = serde_json::from_str(&arg).unwrap_or(Value::Null);
                match dispatch(&st, &env2, &op, &parsed) {
                    Ok(v) => json!({ "ok": v }).to_string(),
                    Err(e) => json!({ "err": e }).to_string(),
                }
            })
            .map_err(|e| js_err(&ctx, e, block, &maps, &deadline))?;
            let globals = ctx.globals();
            globals
                .set("__native", native)
                .and_then(|_| globals.set("__init", init))
                .map_err(|e| js_err(&ctx, e, block, &maps, &deadline))?;
            ctx.eval::<(), _>(PRELUDE)
                .map_err(|e| js_err(&ctx, e, block, &maps, &deadline))?;
            if needs_pm {
                ctx.eval::<(), _>(PM)
                    .map_err(|e| js_err(&ctx, e, block, &maps, &deadline))?;
            }
            let (_module, promise) = Module::declare(ctx.clone(), name.clone(), js)
                .and_then(|m| m.eval())
                .catch(&ctx)
                .map_err(|e| from_caught(e, block, &maps, &deadline))?;
            Ok(Persistent::save(&ctx, promise))
        });
        let result: Result<(), ScriptError> = setup.and_then(|persisted| {
            // Jobs must run outside `Context::with` (the runtime is locked inside it).
            loop {
                match rt.execute_pending_job() {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(job) => {
                        return Err(job.0.with(|c| {
                            let caught = c.catch();
                            let (message, stack) = exception_parts(&c, caught);
                            let timed_out = Instant::now() > deadline.get();
                            let kind = classify(&message, timed_out);
                            let (file, line, col) =
                                locate(&stack, &maps, (&block.file, block.first_line));
                            ScriptError {
                                kind,
                                message,
                                file,
                                line,
                                col,
                                stack,
                            }
                        }));
                    }
                }
            }
            ctx.with(|ctx| {
                let promise = persisted
                    .restore(&ctx)
                    .map_err(|e| js_err(&ctx, e, block, &maps, &deadline))?;
                match promise.result::<JsValue>() {
                    Some(Err(e)) => Err(js_err(&ctx, e, block, &maps, &deadline)),
                    _ => Ok(()),
                }
            })
        });
        if let Err(e) = result {
            errors.push(e);
            break;
        }
        if state.borrow().skip.is_some() && block.role == crate::Role::Pre {
            break;
        }
    }
    drop(ctx);
    drop(rt);
    let state = Rc::try_unwrap(state)
        .map(RefCell::into_inner)
        .unwrap_or_else(|rc| rc.borrow().clone());
    (state, errors)
}

fn from_caught(
    e: rquickjs::CaughtError<'_>,
    block: &ScriptBlock,
    maps: &Maps,
    deadline: &Cell<Instant>,
) -> ScriptError {
    let (message, stack) = match &e {
        rquickjs::CaughtError::Exception(ex) => (
            ex.message().unwrap_or_default(),
            ex.stack().unwrap_or_default(),
        ),
        rquickjs::CaughtError::Value(v) => (format!("{v:?}"), String::new()),
        rquickjs::CaughtError::Error(err) => (err.to_string(), String::new()),
    };
    let timed_out = Instant::now() > deadline.get();
    let kind = classify(&message, timed_out);
    let (file, line, col) = locate(&stack, maps, (&block.file, block.first_line));
    let message = match kind {
        ErrorKind::Timeout => "script exceeded its time limit".to_string(),
        ErrorKind::Memory => "script exceeded its memory limit".to_string(),
        _ => message,
    };
    ScriptError {
        kind,
        message,
        file,
        line,
        col,
        stack,
    }
}
