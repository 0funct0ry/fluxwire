//! Dynamic variables, resolved per send (SPEC §8.1).
//!
//! Arguments are the raw text between the parentheses (the interpolator resolves any nested
//! `{{variable}}` first). Functions that take a list split on commas; `$hmacSha256` splits on
//! the first comma only, so the value may contain commas.

use base64::Engine;
use hmac::{Hmac, Mac};
use md5::Md5;
use rand::RngExt;
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

const B62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const B36: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const NANO: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
/// Snowflake epoch (Twitter's): 2010-11-04T01:42:54.657Z.
const SNOWFLAKE_EPOCH_MS: u64 = 1_288_834_974_657;
/// KSUID epoch: 2014-05-13T16:53:20Z.
const KSUID_EPOCH: i64 = 1_400_000_000;

/// Resolve `$name` or `$name(args)`. `None` means unknown or invalid arguments.
pub(crate) fn resolve(expr: &str) -> Option<String> {
    let (name, args) = match expr.split_once('(') {
        Some((n, rest)) => (n.trim(), Some(rest.strip_suffix(')')?)),
        None => (expr.trim(), None),
    };
    match (name, args) {
        ("$uuid", None) => Some(uuid()),
        ("$timestamp", None) => Some(now_secs().to_string()),
        ("$timestamp", Some(a)) => Some((now_secs() + offset_secs(a)?).to_string()),
        ("$timestampMs", None) => Some(now_ms().to_string()),
        ("$isoTimestamp", None) => Some(iso(now_secs())),
        ("$isoTimestamp", Some(a)) => Some(iso(now_secs() + offset_secs(a)?)),
        ("$today", None) => Some(iso(now_secs())[..10].to_string()),
        ("$now", None) => Some(iso(now_secs())[11..19].to_string()),
        ("$httpDate", None) => Some(http_date(now_secs())),
        ("$randomInt", None) => Some(rand::rng().random_range(0..=1_000_000i64).to_string()),
        ("$randomInt", Some(a)) => {
            let (lo, hi) = a.split_once(',')?;
            let (lo, hi): (i64, i64) = (lo.trim().parse().ok()?, hi.trim().parse().ok()?);
            (lo <= hi).then(|| rand::rng().random_range(lo..=hi).to_string())
        }
        ("$randomFloat", None) => Some(format!("{:.4}", rand::rng().random::<f64>())),
        ("$randomFloat", Some(a)) => {
            let (lo, hi) = a.split_once(',')?;
            let (lo, hi): (f64, f64) = (lo.trim().parse().ok()?, hi.trim().parse().ok()?);
            (lo.is_finite() && hi.is_finite() && lo <= hi).then(|| {
                let v = if lo == hi {
                    lo
                } else {
                    rand::rng().random_range(lo..hi)
                };
                format!("{v:.4}")
            })
        }
        ("$randomBool", None) => Some(rand::rng().random::<bool>().to_string()),
        ("$randomChoice", Some(a)) => {
            let items: Vec<&str> = a
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            (!items.is_empty()).then(|| items[rand::rng().random_range(0..items.len())].to_string())
        }
        ("$randomString", Some(a)) => Some(random_chars(B62, len_arg(a, 4096)?)),
        ("$randomHex", Some(a)) => {
            let n = len_arg(a, 4096)?;
            Some(hex(&random_bytes(n.div_ceil(2)))[..n].to_string())
        }
        ("$randomBytes", Some(a)) => {
            let n = len_arg(a, 1024)?;
            Some(base64::engine::general_purpose::STANDARD.encode(random_bytes(n)))
        }
        ("$randomIp", None) => Some(random_ip()),
        ("$nanoid", None) => Some(random_chars(NANO, 21)),
        ("$nanoid", Some(a)) => Some(random_chars(NANO, len_arg(a, 256)?)),
        ("$shortId", None) => Some(random_chars(B62, 8)),
        ("$shortId", Some(a)) => {
            let n = len_arg(a, 32)?;
            (n >= 4).then(|| random_chars(B62, n))
        }
        ("$ulid", None) => Some(ulid()),
        ("$ksuid", None) => Some(ksuid()),
        ("$snowflakeId", None) => Some(snowflake()),
        ("$cuid", None) => Some(cuid()),
        ("$cuid2", None) => Some(cuid2()),
        ("$base64", Some(a)) => {
            Some(base64::engine::general_purpose::STANDARD.encode(a.as_bytes()))
        }
        ("$base64url", Some(a)) => {
            Some(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(a.as_bytes()))
        }
        ("$hex", Some(a)) => Some(hex(a.as_bytes())),
        ("$urlencode", Some(a)) => Some(pct_encode(a)),
        ("$sha256", Some(a)) => Some(hex(&Sha256::digest(a.as_bytes()))),
        ("$md5", Some(a)) => Some(hex(&Md5::digest(a.as_bytes()))),
        ("$hmacSha256", Some(a)) => {
            let (key, value) = a.split_once(',')?;
            let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).ok()?;
            mac.update(value.as_bytes());
            Some(hex(&mac.finalize().into_bytes()))
        }
        _ => None,
    }
}

/// A length argument in `1..=max`.
fn len_arg(a: &str, max: usize) -> Option<usize> {
    a.trim().parse().ok().filter(|n| (1..=max).contains(n))
}

/// `+1h`, `-30m`, `2d` as seconds; units `s`, `m`, `h`, `d`, `w`. Empty means no offset.
fn offset_secs(a: &str) -> Option<i64> {
    let a = a.trim();
    if a.is_empty() {
        return Some(0);
    }
    let (sign, rest) = match a.as_bytes()[0] {
        b'-' => (-1i64, &a[1..]),
        b'+' => (1, &a[1..]),
        _ => (1, a),
    };
    for (unit, mult) in [
        ('s', 1i64),
        ('m', 60),
        ('h', 3600),
        ('d', 86_400),
        ('w', 604_800),
    ] {
        if let Some(n) = rest.strip_suffix(unit) {
            let n: i64 = n.trim().parse().ok().filter(|n| *n >= 0)?;
            return n.checked_mul(mult)?.checked_mul(sign);
        }
    }
    None
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut r = rand::rng();
    (0..n).map(|_| r.random::<u8>()).collect()
}

fn random_chars(alphabet: &[u8], n: usize) -> String {
    let mut r = rand::rng();
    (0..n)
        .map(|_| alphabet[r.random_range(0..alphabet.len())] as char)
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn pct_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Big-endian base conversion of `bytes` to `alphabet`, left-padded with its first digit to
/// at least `width` digits.
fn encode_base(bytes: &[u8], alphabet: &[u8], width: usize) -> String {
    let base = alphabet.len() as u32;
    let mut num = bytes.to_vec();
    let mut out = Vec::new();
    while num.iter().any(|b| *b != 0) {
        let mut rem = 0u32;
        for b in num.iter_mut() {
            let cur = (rem << 8) | u32::from(*b);
            *b = (cur / base) as u8;
            rem = cur % base;
        }
        out.push(alphabet[rem as usize]);
    }
    while out.len() < width {
        out.push(alphabet[0]);
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

/// Base 36 of `n`, padded to `w` digits and cut to its last `w` digits.
fn b36(n: u64, w: usize) -> String {
    let s = encode_base(&n.to_be_bytes(), B36, w);
    if w > 0 && s.len() > w {
        s[s.len() - w..].to_string()
    } else {
        s
    }
}

fn random_ip() -> String {
    // Documentation ranges (RFC 5737): never routable.
    let mut r = rand::rng();
    let (a, b, c) = [(192, 0, 2), (198, 51, 100), (203, 0, 113)][r.random_range(0..3)];
    format!("{a}.{b}.{c}.{}", r.random_range(1..=254))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// ULID: 48-bit millisecond time and 80 random bits in Crockford base 32. Within one
/// millisecond the random part is incremented, so values from this process stay sorted.
fn ulid() -> String {
    static LAST: Mutex<(u64, u128)> = Mutex::new((0, 0));
    let mask: u128 = (1u128 << 80) - 1;
    let mut g = lock(&LAST);
    let mut ms = now_ms() as u64;
    let rnd = if ms <= g.0 {
        ms = g.0;
        (g.1 + 1) & mask
    } else {
        rand::rng().random::<u128>() & mask
    };
    *g = (ms, rnd);
    encode_base(&((u128::from(ms) << 80) | rnd).to_be_bytes(), CROCKFORD, 26)
}

/// KSUID: 32-bit seconds since the KSUID epoch and 128 random bits, 27 base-62 characters.
fn ksuid() -> String {
    let mut b = Vec::with_capacity(20);
    let t = (now_secs() - KSUID_EPOCH).clamp(0, i64::from(u32::MAX)) as u32;
    b.extend_from_slice(&t.to_be_bytes());
    b.extend(random_bytes(16));
    encode_base(&b, B62, 27)
}

/// Snowflake: 41-bit milliseconds since the Snowflake epoch, a 10-bit worker id (random per
/// process) and a 12-bit sequence, as a decimal string.
fn snowflake() -> String {
    static LAST: Mutex<(u64, u64)> = Mutex::new((0, 0));
    static WORKER: OnceLock<u64> = OnceLock::new();
    let worker = *WORKER.get_or_init(|| rand::rng().random_range(0..1024));
    let mut g = lock(&LAST);
    let mut ms = (now_ms() as u64).max(g.0);
    let mut seq = if ms == g.0 { g.1 + 1 } else { 0 };
    if seq >= 4096 {
        ms += 1;
        seq = 0;
    }
    *g = (ms, seq);
    (((ms.saturating_sub(SNOWFLAKE_EPOCH_MS)) << 22) | (worker << 12) | seq).to_string()
}

fn counter() -> u32 {
    static C: AtomicU32 = AtomicU32::new(0);
    C.fetch_add(1, Ordering::Relaxed) % 1_679_616
}

/// Four characters that identify this process.
fn fingerprint() -> &'static str {
    static F: OnceLock<String> = OnceLock::new();
    F.get_or_init(|| {
        format!(
            "{}{}",
            b36(u64::from(std::process::id()), 2),
            random_chars(B36, 2)
        )
    })
}

/// CUID (v1 layout): `c` + time + counter + fingerprint + 8 random characters, base 36.
/// Its author has deprecated it; prefer `$cuid2`.
fn cuid() -> String {
    format!(
        "c{}{}{}{}{}",
        b36(now_ms() as u64, 0),
        b36(u64::from(counter()), 4),
        fingerprint(),
        random_chars(B36, 4),
        random_chars(B36, 4)
    )
}

/// CUID2 format: 24 base-36 characters, the first a letter, the rest a SHA-256 of the time,
/// a counter, the fingerprint and fresh entropy. (The reference uses SHA-3; the format is
/// the same, the digest differs.)
fn cuid2() -> String {
    let first = (b'a' + rand::rng().random_range(0..26u8)) as char;
    let mut h = Sha256::new();
    h.update(now_ms().to_le_bytes());
    h.update(counter().to_le_bytes());
    h.update(fingerprint().as_bytes());
    h.update(random_bytes(32));
    let digest = encode_base(&h.finalize(), B36, 50);
    format!("{first}{}", &digest[..23])
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn uuid() -> String {
    let mut b: [u8; 16] = rand::rng().random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// (year, month, day) of a day count since 1970-01-01 (civil-from-days algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// RFC 7231 date, for example `Thu, 05 Nov 2026 14:03:22 GMT`.
fn http_date(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!(
        "{}, {d:02} {} {y:04} {:02}:{:02}:{:02} GMT",
        DAYS[(days + 4).rem_euclid(7) as usize],
        MONTHS[(m - 1) as usize],
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// RFC 3339 UTC from unix seconds.
pub(crate) fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_known_values() {
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn dynamic_shapes() {
        assert_eq!(resolve("$uuid").unwrap().len(), 36);
        assert_eq!(resolve("$randomString(16)").unwrap().len(), 16);
        let n: i64 = resolve("$randomInt(5,6)").unwrap().parse().unwrap();
        assert!((5..=6).contains(&n));
        assert_eq!(resolve("$base64(hi)").unwrap(), "aGk=");
        assert!(resolve("$nope").is_none());
        assert!(resolve("$randomInt(9,1)").is_none());
    }

    fn r(e: &str) -> String {
        resolve(e).unwrap_or_else(|| panic!("{e} did not resolve"))
    }

    #[test]
    fn date_and_time() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(1_700_000_000), "Tue, 14 Nov 2023 22:13:20 GMT");
        let (today, now) = (r("$today"), r("$now"));
        assert_eq!(today.len(), 10);
        assert_eq!(now.len(), 8);
        assert!(r("$timestampMs").parse::<i64>().unwrap() > 1_600_000_000_000);
        let base: i64 = r("$timestamp").parse().unwrap();
        let later: i64 = r("$timestamp(+1h)").parse().unwrap();
        assert!((later - base - 3600).abs() <= 1);
        let earlier: i64 = r("$timestamp(-2d)").parse().unwrap();
        assert!((base - earlier - 172_800).abs() <= 1);
        assert_eq!(r("$isoTimestamp(+1d)").len(), 20);
        assert_eq!(offset_secs(""), Some(0));
        assert_eq!(offset_secs("30m"), Some(1800));
        assert!(offset_secs("1x").is_none() && offset_secs("h").is_none());
        assert!(resolve("$timestamp(soon)").is_none());
    }

    #[test]
    fn random_values() {
        assert_eq!(r("$randomHex(7)").len(), 7);
        assert!(r("$randomHex(16)").chars().all(|c| c.is_ascii_hexdigit()));
        assert!(resolve("$randomHex(0)").is_none());
        let b = base64::engine::general_purpose::STANDARD
            .decode(r("$randomBytes(12)"))
            .unwrap();
        assert_eq!(b.len(), 12);
        let f: f64 = r("$randomFloat(2,3)").parse().unwrap();
        assert!((2.0..=3.0).contains(&f));
        assert!((0.0..=1.0).contains(&r("$randomFloat").parse::<f64>().unwrap()));
        assert!(resolve("$randomFloat(3,2)").is_none());
        assert!(["true", "false"].contains(&r("$randomBool").as_str()));
        assert!(["a", "b c"].contains(&r("$randomChoice(a, b c)").as_str()));
        assert!(resolve("$randomChoice( , )").is_none());
        assert_eq!(r("$nanoid").len(), 21);
        assert_eq!(r("$nanoid(10)").len(), 10);
        let ip = r("$randomIp");
        assert!(
            ["192.0.2.", "198.51.100.", "203.0.113."]
                .iter()
                .any(|p| ip.starts_with(p))
        );
        assert_eq!(r("$shortId").len(), 8);
        assert_eq!(r("$shortId(12)").len(), 12);
        assert!(resolve("$shortId(2)").is_none());
        assert_ne!(r("$randomString(16)"), r("$randomString(16)"));
    }

    #[test]
    fn identifiers() {
        let u = r("$ulid");
        assert_eq!(u.len(), 26);
        assert!(u.bytes().all(|b| CROCKFORD.contains(&b)));
        let ids: Vec<String> = (0..200).map(|_| r("$ulid")).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "ULIDs from one process sort in creation order");
        let k = r("$ksuid");
        assert_eq!(k.len(), 27);
        assert!(k.bytes().all(|b| B62.contains(&b)));
        assert_eq!(encode_base(&[0u8; 20], B62, 27), "0".repeat(27));
        assert_eq!(
            encode_base(&[0xff; 20], B62, 27),
            "aWgEPTl1tmebfsQzFP4bxwgy80V"
        );
        let s: Vec<u64> = (0..5000)
            .map(|_| r("$snowflakeId").parse().unwrap())
            .collect();
        assert!(s.windows(2).all(|w| w[0] < w[1]), "strictly increasing");
        assert!(s[0] < (1u64 << 63));
        let c = r("$cuid");
        assert!(c.starts_with('c') && c.len() == 25, "{c}");
        assert_ne!(c, r("$cuid"));
        let c2 = r("$cuid2");
        assert_eq!(c2.len(), 24);
        assert!(c2.as_bytes()[0].is_ascii_lowercase());
        assert!(c2.bytes().all(|b| B36.contains(&b)));
    }

    #[test]
    fn encoding_and_hashing() {
        assert_eq!(r("$hex(hi)"), "6869");
        assert_eq!(r("$urlencode(a b&c/é)"), "a%20b%26c%2F%C3%A9");
        assert_eq!(r("$base64url(\u{fb}\u{ff}?>)"), "w7vDvz8-");
        assert_eq!(
            r("$sha256(abc)"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(r("$md5(abc)"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            r("$hmacSha256(key,The quick brown fox jumps over the lazy dog)"),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        assert_eq!(r("$hmacSha256(k,a,b)"), r("$hmacSha256(k,a,b)"));
        assert!(resolve("$hmacSha256(nokey)").is_none());
        assert!(resolve("$sha256").is_none(), "needs an argument");
    }
}
