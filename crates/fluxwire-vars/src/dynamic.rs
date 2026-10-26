//! Dynamic variables, resolved per send (SPEC §8.1).

use base64::Engine;
use rand::RngExt;

/// Resolve `$name` or `$name(args)`. `None` means unknown.
pub(crate) fn resolve(expr: &str) -> Option<String> {
    let (name, args) = match expr.split_once('(') {
        Some((n, rest)) => (n.trim(), Some(rest.strip_suffix(')')?)),
        None => (expr.trim(), None),
    };
    match (name, args) {
        ("$uuid", None) => Some(uuid()),
        ("$timestamp", None) => Some(now_secs().to_string()),
        ("$isoTimestamp", None) => Some(iso(now_secs())),
        ("$randomInt", None) => Some(rand::rng().random_range(0..=1_000_000i64).to_string()),
        ("$randomInt", Some(a)) => {
            let (lo, hi) = a.split_once(',')?;
            let (lo, hi): (i64, i64) = (lo.trim().parse().ok()?, hi.trim().parse().ok()?);
            (lo <= hi).then(|| rand::rng().random_range(lo..=hi).to_string())
        }
        ("$randomString", Some(a)) => {
            let n: usize = a.trim().parse().ok().filter(|n| *n <= 4096)?;
            const CS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let mut r = rand::rng();
            Some(
                (0..n)
                    .map(|_| CS[r.random_range(0..CS.len())] as char)
                    .collect(),
            )
        }
        ("$base64", Some(a)) => {
            Some(base64::engine::general_purpose::STANDARD.encode(a.as_bytes()))
        }
        _ => None,
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
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

/// RFC 3339 UTC from unix seconds (civil-from-days algorithm).
pub(crate) fn iso(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
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
}
