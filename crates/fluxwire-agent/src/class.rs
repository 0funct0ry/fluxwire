//! Tool classes (ADR-009): `--allow` selects which classes a server exposes.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolClass {
    Read,
    Write,
    Execute,
    /// Only in `dev-mcp` GUI builds, through a `UiBridge`; never selectable by `--allow`.
    Ui,
}

impl ToolClass {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolClass::Read => "read",
            ToolClass::Write => "write",
            ToolClass::Execute => "execute",
            ToolClass::Ui => "ui",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowSet(BTreeSet<ToolClass>);

impl Default for AllowSet {
    /// `read,execute` (ADR-009).
    fn default() -> Self {
        AllowSet([ToolClass::Read, ToolClass::Execute].into())
    }
}

impl AllowSet {
    /// Parses names like `read,execute`. `ui` is rejected: it exists only through a bridge.
    pub fn parse<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        let mut set = BTreeSet::new();
        for raw in names {
            for n in raw.as_ref().split(',') {
                match n.trim() {
                    "" => continue,
                    "read" => {
                        set.insert(ToolClass::Read);
                    }
                    "write" => {
                        set.insert(ToolClass::Write);
                    }
                    "execute" => {
                        set.insert(ToolClass::Execute);
                    }
                    other => {
                        return Err(format!(
                            "unknown tool class `{other}` (expected read, write, execute)"
                        ));
                    }
                }
            }
        }
        Ok(AllowSet(set))
    }

    /// Lets the `ui` class through; called by hosts that supply a `UiBridge`.
    pub fn with_ui(mut self) -> Self {
        self.0.insert(ToolClass::Ui);
        self
    }

    pub fn contains(&self, c: ToolClass) -> bool {
        self.0.contains(&c)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.0.iter().map(|c| c.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_read_and_execute() {
        let a = AllowSet::default();
        assert!(a.contains(ToolClass::Read) && a.contains(ToolClass::Execute));
        assert!(!a.contains(ToolClass::Write));
    }

    #[test]
    fn parses_lists_and_rejects_unknown_and_ui() {
        let a = AllowSet::parse(&["read,write"]).unwrap();
        assert_eq!(a.names(), ["read", "write"]);
        assert!(AllowSet::parse(&["read", "bogus"]).is_err());
        assert!(AllowSet::parse(&["ui"]).is_err());
    }
}
