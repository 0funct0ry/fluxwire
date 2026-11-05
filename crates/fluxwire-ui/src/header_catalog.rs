//! Standard HTTP header names and their common values, for the Headers table popup. Pure
//! data and lookups (no GPUI); names are in canonical casing and matched case-insensitively.

/// (name, one-line description) of the headers offered while typing a Key.
pub const HEADERS: &[(&str, &str)] = &[
    ("Accept", "Media types the client accepts"),
    ("Accept-Charset", "Character sets the client accepts"),
    ("Accept-Encoding", "Content encodings the client accepts"),
    ("Accept-Language", "Natural languages the client prefers"),
    (
        "Access-Control-Request-Headers",
        "Headers a CORS preflight will send",
    ),
    (
        "Access-Control-Request-Method",
        "Method a CORS preflight will use",
    ),
    ("Authorization", "Credentials for the server"),
    ("Cache-Control", "Caching directives"),
    ("Connection", "Connection options"),
    ("Content-Disposition", "How the body is presented or saved"),
    ("Content-Encoding", "Encoding applied to the body"),
    ("Content-Language", "Language of the body"),
    ("Content-Length", "Size of the body in bytes"),
    ("Content-Location", "Alternate location of the body"),
    ("Content-MD5", "Base64 MD5 digest of the body"),
    ("Content-Range", "Where a partial body belongs"),
    ("Content-Type", "Media type of the body"),
    ("Cookie", "Cookies sent to the server"),
    ("Date", "When the message was sent"),
    ("DNT", "Do Not Track preference"),
    ("Expect", "Behaviour the server must support"),
    ("Forwarded", "Standard proxy information"),
    ("From", "Email address of the requester"),
    ("Host", "Host and port of the server"),
    ("If-Match", "Proceed only if the ETag matches"),
    ("If-Modified-Since", "Proceed only if changed since"),
    ("If-None-Match", "Proceed only if no ETag matches"),
    ("If-Range", "Range request only if unchanged"),
    ("If-Unmodified-Since", "Proceed only if not changed since"),
    ("Idempotency-Key", "Key that makes a POST safely repeatable"),
    ("Keep-Alive", "Parameters for a persistent connection"),
    ("Max-Forwards", "Limit on proxy hops"),
    ("Origin", "Origin that started a cross-site request"),
    ("Pragma", "Legacy caching directive"),
    ("Prefer", "Preferred server behaviour"),
    ("Proxy-Authorization", "Credentials for a proxy"),
    ("Range", "Part of the resource to return"),
    ("Referer", "Address of the linking page"),
    ("Referrer-Policy", "How much referrer information to send"),
    ("Sec-Fetch-Dest", "Destination of the request"),
    ("Sec-Fetch-Mode", "Mode of the request"),
    ("Sec-Fetch-Site", "Relationship of the request origin"),
    ("TE", "Transfer encodings the client accepts"),
    ("Trailer", "Fields sent after the body"),
    ("Transfer-Encoding", "Encoding used to transfer the body"),
    ("Upgrade", "Protocols the client can switch to"),
    ("Upgrade-Insecure-Requests", "Prefers an encrypted response"),
    ("User-Agent", "Identifies the client software"),
    ("Via", "Proxies the message passed through"),
    ("Warning", "Information about possible problems"),
    ("X-API-Key", "API key"),
    ("X-Correlation-Id", "Id that ties related requests together"),
    ("X-Forwarded-For", "Original client address behind a proxy"),
    ("X-Forwarded-Host", "Original Host behind a proxy"),
    ("X-Forwarded-Proto", "Original protocol behind a proxy"),
    (
        "X-HTTP-Method-Override",
        "Method to use instead of the real one",
    ),
    ("X-Request-Id", "Id of this request"),
    ("X-Requested-With", "Marks an Ajax request"),
];

const MIME: &[&str] = &[
    "application/json",
    "application/xml",
    "application/x-www-form-urlencoded",
    "multipart/form-data",
    "text/plain",
    "text/html",
    "text/csv",
    "application/octet-stream",
    "application/pdf",
    "application/graphql",
    "application/javascript",
    "application/gzip",
    "application/zip",
    "application/atom+xml",
    "application/ecmascript",
    "application/font-woff",
    "application/ogg",
    "application/json-patch+json",
    "application/merge-patch+json",
    "application/vnd.api+json",
    "application/problem+json",
    "application/x-ndjson",
    "image/gif",
    "image/jpeg",
    "image/png",
    "image/svg+xml",
    "image/webp",
    "text/css",
    "text/event-stream",
    "text/markdown",
    "text/xml",
];

const ENCODINGS: &[&str] = &["gzip", "deflate", "br", "zstd", "identity", "*"];

/// Common values for the header `name` (case-insensitive); empty for an unknown header.
pub fn values_for(name: &str) -> &'static [&'static str] {
    match name.trim().to_ascii_lowercase().as_str() {
        "content-type" | "accept" => MIME,
        "accept-encoding" => ENCODINGS,
        "content-encoding" => &["gzip", "deflate", "br", "zstd", "identity"],
        "accept-charset" => &["utf-8", "iso-8859-1", "*"],
        "accept-language" => &["en-US", "en-GB", "en", "fr-FR", "de-DE", "es-ES", "*"],
        "authorization" | "proxy-authorization" => &["Bearer ", "Basic ", "Digest ", "ApiKey "],
        "cache-control" => &[
            "no-cache",
            "no-store",
            "no-transform",
            "max-age=0",
            "max-age=3600",
            "must-revalidate",
            "public",
            "private",
            "only-if-cached",
        ],
        "connection" => &["keep-alive", "close", "Upgrade"],
        "pragma" => &["no-cache"],
        "upgrade" => &["websocket", "h2c"],
        "upgrade-insecure-requests" => &["1"],
        "dnt" => &["0", "1"],
        "expect" => &["100-continue"],
        "te" => &["trailers", "gzip", "deflate"],
        "transfer-encoding" => &["chunked", "gzip", "deflate", "identity"],
        "prefer" => &["return=minimal", "return=representation", "respond-async"],
        "referrer-policy" => &[
            "no-referrer",
            "same-origin",
            "origin",
            "strict-origin",
            "strict-origin-when-cross-origin",
            "unsafe-url",
        ],
        "sec-fetch-dest" => &["document", "empty", "image", "script", "style", "iframe"],
        "sec-fetch-mode" => &["cors", "navigate", "no-cors", "same-origin", "websocket"],
        "sec-fetch-site" => &["cross-site", "same-origin", "same-site", "none"],
        "x-requested-with" => &["XMLHttpRequest"],
        "x-http-method-override" => &["GET", "POST", "PUT", "PATCH", "DELETE"],
        "access-control-request-method" => &["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"],
        _ => &[],
    }
}

/// Canonical casing of a known header name, if `name` is one.
pub fn canonical(name: &str) -> Option<&'static str> {
    HEADERS
        .iter()
        .map(|(n, _)| *n)
        .find(|n| n.eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_canonical() {
        let mut seen = std::collections::HashSet::new();
        for (n, d) in HEADERS {
            assert!(seen.insert(n.to_ascii_lowercase()), "duplicate {n}");
            assert!(!d.is_empty());
            assert!(!n.contains(char::is_whitespace));
        }
    }

    #[test]
    fn values_follow_the_header_case_insensitively() {
        assert!(values_for("content-type").contains(&"application/json"));
        assert_eq!(values_for("Accept"), values_for("ACCEPT"));
        assert!(values_for("X-Unknown").is_empty());
    }

    #[test]
    fn every_header_with_values_is_listed() {
        for (n, _) in HEADERS {
            let _ = values_for(n);
        }
        assert_eq!(canonical("content-type"), Some("Content-Type"));
        assert_eq!(canonical("nope"), None);
    }
}
