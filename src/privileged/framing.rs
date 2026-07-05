use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, String> {
    let length = reader
        .read_u32()
        .await
        .map_err(|error| format!("read frame length: {error}"))? as usize;
    if length == 0 || length > shared::trusted::MAX_TRUSTED_FRAME_BYTES {
        return Err("invalid trusted frame length".into());
    }
    let mut body = vec![0; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|error| format!("read trusted frame: {error}"))?;
    Ok(body)
}

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, body: &[u8]) -> Result<(), String> {
    if body.is_empty() || body.len() > shared::trusted::MAX_TRUSTED_FRAME_BYTES {
        return Err("invalid trusted frame length".into());
    }
    writer
        .write_u32(body.len() as u32)
        .await
        .map_err(|error| format!("write frame length: {error}"))?;
    writer
        .write_all(body)
        .await
        .map_err(|error| format!("write trusted frame: {error}"))?;
    writer
        .flush()
        .await
        .map_err(|error| format!("flush trusted frame: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn framing_roundtrip_and_oversize_rejection() {
        let (mut a, mut b) = tokio::io::duplex(128);
        let writer = tokio::spawn(async move { write_frame(&mut a, b"hello").await });
        assert_eq!(read_frame(&mut b).await.unwrap(), b"hello");
        writer.await.unwrap().unwrap();

        let mut encoded = (shared::trusted::MAX_TRUSTED_FRAME_BYTES as u32 + 1)
            .to_be_bytes()
            .to_vec();
        encoded.push(0);
        assert!(read_frame(&mut encoded.as_slice()).await.is_err());
    }
}
