//! Wire protocol between CrowBrowser and `crow-server`: a Noise channel keyed by the shared
//! token, carried in binary WebSocket messages, and the messages sent through it.

mod channel;
pub mod socks5;

pub use channel::{Frame, Kind, Rx, Tx, accept, connect, derive_psk, parse_token, relay};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Shortest token accepted by the client and the server.
pub const MIN_TOKEN: usize = 16;

/// First message of every connection: what the connection is for.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Open {
    Ping,
    Eco { url: String },
    Rbi { url: String, width: u32, height: u32 },
    Tunnel { host: String, port: u16 },
}

/// Answer to `Open`. Error codes are `locales.json` keys.
#[derive(Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Err { code: String },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MouseKind {
    Move,
    Down,
    Up,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    None,
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyKind {
    Down,
    Up,
}

/// Client to server on an RBI connection.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RbiCommand {
    Navigate { url: String },
    Back,
    Forward,
    Reload,
    Stop,
    Resize { width: u32, height: u32 },
    Visible { on: bool },
    Mouse { kind: MouseKind, x: f64, y: f64, button: MouseButton, buttons: u32, clicks: u32, mods: u32 },
    Wheel { x: f64, y: f64, dx: f64, dy: f64, mods: u32 },
    Key { kind: KeyKind, key: String, code: String, text: String, vk: u32, mods: u32 },
}

/// Server to client on an RBI connection, next to the binary frames.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum RbiEvent {
    /// The main frame committed to `url` (the only event that moves the address bar).
    Committed { url: String },
    Title { title: String },
    Loading { on: bool },
    Error { code: String },
}

#[derive(Serialize, Deserialize, Default, Zeroize, ZeroizeOnDrop)]
pub struct Span {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strong: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub em: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub code: bool,
}

#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Block {
    Heading { level: u8, text: String },
    Paragraph { spans: Vec<Span> },
    Quote { spans: Vec<Span> },
    List { ordered: bool, items: Vec<Vec<Span>> },
    Code { text: String },
    /// `data`: base64 of a WebP file.
    Image { data: String, alt: String, width: u32, height: u32 },
}

/// Eco Reader result: the sanitized page.
#[derive(Serialize, Deserialize, Default, Zeroize, ZeroizeOnDrop)]
pub struct ReaderDoc {
    /// Final address after redirects.
    pub url: String,
    pub title: String,
    pub byline: String,
    pub site: String,
    pub description: String,
    pub published: String,
    pub lang: String,
    pub blocks: Vec<Block>,
}
