//! Length-prefixed JSON framing for the daemon's Unix-socket protocol:
//! a 4-byte big-endian length followed by that many bytes of JSON. Shared by
//! the daemon, the TUI's socket client, and the web proxy.

use anyhow::{anyhow, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Read one frame. Returns `Ok(None)` on a clean EOF at a frame boundary.
pub async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(anyhow!("frame too large: {} bytes", len));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(Some(buf))
}

pub async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<()> {
    let len = (data.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}
