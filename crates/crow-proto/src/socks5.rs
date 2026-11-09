//! Server half of SOCKS5 (RFC 1928), no authentication, CONNECT only. Host names stay names, so
//! they are resolved by the far end.

use std::{
    io::{self, ErrorKind},
    net::{Ipv4Addr, Ipv6Addr},
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const SUCCEEDED: u8 = 0;
pub const NOT_ALLOWED: u8 = 2;
pub const UNREACHABLE: u8 = 4;
pub const UNSUPPORTED: u8 = 7;

/// Reads the greeting and the request: `(host, port)` of the CONNECT.
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> io::Result<(String, u16)> {
    let bad = || io::Error::from(ErrorKind::InvalidData);
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(bad());
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    s.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        s.write_all(&[5, 0xff]).await?;
        return Err(bad());
    }
    s.write_all(&[5, 0]).await?;

    let mut req = [0u8; 4];
    s.read_exact(&mut req).await?;
    if req[0] != 5 {
        return Err(bad());
    }
    if req[1] != 1 {
        reply(s, UNSUPPORTED).await?;
        return Err(bad());
    }
    let host = match req[3] {
        1 => {
            let mut a = [0u8; 4];
            s.read_exact(&mut a).await?;
            Ipv4Addr::from(a).to_string()
        }
        3 => {
            let n = usize::from(s.read_u8().await?);
            let mut a = vec![0u8; n];
            s.read_exact(&mut a).await?;
            String::from_utf8(a).map_err(|_| bad())?
        }
        4 => {
            let mut a = [0u8; 16];
            s.read_exact(&mut a).await?;
            Ipv6Addr::from(a).to_string()
        }
        _ => {
            reply(s, UNSUPPORTED).await?;
            return Err(bad());
        }
    };
    let port = s.read_u16().await?;
    Ok((host, port))
}

pub async fn reply<S: AsyncWrite + Unpin>(s: &mut S, code: u8) -> io::Result<()> {
    s.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await
}
