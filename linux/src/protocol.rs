/// Wire protocol for the Tunnel handshake.
///
/// Flow:
///   1. Sender connects via TLS.
///   2. Sender writes `Message::Ask` (JSON, length-prefixed).
///   3. Receiver shows confirmation dialog.
///   4. Receiver writes `Message::Response`.
///   5. If ACCEPTED: sender streams the raw file bytes.
///   6. Sender writes `Message::Done` with SHA-256 checksum.
///   7. Receiver verifies checksum, writes `Message::Response` (ACK).
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const PROTOCOL_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Message {
    /// Sender → Receiver: "I want to send you this file."
    Ask {
        version: u8,
        transfer_id: String,
        sender_name: String,
        file_name: String,
        size_bytes: u64,
    },
    /// Receiver → Sender: accept or deny.
    Response {
        version: u8,
        status: ResponseStatus,
    },
    /// Sender → Receiver: after file bytes, the SHA-256 digest.
    Done {
        checksum_sha256: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseStatus {
    Accepted,
    Denied,
    ChecksumOk,
    ChecksumFail,
}

/// Write a length-prefixed JSON message to any async writer.
pub async fn write_message<W>(writer: &mut W, msg: &Message) -> Result<()>
where
    W: AsyncWriteExt + Unpin,
{
    let json = serde_json::to_vec(msg)?;
    let len = json.len() as u32;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(&json).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a length-prefixed JSON message from any async reader.
pub async fn read_message<R>(reader: &mut R) -> Result<Message>
where
    R: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    reader.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > 1024 * 1024 {
        bail!("Protocol message too large ({len} bytes)");
    }

    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}
