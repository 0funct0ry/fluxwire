//! Reporters: pretty (tree), json, junit and tap. Everything printed is redacted.

use clap::ValueEnum;
use fluxwire_runner::{
    FolderRunResult, RequestRun, RequestStatus, error_text, to_json, to_junit, to_tap,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Reporter {
    Pretty,
    Json,
    Junit,
    Tap,
}

#[derive(Clone, Copy)]
pub struct Style {
    pub color: bool,
    pub verbose: bool,
}

impl Style {
    fn paint(&self, code: &str, s: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
}

/// Text of `reporter` for one finished run.
pub fn render(reporter: Reporter, run: &FolderRunResult, style: Style) -> String {
    match reporter {
        Reporter::Pretty => pretty(run, style),
        Reporter::Json => to_json(run),
        Reporter::Junit => to_junit(run),
        Reporter::Tap => to_tap(run),
    }
}

fn ms(v: u64) -> String {
    if v >= 1000 {
        format!("{:.2}s", v as f64 / 1000.0)
    } else {
        format!("{v}ms")
    }
}

fn http_status(r: &RequestRun) -> Option<u16> {
    r.result
        .outcome
        .as_ref()?
        .exchange
        .as_ref()
        .map(|e| e.status)
}

fn request_lines(r: &RequestRun, st: Style, out: &mut String) {
    let res = &r.result;
    let (mark, code) = match r.status() {
        RequestStatus::Passed => ("✓", "32"),
        RequestStatus::Failed => ("✗", "31"),
        RequestStatus::Skipped => ("-", "33"),
    };
    let status = http_status(r).map(|s| format!(" {s}")).unwrap_or_default();
    let iter = if r.iteration > 0 {
        format!(" [#{}]", r.iteration + 1)
    } else {
        String::new()
    };
    out.push_str(&format!(
        "  {} {} {}{}{}  {}\n",
        st.paint(code, mark),
        r.request.method,
        r.request.name,
        iter,
        status,
        st.paint("2", &ms(r.duration_ms)),
    ));
    if let Some(reason) = &res.skipped {
        out.push_str(&format!("      skipped: {reason}\n"));
    }
    if let Some(e) = error_text(r) {
        for l in e.lines() {
            out.push_str(&format!("      {}\n", st.paint("31", l)));
        }
    }
    for t in &res.tests {
        if t.passed && !st.verbose {
            continue;
        }
        let (m, c) = if t.passed {
            ("✓", "32")
        } else {
            ("✗", "31")
        };
        out.push_str(&format!(
            "      {} {}\n",
            st.paint(c, m),
            res.redact(&t.name)
        ));
        if !t.passed {
            if let Some(msg) = &t.message {
                out.push_str(&format!("        {}\n", res.redact(msg)));
            }
            if let (Some(e), Some(a)) = (&t.expected, &t.actual) {
                out.push_str(&format!(
                    "        expected {}\n        actual   {}\n",
                    res.redact(e),
                    res.redact(a)
                ));
            }
        }
    }
    if st.verbose {
        for l in &res.console {
            out.push_str(&format!("      console: {}\n", res.redact(&l.text)));
        }
    }
}

pub fn pretty(run: &FolderRunResult, st: Style) -> String {
    let mut out = String::new();
    let title = if run.folder.is_empty() {
        "collection"
    } else {
        &run.folder
    };
    out.push_str(&format!("{}\n", st.paint("1", title)));
    let mut folder: Option<&str> = None;
    for r in &run.requests {
        if folder != Some(r.request.folder.as_str()) {
            folder = Some(&r.request.folder);
            if !r.request.folder.is_empty() {
                out.push_str(&format!("{}\n", st.paint("36", &r.request.folder)));
            }
        }
        request_lines(r, st, &mut out);
    }
    let s = run.summary();
    out.push_str(&format!(
        "\n{} requests: {} passed, {} failed, {} skipped; {} tests, {} failed; {}\n",
        s.requests,
        s.passed,
        s.failed,
        s.skipped,
        s.tests,
        s.tests_failed,
        ms(run.total_ms)
    ));
    if run.bailed {
        out.push_str("stopped at first failure (--bail)\n");
    }
    if run.timed_out {
        out.push_str("run timed out\n");
    }
    if let Some(e) = &run.error {
        out.push_str(&format!("error: {e}\n"));
    }
    out
}
