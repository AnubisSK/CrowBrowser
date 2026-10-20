//! Minimal X.509 (DER) reader for the site-info popover: names, validity, fingerprint.
//! Only reads fields; trust is decided by WebKit, never here.

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize, Default)]
pub struct CertSummary {
    pub subject_cn: String,
    pub subject_o: String,
    pub issuer_cn: String,
    pub issuer_o: String,
    pub not_before: String,
    pub not_after: String,
    pub expired: bool,
    pub sha256: String,
}

/// (tag, content, rest)
fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, b) = b.split_first()?;
    let (&first, mut b) = b.split_first()?;
    let len = if first & 0x80 == 0 {
        first as usize
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || b.len() < n {
            return None;
        }
        let len = b[..n].iter().fold(0usize, |a, &x| (a << 8) | x as usize);
        b = &b[n..];
        len
    };
    (b.len() >= len).then(|| (tag, &b[..len], &b[len..]))
}

fn text(tag: u8, v: &[u8]) -> String {
    if tag == 0x1e {
        // BMPString: UTF-16BE
        let u: Vec<u16> = v.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return String::from_utf16_lossy(&u);
    }
    String::from_utf8_lossy(v).into_owned()
}

/// Name ::= SEQUENCE OF SET OF { OID, value } → (CN, O)
fn name(seq: &[u8]) -> (String, String) {
    let (mut cn, mut o) = (String::new(), String::new());
    let mut rest = seq;
    while let Some((_, set, r)) = tlv(rest) {
        rest = r;
        let mut inner = set;
        while let Some((_, atv, r2)) = tlv(inner) {
            inner = r2;
            let Some((0x06, oid, r3)) = tlv(atv) else { continue };
            let Some((tag, val, _)) = tlv(r3) else { continue };
            match oid {
                [0x55, 0x04, 0x03] if cn.is_empty() => cn = text(tag, val),
                [0x55, 0x04, 0x0a] if o.is_empty() => o = text(tag, val),
                _ => {}
            }
        }
    }
    (cn, o)
}

/// UTCTime / GeneralizedTime → ("DD.MM.YYYY", sortable YYYYMMDDhhmmss)
fn time(tag: u8, v: &[u8]) -> Option<(String, u64)> {
    let s = std::str::from_utf8(v).ok()?;
    let full = match tag {
        0x17 => {
            let yy: u32 = s.get(..2)?.parse().ok()?;
            format!("{}{}", if yy < 50 { 2000 + yy } else { 1900 + yy }, s.get(2..12)?)
        }
        0x18 => s.get(..14)?.to_string(),
        _ => return None,
    };
    let key: u64 = full.get(..14)?.parse().ok()?;
    Some((format!("{}.{}.{}", &full[6..8], &full[4..6], &full[..4]), key))
}

fn now_key() -> u64 {
    // Civil date from Unix time (Howard Hinnant's algorithm), UTC.
    let secs = crate::vault::now_ms() / 1000;
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = (rem / 3600, rem / 60 % 60, rem % 60);
    format!("{y:04}{m:02}{d:02}{h:02}{mi:02}{s:02}").parse().unwrap_or(0)
}

pub fn summarize(der: &[u8]) -> Option<CertSummary> {
    let (_, cert, _) = tlv(der)?;
    let (_, tbs, _) = tlv(cert)?;
    let mut r = tbs;
    let (tag, _, rest) = tlv(r)?;
    if tag == 0xa0 {
        r = rest; // explicit version
    }
    let (_, _, r) = tlv(r)?; // serial
    let (_, _, r) = tlv(r)?; // signature algorithm
    let (_, issuer, r) = tlv(r)?;
    let (_, validity, r) = tlv(r)?;
    let (_, subject, _) = tlv(r)?;
    let (t1, v1, vr) = tlv(validity)?;
    let (t2, v2, _) = tlv(vr)?;
    let (not_before, kb) = time(t1, v1)?;
    let (not_after, ka) = time(t2, v2)?;
    let (subject_cn, subject_o) = name(subject);
    let (issuer_cn, issuer_o) = name(issuer);
    let now = now_key();
    let sha256 = Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
    Some(CertSummary {
        subject_cn,
        subject_o,
        issuer_cn,
        issuer_o,
        not_before,
        not_after,
        expired: now < kb || now > ka,
        sha256,
    })
}
