//! Cross-platform local IPC transport for the session daemon, web proxy, and
//! control channels. Unix uses filesystem domain sockets; Windows uses named
//! pipes. The length-prefixed framing in [`crate::ipc`] is generic over
//! `AsyncRead`/`AsyncWrite`, so only the listener, the connection type, and
//! endpoint naming differ per platform.
//!
//! An *endpoint* is an opaque address string: a filesystem path on Unix
//! (`~/.config/conduit/sessions/<name>.sock`) and a pipe name on Windows
//! (`\\.\pipe\conduit-...`). The session registry stores it verbatim.

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// A connected IPC stream. Every variant is `Unpin`, so the trait impls below
/// project the pin trivially with `Pin::new`.
pub enum Conn {
    #[cfg(unix)]
    Unix(tokio::net::UnixStream),
    #[cfg(windows)]
    PipeServer(tokio::net::windows::named_pipe::NamedPipeServer),
    #[cfg(windows)]
    PipeClient(tokio::net::windows::named_pipe::NamedPipeClient),
}

impl AsyncRead for Conn {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Conn::Unix(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(windows)]
            Conn::PipeServer(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(windows)]
            Conn::PipeClient(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for Conn {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            #[cfg(unix)]
            Conn::Unix(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(windows)]
            Conn::PipeServer(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(windows)]
            Conn::PipeClient(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Conn::Unix(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            Conn::PipeServer(s) => Pin::new(s).poll_flush(cx),
            #[cfg(windows)]
            Conn::PipeClient(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            #[cfg(unix)]
            Conn::Unix(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            Conn::PipeServer(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(windows)]
            Conn::PipeClient(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub use imp::{bind, connect, is_alive, Listener};

#[cfg(unix)]
mod imp {
    use std::path::PathBuf;

    use anyhow::{Context as _, Result};

    use super::Conn;

    /// A bound listener that cleans up its socket file when dropped.
    pub struct Listener {
        inner: tokio::net::UnixListener,
        path: PathBuf,
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            // Leave no stale socket behind so a future bind on the same path is
            // clean. (Best-effort: a hard kill skips Drop, but callers also
            // remove the file before re-binding.)
            let _ = std::fs::remove_file(&self.path);
        }
    }

    impl Listener {
        pub async fn accept(&mut self) -> Result<Conn> {
            let (stream, _) = self.inner.accept().await.context("socket accept failed")?;
            Ok(Conn::Unix(stream))
        }
    }

    pub async fn bind(endpoint: &str) -> Result<Listener> {
        let path = PathBuf::from(endpoint);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create socket dir for {endpoint}"))?;
        }
        // Clear any stale socket from a previous (crashed) run before binding.
        let _ = std::fs::remove_file(&path);
        let inner = tokio::net::UnixListener::bind(&path)
            .with_context(|| format!("failed to bind unix socket: {endpoint}"))?;
        // Local-only, owner-private: same trust model as the daemon's pid.
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Listener { inner, path })
    }

    pub async fn connect(endpoint: &str) -> Result<Conn> {
        let stream = tokio::net::UnixStream::connect(endpoint)
            .await
            .with_context(|| format!("failed to connect to {endpoint}"))?;
        Ok(Conn::Unix(stream))
    }

    pub fn is_alive(endpoint: &str) -> bool {
        std::os::unix::net::UnixStream::connect(endpoint).is_ok()
    }
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use anyhow::{Context as _, Result};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    use super::Conn;

    // Win32 error code; special-cased to avoid a windows-sys dependency.
    const ERROR_PIPE_BUSY: i32 = 231;

    /// A named-pipe listener. Named pipes have no on-disk artifact to clean up —
    /// the pipe disappears once the last handle closes.
    pub struct Listener {
        name: String,
        next: Option<NamedPipeServer>,
    }

    impl Listener {
        pub async fn accept(&mut self) -> Result<Conn> {
            // Hand out the staged instance once a client connects, then create
            // the next instance so the following accept() has one ready. This is
            // the idiomatic tokio named-pipe accept loop.
            let server = self
                .next
                .take()
                .expect("named pipe listener always holds a staged instance");
            server.connect().await.context("named pipe connect failed")?;
            self.next = Some(
                ServerOptions::new()
                    .create(&self.name)
                    .with_context(|| format!("failed to stage named pipe: {}", self.name))?,
            );
            Ok(Conn::PipeServer(server))
        }
    }

    pub async fn bind(endpoint: &str) -> Result<Listener> {
        let server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(endpoint)
            .with_context(|| format!("failed to create named pipe: {endpoint}"))?;
        Ok(Listener {
            name: endpoint.to_string(),
            next: Some(server),
        })
    }

    pub async fn connect(endpoint: &str) -> Result<Conn> {
        loop {
            match ClientOptions::new().open(endpoint) {
                Ok(client) => return Ok(Conn::PipeClient(client)),
                // All instances momentarily taken — the server is mid-accept.
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => {
                    return Err(anyhow::Error::from(e))
                        .with_context(|| format!("failed to connect to {endpoint}"));
                }
            }
        }
    }

    pub fn is_alive(endpoint: &str) -> bool {
        // Named pipes live in the filesystem namespace, so a plain blocking open
        // is a runtime-independent liveness probe (the tokio client needs an I/O
        // driver). A successful open consumes an instance; the daemon's accept
        // loop tolerates the instant close. ERROR_PIPE_BUSY means the pipe exists
        // but is saturated — still alive.
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(endpoint)
        {
            Ok(_) => true,
            Err(e) => e.raw_os_error() == Some(ERROR_PIPE_BUSY),
        }
    }
}
