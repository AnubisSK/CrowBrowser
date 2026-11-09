use crate::MIN_TOKEN;
use argon2::{Algorithm, Argon2, Params, Version};
use futures_util::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use serde::Serialize;
use snow::{Builder, HandshakeState, StatelessTransportState};
use std::{io, sync::Arc};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::{
    WebSocketStream, accept_async_with_config, client_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use zeroize::Zeroizing;

/// Mutual authentication by knowledge of the token, forward secrecy from the ephemeral keys.
const NOISE: &str = "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s";
const SALT: &[u8] = b"crow-remote-psk-v1";
const TAG: usize = 16;
/// Largest Noise message, minus its tag and the one-byte chunk header.
const CHUNK: usize = 65535 - TAG - 1;
/// Largest reassembled message (an Eco page with its images).
const MAX_MESSAGE: usize = 32 * 1024 * 1024;
const FINAL: u8 = 1;
const BINARY: u8 = 2;

/// 32-byte channel key of `token`. Argon2id keeps a captured handshake from being searched
/// offline for a guessable token; run off any UI or async thread.
pub fn derive_psk(token: &str) -> Zeroizing<[u8; 32]> {
    let params = Params::new(64 * 1024, 3, 1, Some(32)).expect("argon2 params");
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(token.as_bytes(), SALT, key.as_mut())
        .expect("argon2");
    key
}

/// `--token` value: `argon2:<64 hex digits>` (an already derived key) or the secret itself.
pub fn parse_token(value: &str) -> Result<Zeroizing<[u8; 32]>, &'static str> {
    let Some(hex) = value.strip_prefix("argon2:") else {
        if value.chars().count() < MIN_TOKEN {
            return Err("token too short (16 characters minimum)");
        }
        return Ok(derive_psk(value));
    };
    if hex.len() != 64 || !hex.is_ascii() {
        return Err("argon2: value must be 64 hex digits");
    }
    let mut key = Zeroizing::new([0u8; 32]);
    for (i, b) in key.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| "argon2: value must be 64 hex digits")?;
    }
    Ok(key)
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

fn builder(psk: &[u8; 32]) -> io::Result<Builder<'_>> {
    Builder::new(NOISE.parse().map_err(other)?)
        .psk(0, psk)
        .map_err(other)
}

fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(70_000))
        .max_frame_size(Some(70_000))
}

async fn next_binary<S: AsyncRead + AsyncWrite + Unpin>(
    ws: &mut WebSocketStream<S>,
) -> io::Result<Vec<u8>> {
    while let Some(m) = ws.next().await {
        match m.map_err(other)? {
            Message::Binary(b) => return Ok(b.into()),
            Message::Close(_) => break,
            _ => {}
        }
    }
    Err(io::ErrorKind::UnexpectedEof.into())
}

fn split<S: AsyncRead + AsyncWrite + Unpin>(
    ws: WebSocketStream<S>,
    hs: HandshakeState,
) -> io::Result<(Tx<S>, Rx<S>)> {
    let noise = Arc::new(hs.into_stateless_transport_mode().map_err(other)?);
    let (sink, stream) = ws.split();
    Ok((
        Tx {
            sink,
            noise: noise.clone(),
            nonce: 0,
        },
        Rx {
            stream,
            noise,
            nonce: 0,
            buf: Zeroizing::new(Vec::new()),
            kind: None,
        },
    ))
}

/// Client side: WebSocket upgrade over `stream`, then the Noise handshake.
pub async fn connect<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    authority: &str,
    psk: &[u8; 32],
) -> io::Result<(Tx<S>, Rx<S>)> {
    let (mut ws, _) =
        client_async_with_config(format!("ws://{authority}/crow"), stream, Some(ws_config()))
            .await
            .map_err(other)?;
    let mut hs = builder(psk)?.build_initiator().map_err(other)?;
    let mut out = [0u8; 128];
    let n = hs.write_message(&[], &mut out).map_err(other)?;
    ws.send(Message::binary(out[..n].to_vec()))
        .await
        .map_err(other)?;
    let msg = next_binary(&mut ws).await?;
    let mut sink = [0u8; 128];
    hs.read_message(&msg, &mut sink)
        .map_err(|_| io::Error::from(io::ErrorKind::PermissionDenied))?;
    split(ws, hs)
}

/// Server side. A wrong token ends here, before any request is read.
pub async fn accept<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    psk: &[u8; 32],
) -> io::Result<(Tx<S>, Rx<S>)> {
    let mut ws = accept_async_with_config(stream, Some(ws_config()))
        .await
        .map_err(other)?;
    let mut hs = builder(psk)?.build_responder().map_err(other)?;
    let msg = next_binary(&mut ws).await?;
    let mut sink = [0u8; 128];
    hs.read_message(&msg, &mut sink)
        .map_err(|_| io::Error::from(io::ErrorKind::PermissionDenied))?;
    let mut out = [0u8; 128];
    let n = hs.write_message(&[], &mut out).map_err(other)?;
    ws.send(Message::binary(out[..n].to_vec()))
        .await
        .map_err(other)?;
    split(ws, hs)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Json,
    Binary,
}

/// One received message; the plaintext is wiped on drop.
pub struct Frame {
    pub kind: Kind,
    pub data: Zeroizing<Vec<u8>>,
}

pub struct Tx<S> {
    sink: SplitSink<WebSocketStream<S>, Message>,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Tx<S> {
    pub async fn send_json<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        let body = Zeroizing::new(serde_json::to_vec(value)?);
        self.send(Kind::Json, &body).await
    }

    pub async fn send(&mut self, kind: Kind, data: &[u8]) -> io::Result<()> {
        let mut chunks = data.chunks(CHUNK).peekable();
        let mut plain = Zeroizing::new(Vec::with_capacity(data.len().min(CHUNK) + 1));
        loop {
            let chunk = chunks.next().unwrap_or(&[]);
            let last = chunks.peek().is_none();
            plain.clear();
            plain.push((u8::from(last) * FINAL) | (u8::from(kind == Kind::Binary) * BINARY));
            plain.extend_from_slice(chunk);
            let mut out = vec![0u8; plain.len() + TAG];
            let n = self
                .noise
                .write_message(self.nonce, &plain, &mut out)
                .map_err(other)?;
            out.truncate(n);
            self.nonce += 1;
            self.sink.send(Message::binary(out)).await.map_err(other)?;
            if last {
                return Ok(());
            }
        }
    }

    pub async fn close(&mut self) {
        let _ = self.sink.close().await;
    }
}

pub struct Rx<S> {
    stream: SplitStream<WebSocketStream<S>>,
    noise: Arc<StatelessTransportState>,
    nonce: u64,
    buf: Zeroizing<Vec<u8>>,
    kind: Option<Kind>,
}

impl<S: AsyncRead + AsyncWrite + Unpin> Rx<S> {
    /// Next whole message; `None` when the peer is gone or sent anything invalid. Cancel safe:
    /// a partly reassembled message stays in `self`.
    pub async fn recv(&mut self) -> Option<Frame> {
        loop {
            let msg = match self.stream.next().await?.ok()? {
                Message::Binary(b) => b,
                Message::Close(_) => return None,
                _ => continue,
            };
            let mut plain = Zeroizing::new(vec![0u8; msg.len().checked_sub(TAG)?]);
            let n = self.noise.read_message(self.nonce, &msg, &mut plain).ok()?;
            self.nonce += 1;
            let (&flags, body) = plain[..n].split_first()?;
            let kind = if flags & BINARY != 0 {
                Kind::Binary
            } else {
                Kind::Json
            };
            if *self.kind.get_or_insert(kind) != kind || self.buf.len() + body.len() > MAX_MESSAGE {
                return None;
            }
            self.buf.extend_from_slice(body);
            if flags & FINAL != 0 {
                self.kind = None;
                return Some(Frame {
                    kind,
                    data: std::mem::take(&mut self.buf),
                });
            }
        }
    }

    /// Next message, parsed as JSON.
    pub async fn recv_json<T: serde::de::DeserializeOwned>(&mut self) -> Option<T> {
        let f = self.recv().await?;
        (f.kind == Kind::Json)
            .then(|| serde_json::from_slice(&f.data).ok())
            .flatten()
    }
}

/// Relays `plain` through the channel until either side ends; buffers are wiped.
pub async fn relay<S, P>(plain: P, tx: &mut Tx<S>, rx: &mut Rx<S>)
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: AsyncRead + AsyncWrite + Unpin,
{
    let (mut r, mut w) = tokio::io::split(plain);
    let mut buf = Zeroizing::new(vec![0u8; 32 * 1024]);
    let up = async {
        loop {
            match r.read(&mut buf).await {
                Ok(n) if n > 0 => {
                    if tx.send(Kind::Binary, &buf[..n]).await.is_err() {
                        return;
                    }
                }
                _ => return,
            }
        }
    };
    let down = async {
        while let Some(f) = rx.recv().await {
            if f.kind != Kind::Binary || w.write_all(&f.data).await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        () = up => {}
        () = down => {}
    }
    let _ = w.shutdown().await;
    tx.close().await;
}
