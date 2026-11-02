//! `serve`: runs the chosen transports until the client leaves or the process is stopped.

use crate::{FluxwireServer, Roots, ServeConfig};

pub async fn serve(cfg: ServeConfig) -> anyhow::Result<()> {
    let roots = Roots::new(&cfg.roots)?;
    let server = FluxwireServer::new(cfg.allow.clone(), cfg.transport.names(), roots);

    #[cfg(unix)]
    let unix = if cfg.transport.unix() {
        let path = cfg
            .socket
            .clone()
            .unwrap_or_else(crate::transport::default_socket_path);
        let (listener, guard) = crate::transport::bind_unix(&path).await?;
        eprintln!("fluxwire mcp: listening on {}", guard.path().display());
        Some((listener, guard))
    } else {
        None
    };
    #[cfg(not(unix))]
    anyhow::ensure!(
        !cfg.transport.unix(),
        "the unix transport is not available on this platform"
    );

    let stop = async {
        wait_for_signal().await;
    };
    match (cfg.transport.stdio(), cfg.transport.unix()) {
        (true, false) => {
            tokio::select! {
                r = crate::transport::serve_stdio(server) => r,
                _ = stop => Ok(()),
            }
        }
        #[cfg(unix)]
        (false, true) => {
            let (listener, _guard) = unix.unwrap();
            crate::transport::serve_unix(listener, server, stop).await
        }
        #[cfg(unix)]
        (true, true) => {
            let (listener, _guard) = unix.unwrap();
            // The socket keeps serving after stdin closes; a signal ends both.
            let stdio = crate::transport::serve_stdio(server.clone());
            let sock = crate::transport::serve_unix(listener, server, stop);
            tokio::pin!(stdio, sock);
            let mut stdio_done = false;
            loop {
                tokio::select! {
                    r = &mut sock => return r,
                    r = &mut stdio, if !stdio_done => {
                        stdio_done = true;
                        if let Err(e) = r { eprintln!("fluxwire mcp: stdio ended: {e}"); }
                    }
                }
            }
        }
        _ => unreachable!("transport config always selects one"),
    }
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
}
