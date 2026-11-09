//! Engine 3: encrypted relay. The target is reached from the node, so sites see its address.

use crate::policy;
use crow_proto::{Reply, Rx, Tx, relay, socks5};
use std::io::ErrorKind;
use tokio::{
    io::{AsyncRead, AsyncWrite, copy_bidirectional},
    net::TcpListener,
};

pub async fn run<S: AsyncRead + AsyncWrite + Unpin>(mut tx: Tx<S>, mut rx: Rx<S>, host: &str, port: u16) {
    match policy::connect(host, port).await {
        Ok(target) => {
            if tx.send_json(&Reply::Ok).await.is_ok() {
                relay(target, &mut tx, &mut rx).await;
            }
        }
        Err(e) => {
            let code = if e.kind() == ErrorKind::PermissionDenied { "remote_err_blocked" } else { "remote_err_fetch" };
            let _ = tx.send_json(&Reply::Err { code: code.into() }).await;
        }
    }
}

/// Loopback SOCKS5 for the headless browsers: all their traffic obeys the same policy.
pub async fn local_socks() -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok((host, port)) = socks5::handshake(&mut s).await else { return };
                match policy::connect(&host, port).await {
                    Ok(mut target) => {
                        if socks5::reply(&mut s, socks5::SUCCEEDED).await.is_ok() {
                            let _ = copy_bidirectional(&mut s, &mut target).await;
                        }
                    }
                    Err(e) => {
                        let code = if e.kind() == ErrorKind::PermissionDenied { socks5::NOT_ALLOWED } else { socks5::UNREACHABLE };
                        let _ = socks5::reply(&mut s, code).await;
                    }
                }
            });
        }
    });
    Ok(port)
}
