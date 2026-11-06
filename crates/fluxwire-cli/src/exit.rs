//! Process exit codes (SPEC §18). Precedence when several apply: 3 (script threw) over 2
//! (network failure) over 1 (test or request failure).

use fluxwire_runner::{FolderRunResult, RunError, RunResult};

pub const TESTS_FAILED: u8 = 1;
pub const NETWORK: u8 = 2;
pub const SCRIPT: u8 = 3;
/// OpenAPI conflicts block a pull or push.
pub const CONFLICTS: u8 = 4;
pub const USAGE: u8 = 64;
pub const PARSE: u8 = 65;
pub const UNSUPPORTED: u8 = 66;

/// Exit code of one request's result.
pub fn result_code(r: &RunResult) -> u8 {
    if !r.script_errors.is_empty() || matches!(r.error, Some(RunError::Script(_))) {
        return SCRIPT;
    }
    match &r.error {
        Some(e) if e.is_cancelled() => 0,
        Some(RunError::Exec(_)) => NETWORK,
        Some(_) => TESTS_FAILED,
        None if r.tests.iter().any(|t| !t.passed) => TESTS_FAILED,
        None => 0,
    }
}

/// Exit code of a whole run: the highest-precedence code among its requests.
pub fn run_code(run: &FolderRunResult) -> u8 {
    let mut code = 0;
    for r in &run.requests {
        code = merge(code, result_code(&r.result));
    }
    if code == 0 && run.error.is_some() {
        code = TESTS_FAILED;
    }
    code
}

/// Combine two codes by precedence (usage/parse codes win over run codes).
pub fn merge(a: u8, b: u8) -> u8 {
    fn rank(c: u8) -> u8 {
        match c {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            _ => 4,
        }
    }
    if rank(b) > rank(a) { b } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence() {
        assert_eq!(merge(1, 2), 2);
        assert_eq!(merge(3, 2), 3);
        assert_eq!(merge(0, 1), 1);
        assert_eq!(merge(3, 65), 65);
        assert_eq!(merge(0, 0), 0);
    }
}
