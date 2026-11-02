//! Resolves serve options: flags win over `fluxwire.toml`, which wins over user settings.

use crate::AllowSet;
use fluxwire_format::McpSettings;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Stdio,
    Unix,
    Both,
}

impl Transport {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "stdio" => Ok(Transport::Stdio),
            "unix" if cfg!(unix) => Ok(Transport::Unix),
            "both" if cfg!(unix) => Ok(Transport::Both),
            "unix" | "both" => Err("the unix transport is not available on this platform".into()),
            other => Err(format!(
                "unknown transport `{other}` (expected stdio, unix, both)"
            )),
        }
    }

    pub fn stdio(self) -> bool {
        matches!(self, Transport::Stdio | Transport::Both)
    }

    pub fn unix(self) -> bool {
        matches!(self, Transport::Unix | Transport::Both)
    }

    pub fn names(self) -> Vec<&'static str> {
        let mut v = vec![];
        if self.stdio() {
            v.push("stdio");
        }
        if self.unix() {
            v.push("unix");
        }
        v
    }
}

#[derive(Debug, Clone)]
pub struct ServeConfig {
    pub transport: Transport,
    /// `None` means the default location, chosen at bind time.
    pub socket: Option<PathBuf>,
    pub allow: AllowSet,
    pub roots: Vec<PathBuf>,
}

impl ServeConfig {
    /// `flags` are the command line, `manifest` the `[mcp]` of fluxwire.toml, `user` of settings.toml.
    pub fn resolve(
        flags: McpSettings,
        manifest: McpSettings,
        user: McpSettings,
    ) -> Result<Self, String> {
        let m = flags.over(manifest).over(user);
        let transport = match m.transport {
            Some(t) => Transport::parse(&t)?,
            None => Transport::Stdio,
        };
        let allow = match m.allow {
            Some(a) => AllowSet::parse(&a)?,
            None => AllowSet::default(),
        };
        Ok(ServeConfig {
            transport,
            socket: m.socket,
            allow,
            roots: m.roots.unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> McpSettings {
        McpSettings {
            transport: Some(s.into()),
            ..Default::default()
        }
    }

    #[test]
    fn defaults() {
        let c = ServeConfig::resolve(Default::default(), Default::default(), Default::default())
            .unwrap();
        assert_eq!(c.transport, Transport::Stdio);
        assert_eq!(c.allow, AllowSet::default());
        assert!(c.socket.is_none() && c.roots.is_empty());
    }

    #[test]
    fn flags_beat_manifest_beat_user() {
        let c = ServeConfig::resolve(t("stdio"), t("both"), t("unix")).unwrap();
        assert_eq!(c.transport, Transport::Stdio);
        let c = ServeConfig::resolve(Default::default(), t("both"), t("stdio")).unwrap();
        assert_eq!(c.transport, Transport::Both);
    }

    #[test]
    fn bad_values_are_errors() {
        assert!(ServeConfig::resolve(t("tcp"), Default::default(), Default::default()).is_err());
        let bad = McpSettings {
            allow: Some(vec!["root".into()]),
            ..Default::default()
        };
        assert!(ServeConfig::resolve(bad, Default::default(), Default::default()).is_err());
    }
}
