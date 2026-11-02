//! stdio and unix-socket transports. No TCP (ADR-009).

use crate::FluxwireServer;
use rmcp::ServiceExt;
use std::path::{Path, PathBuf};

/// Serves one client on stdin/stdout until it disconnects. stdout carries protocol only.
pub async fn serve_stdio(server: FluxwireServer) -> anyhow::Result<()> {
    let running = server
        .serve((tokio::io::stdin(), tokio::io::stdout()))
        .await?;
    running.waiting().await?;
    Ok(())
}

/// `$XDG_RUNTIME_DIR/fluxwire/mcp-<pid>.sock`, else the per-user temp dir (macOS).
pub fn default_socket_path() -> PathBuf {
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    base.join("fluxwire")
        .join(format!("mcp-{}.sock", std::process::id()))
}

#[cfg(unix)]
pub use unix::{SocketGuard, bind_unix, serve_unix};

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use tokio::net::{UnixListener, UnixStream};

    /// Removes the socket file when dropped.
    pub struct SocketGuard(PathBuf);

    impl SocketGuard {
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for SocketGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Binds `path` (mode 0600). A leftover file nobody listens on is replaced; a live one is refused.
    pub async fn bind_unix(path: &Path) -> anyhow::Result<(UnixListener, SocketGuard)> {
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        if path.exists() {
            match UnixStream::connect(path).await {
                Ok(_) => anyhow::bail!(
                    "socket {} is already in use by a live server",
                    path.display()
                ),
                Err(_) => std::fs::remove_file(path)?,
            }
        }
        let listener = UnixListener::bind(path)?;
        let guard = SocketGuard(path.to_path_buf());
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok((listener, guard))
    }

    /// Accepts clients until `shutdown` resolves; each gets its own session on a clone of the server.
    pub async fn serve_unix(
        listener: UnixListener,
        server: FluxwireServer,
        shutdown: impl std::future::Future<Output = ()>,
    ) -> anyhow::Result<()> {
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                _ = &mut shutdown => return Ok(()),
                accepted = listener.accept() => {
                    let (stream, _) = accepted?;
                    let s = server.clone();
                    tokio::spawn(async move {
                        match s.serve(stream).await {
                            Ok(running) => { let _ = running.waiting().await; }
                            Err(e) => eprintln!("fluxwire mcp: session failed to start: {e}"),
                        }
                    });
                }
            }
        }
    }
}
