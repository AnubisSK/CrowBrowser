//! Engine 1: Eco Reader. Fetches a page on the node and returns only its text structure and
//! recompressed images: no script, style, frame or third-party request reaches the client.

use crate::policy;
use base64::{Engine, engine::general_purpose::STANDARD};
use crow_proto::{Block, ReaderDoc, Reply, Span, Tx};
use image::{DynamicImage, GenericImageView, ImageReader, Limits};
use reqwest::{Client, header, redirect};
use scraper::{ElementRef, Html, Node, Selector};
use std::{collections::HashMap, io::Cursor, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::Semaphore,
    task::JoinSet,
};
use url::Url;
use zeroize::Zeroizing;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0 Safari/537.36";
const TIMEOUT: Duration = Duration::from_secs(25);
const MAX_REDIRECTS: usize = 8;
const MAX_PAGE: usize = 8 * 1024 * 1024;
const MAX_IMAGE: usize = 5 * 1024 * 1024;
const MAX_IMAGES: usize = 24;
const MAX_DECODE: u64 = 128 * 1024 * 1024;
const MAX_SIDE: u32 = 8192;
const IMAGE_WIDTH: u32 = 640;
const IMAGE_QUALITY: f32 = 50.0;
const IMAGE_WORKERS: usize = 4;
const MAX_DEPTH: usize = 96;

pub fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(TIMEOUT)
        .referer(false)
        .dns_resolver(Arc::new(policy::Dns))
        .redirect(redirect::Policy::custom(|a| {
            if a.previous().len() >= MAX_REDIRECTS || policy::check_url(a.url()).is_err() {
                a.stop()
            } else {
                a.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

/// Body of `url` up to `limit` bytes, with its final address and content type. A redirect the
/// policy refused ends as a 3xx here and fails the status check.
async fn fetch(http: &Client, url: Url, limit: usize) -> Result<(Url, String, Zeroizing<Vec<u8>>), &'static str> {
    policy::check_url(&url)?;
    let mut r = http.get(url).send().await.map_err(|_| "remote_err_fetch")?;
    if r.status().is_redirection() {
        return Err("remote_err_blocked");
    }
    if !r.status().is_success() {
        return Err("remote_err_fetch");
    }
    let kind = r.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
    let last = r.url().clone();
    let mut body = Zeroizing::new(Vec::new());
    while let Some(chunk) = r.chunk().await.map_err(|_| "remote_err_fetch")? {
        if body.len() + chunk.len() > limit {
            return Err("remote_err_too_large");
        }
        body.extend_from_slice(&chunk);
    }
    Ok((last, kind, body))
}

fn decode_text(body: &[u8], content_type: &str) -> Zeroizing<String> {
    let from_header = content_type.split("charset=").nth(1).map(|c| c.split(';').next().unwrap_or(c).trim().trim_matches('"').to_string());
    let from_meta = || {
        let head = String::from_utf8_lossy(&body[..body.len().min(2048)]).to_ascii_lowercase();
        let rest = head.split("charset=").nth(1)?;
        let label: String = rest.trim_start_matches(['"', '\'']).chars().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
        Some(label)
    };
    let encoding = from_header
        .or_else(from_meta)
        .and_then(|l| encoding_rs::Encoding::for_label(l.as_bytes()))
        .unwrap_or(encoding_rs::UTF_8);
    Zeroizing::new(encoding.decode(body).0.into_owned())
}

/// Image waiting for its bytes: index into `blocks` and absolute address.
struct Pending {
    block: usize,
    src: Url,
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("static selector")
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const JUNK_TAGS: &[&str] = &[
    "script", "style", "noscript", "template", "iframe", "frame", "frameset", "object", "embed", "applet", "svg", "canvas", "audio",
    "video", "form", "button", "input", "select", "textarea", "nav", "aside", "footer", "dialog", "menu", "link", "meta", "head",
];

const JUNK_WORDS: &[&str] = &[
    "ad", "ads", "advert", "adverts", "advertisement", "advertising", "sponsor", "sponsored", "promo", "banner", "cookie", "cookies",
    "consent", "newsletter", "subscribe", "share", "sharing", "social", "related", "recommended", "comment", "comments", "popup", "modal",
    "sidebar", "breadcrumb", "breadcrumbs", "paywall", "outbrain", "taboola",
];

fn junk_name(v: &str) -> bool {
    v.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| JUNK_WORDS.iter().any(|j| w.eq_ignore_ascii_case(j)))
}

fn is_junk(e: ElementRef<'_>) -> bool {
    let v = e.value();
    let style = v.attr("style").unwrap_or("").replace(' ', "").to_ascii_lowercase();
    JUNK_TAGS.contains(&v.name())
        || v.attr("hidden").is_some()
        || v.attr("aria-hidden") == Some("true")
        || style.contains("display:none")
        || style.contains("visibility:hidden")
        || matches!(v.attr("role"), Some("navigation" | "banner" | "contentinfo" | "complementary" | "dialog"))
        || v.attr("class").is_some_and(junk_name)
        || v.attr("id").is_some_and(junk_name)
}

/// Text of `e` without the junk inside it.
fn text_of(e: ElementRef<'_>, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for c in e.children() {
        match c.value() {
            Node::Text(t) => out.push_str(t),
            Node::Element(_) => {
                if let Some(ce) = ElementRef::wrap(c)
                    && !is_junk(ce)
                {
                    text_of(ce, out, depth + 1);
                }
            }
            _ => {}
        }
    }
}

fn plain(e: ElementRef<'_>) -> String {
    let mut s = String::new();
    text_of(e, &mut s, 0);
    clean(&s)
}

fn link_target(base: &Url, href: &str) -> Option<String> {
    let u = base.join(href.trim()).ok()?;
    (policy::check_url(&u).is_ok()).then(|| u.to_string())
}

#[derive(Clone, Default)]
struct Style {
    href: Option<String>,
    strong: bool,
    em: bool,
    code: bool,
}

fn push_text(out: &mut Vec<Span>, text: &str, st: &Style) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.href == st.href
        && (last.strong, last.em, last.code) == (st.strong, st.em, st.code)
    {
        last.text.push_str(text);
        return;
    }
    out.push(Span { text: text.to_string(), href: st.href.clone(), strong: st.strong, em: st.em, code: st.code });
}

fn inline(e: ElementRef<'_>, base: &Url, st: &Style, out: &mut Vec<Span>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for c in e.children() {
        match c.value() {
            Node::Text(t) => {
                let mut s = String::with_capacity(t.len());
                let mut space = false;
                for ch in t.chars() {
                    if ch.is_whitespace() {
                        if !space {
                            s.push(' ');
                        }
                        space = true;
                    } else {
                        s.push(ch);
                        space = false;
                    }
                }
                push_text(out, &s, st);
            }
            Node::Element(v) => {
                let Some(ce) = ElementRef::wrap(c) else { continue };
                if is_junk(ce) {
                    continue;
                }
                let mut next = st.clone();
                match v.name() {
                    "br" => {
                        push_text(out, "\n", st);
                        continue;
                    }
                    "a" => next.href = v.attr("href").and_then(|h| link_target(base, h)),
                    "b" | "strong" => next.strong = true,
                    "i" | "em" | "cite" => next.em = true,
                    "code" | "kbd" | "samp" | "tt" => next.code = true,
                    "img" | "picture" => continue,
                    _ => {}
                }
                inline(ce, base, &next, out, depth + 1);
            }
            _ => {}
        }
    }
}

fn trim_spans(mut spans: Vec<Span>) -> Vec<Span> {
    if let Some(s) = spans.first_mut() {
        s.text = s.text.trim_start().to_string();
    }
    if let Some(s) = spans.last_mut() {
        s.text = s.text.trim_end().to_string();
    }
    spans.retain(|s| !s.text.is_empty());
    spans
}

fn spans_of(e: ElementRef<'_>, base: &Url) -> Vec<Span> {
    let mut v = Vec::new();
    inline(e, base, &Style::default(), &mut v, 0);
    trim_spans(v)
}

fn dimension(e: ElementRef<'_>, name: &str) -> Option<u32> {
    e.value().attr(name)?.trim().trim_end_matches("px").parse().ok()
}

fn image_source(e: ElementRef<'_>) -> Option<&str> {
    let v = e.value();
    let srcset = || v.attr("srcset").or_else(|| v.attr("data-srcset")).and_then(|s| s.split(',').next()?.split_whitespace().next());
    [v.attr("data-src"), v.attr("data-lazy-src"), v.attr("data-original"), v.attr("src")]
        .into_iter()
        .flatten()
        .find(|s| !s.trim().is_empty() && !s.trim_start().to_ascii_lowercase().starts_with("data:"))
        .or_else(srcset)
}

struct Walk<'a> {
    base: &'a Url,
    blocks: Vec<Block>,
    images: Vec<Pending>,
}

impl Walk<'_> {
    fn image(&mut self, e: ElementRef<'_>) {
        if self.images.len() >= MAX_IMAGES || matches!((dimension(e, "width"), dimension(e, "height")), (Some(0..=2), _) | (_, Some(0..=2))) {
            return;
        }
        let Some(src) = image_source(e).and_then(|s| self.base.join(s.trim()).ok()) else { return };
        if policy::check_url(&src).is_err() || src.path().to_ascii_lowercase().ends_with(".svg") {
            return;
        }
        self.images.push(Pending { block: self.blocks.len(), src });
        self.blocks.push(Block::Image { data: String::new(), alt: clean(e.value().attr("alt").unwrap_or("")), width: 0, height: 0 });
    }

    fn paragraph(&mut self, e: ElementRef<'_>, quote: bool) {
        let spans = spans_of(e, self.base);
        if !spans.is_empty() {
            self.blocks.push(if quote { Block::Quote { spans } } else { Block::Paragraph { spans } });
        }
    }

    fn walk(&mut self, e: ElementRef<'_>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut loose: Vec<Span> = Vec::new();
        for c in e.children() {
            match c.value() {
                Node::Text(t) => push_text(&mut loose, &clean_keep_edges(t), &Style::default()),
                Node::Element(v) => {
                    let Some(ce) = ElementRef::wrap(c) else { continue };
                    if is_junk(ce) {
                        continue;
                    }
                    if !is_block(ce) {
                        inline_child(ce, self.base, &mut loose);
                        continue;
                    }
                    self.flush(&mut loose);
                    match v.name() {
                        "p" => {
                            self.paragraph(ce, false);
                            for img in ce.select(&sel("img")) {
                                self.image(img);
                            }
                        }
                        "blockquote" => self.paragraph(ce, true),
                        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                            let level = v.name().as_bytes()[1] - b'0';
                            let text = plain(ce);
                            if !text.is_empty() {
                                self.blocks.push(Block::Heading { level, text });
                            }
                        }
                        "pre" => {
                            let mut text = String::new();
                            text_of(ce, &mut text, 0);
                            if !text.trim().is_empty() {
                                self.blocks.push(Block::Code { text });
                            }
                        }
                        "ul" | "ol" => {
                            let li = sel(":scope > li");
                            let items: Vec<Vec<Span>> = ce.select(&li).filter(|l| !is_junk(*l)).map(|l| spans_of(l, self.base)).filter(|s| !s.is_empty()).collect();
                            if !items.is_empty() {
                                self.blocks.push(Block::List { ordered: v.name() == "ol", items });
                            }
                        }
                        "img" => self.image(ce),
                        _ => self.walk(ce, depth + 1),
                    }
                }
                _ => {}
            }
        }
        self.flush(&mut loose);
    }

    fn flush(&mut self, loose: &mut Vec<Span>) {
        let spans = trim_spans(std::mem::take(loose));
        if !spans.is_empty() {
            self.blocks.push(Block::Paragraph { spans });
        }
    }
}

fn clean_keep_edges(t: &str) -> String {
    let body = clean(t);
    if body.is_empty() {
        return if t.is_empty() { String::new() } else { " ".into() };
    }
    let lead = if t.starts_with(char::is_whitespace) { " " } else { "" };
    let tail = if t.ends_with(char::is_whitespace) { " " } else { "" };
    format!("{lead}{body}{tail}")
}

fn inline_child(e: ElementRef<'_>, base: &Url, out: &mut Vec<Span>) {
    let v = e.value();
    let mut st = Style::default();
    match v.name() {
        "a" => st.href = v.attr("href").and_then(|h| link_target(base, h)),
        "b" | "strong" => st.strong = true,
        "i" | "em" | "cite" => st.em = true,
        "code" | "kbd" | "samp" | "tt" => st.code = true,
        "br" => return push_text(out, "\n", &st),
        _ => {}
    }
    inline(e, base, &st, out, 1);
}

fn is_block(e: ElementRef<'_>) -> bool {
    const INLINE: &[&str] = &["a", "abbr", "b", "bdi", "bdo", "br", "cite", "code", "data", "dfn", "em", "i", "kbd", "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup", "time", "tt", "u", "var", "wbr"];
    if !INLINE.contains(&e.value().name()) {
        return true;
    }
    // An inline wrapper around block content (`<a><div>…`) is walked as a block.
    e.descendent_elements().any(|d| matches!(d.value().name(), "p" | "div" | "img" | "ul" | "ol" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "blockquote"))
}

fn link_text_len(e: ElementRef<'_>) -> usize {
    e.select(&sel("a")).map(|a| plain(a).len()).sum()
}

/// Node with the most article-like text: paragraph text scored to its parent and grandparent,
/// discounted by link density.
fn best_candidate(doc: &Html) -> Option<ElementRef<'_>> {
    let mut scores = HashMap::new();
    for p in doc.select(&sel("p, pre, blockquote")) {
        if p.ancestors().filter_map(ElementRef::wrap).any(is_junk) {
            continue;
        }
        let len = plain(p).len();
        if len < 25 {
            continue;
        }
        let score = 1.0 + (len as f64 / 100.0).min(3.0) + (len as f64) / 80.0;
        let mut parents = p.ancestors().filter_map(ElementRef::wrap);
        if let Some(a) = parents.next() {
            *scores.entry(a.id()).or_insert(0.0) += score;
        }
        if let Some(a) = parents.next() {
            *scores.entry(a.id()).or_insert(0.0) += score / 2.0;
        }
    }
    let best = scores
        .into_iter()
        .filter_map(|(id, s)| {
            let e = ElementRef::wrap(doc.tree.get(id)?)?;
            let total = plain(e).len().max(1);
            let density = link_text_len(e) as f64 / total as f64;
            let bonus = match e.value().name() {
                "article" | "main" => 1.5,
                _ => 1.0,
            };
            Some((e, s * (1.0 - density) * bonus))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(e, _)| e)?;
    Some(widen(doc, best))
}

fn article_text(root: ElementRef<'_>) -> usize {
    root.select(&sel("p, pre, blockquote")).filter(|p| !p.ancestors().filter_map(ElementRef::wrap).any(is_junk)).map(|p| plain(p).len()).sum()
}

/// Articles split into sections score one section highest: climbs until most of the page's
/// paragraph text is inside.
fn widen<'a>(doc: &'a Html, mut e: ElementRef<'a>) -> ElementRef<'a> {
    let total = article_text(doc.root_element());
    while article_text(e) * 10 < total * 7 {
        match e.parent().and_then(ElementRef::wrap) {
            Some(up) if !matches!(up.value().name(), "html") => e = up,
            _ => break,
        }
    }
    e
}

fn meta(doc: &Html, names: &[&str]) -> String {
    let m = sel("meta");
    for name in names {
        let found = doc.select(&m).find_map(|e| {
            let v = e.value();
            (v.attr("property") == Some(name) || v.attr("name") == Some(name)).then(|| v.attr("content")).flatten()
        });
        if let Some(c) = found.map(clean).filter(|c| !c.is_empty()) {
            return c;
        }
    }
    String::new()
}

/// Parses the page. Synchronous: the DOM is not `Send`.
fn extract(html: &str, url: &Url) -> (ReaderDoc, Vec<Pending>) {
    let doc = Html::parse_document(html);
    let title_tag = doc.select(&sel("title")).next().map(plain).unwrap_or_default();
    let mut out = ReaderDoc {
        url: url.to_string(),
        title: {
            let og = meta(&doc, &["og:title", "twitter:title"]);
            if og.is_empty() { title_tag } else { og }
        },
        byline: meta(&doc, &["author", "article:author", "byl", "parsely-author"]),
        site: meta(&doc, &["og:site_name", "application-name"]),
        description: meta(&doc, &["og:description", "description", "twitter:description"]),
        published: meta(&doc, &["article:published_time", "date", "parsely-pub-date"]),
        lang: doc.root_element().value().attr("lang").map(clean).unwrap_or_default(),
        blocks: Vec::new(),
    };
    let body = sel("body");
    let Some(root) = best_candidate(&doc).or_else(|| doc.select(&body).next()) else {
        return (out, Vec::new());
    };
    let mut w = Walk { base: url, blocks: Vec::new(), images: Vec::new() };
    w.walk(root, 0);
    if w.images.is_empty()
        && let Some(src) = doc.select(&sel("meta[property='og:image']")).next().and_then(|e| e.value().attr("content")).and_then(|s| url.join(s).ok()).filter(|u| policy::check_url(u).is_ok())
    {
        w.images.push(Pending { block: 0, src });
        w.blocks.insert(0, Block::Image { data: String::new(), alt: String::new(), width: 0, height: 0 });
    }
    let images = std::mem::take(&mut w.images);
    out.blocks = std::mem::take(&mut w.blocks);
    (out, images)
}

/// Decodes under memory limits, scales down and encodes as lossy WebP. `None` for tracking
/// pixels and anything undecodable.
fn to_webp(bytes: &[u8]) -> Option<(String, u32, u32)> {
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE);
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let (w, h) = img.dimensions();
    if w <= 2 || h <= 2 {
        return None;
    }
    let img: DynamicImage = if w > IMAGE_WIDTH { img.thumbnail(IMAGE_WIDTH, MAX_SIDE) } else { img };
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let encoded = webp::Encoder::from_rgba(rgba.as_raw(), w, h).encode(IMAGE_QUALITY);
    Some((STANDARD.encode(&*encoded), w, h))
}

async fn fill_images(http: &Client, doc: &mut ReaderDoc, images: Vec<Pending>) {
    let limit = Arc::new(Semaphore::new(IMAGE_WORKERS));
    let mut set = JoinSet::new();
    for p in images {
        let (http, limit) = (http.clone(), limit.clone());
        set.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            let (_, kind, body) = fetch(&http, p.src, MAX_IMAGE).await.ok()?;
            if kind.contains("svg") {
                return None;
            }
            let out = tokio::task::spawn_blocking(move || to_webp(&body)).await.ok()??;
            Some((p.block, out))
        });
    }
    while let Some(done) = set.join_next().await {
        if let Ok(Some((i, (b64, w, h)))) = done
            && let Some(Block::Image { data, width, height, .. }) = doc.blocks.get_mut(i)
        {
            *data = b64;
            *width = w;
            *height = h;
        }
    }
    doc.blocks.retain(|b| !matches!(b, Block::Image { data, .. } if data.is_empty()));
}

async fn build(http: &Client, url: &str) -> Result<ReaderDoc, &'static str> {
    let url = Url::parse(url).map_err(|_| "remote_err_blocked")?;
    let (last, kind, body) = fetch(http, url, MAX_PAGE).await?;
    if !(kind.is_empty() || kind.contains("html") || kind.contains("xml")) {
        return Err("remote_err_not_html");
    }
    let (mut doc, images) = tokio::task::spawn_blocking(move || {
        let text = decode_text(&body, &kind);
        extract(&text, &last)
    })
    .await
    .map_err(|_| "remote_err_fetch")?;
    fill_images(http, &mut doc, images).await;
    Ok(doc)
}

pub async fn run<S: AsyncRead + AsyncWrite + Unpin>(mut tx: Tx<S>, url: &str, http: &Client) {
    match build(http, url).await {
        Ok(doc) => {
            if tx.send_json(&Reply::Ok).await.is_ok() {
                let _ = tx.send_json(&doc).await;
            }
        }
        Err(code) => {
            let _ = tx.send_json(&Reply::Err { code: code.into() }).await;
        }
    }
    tx.close().await;
}
