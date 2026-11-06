//! Sections generated beneath a request's prose (SPEC §16). Never written to disk.

use fluxwire_model::{BlockData, Body, ExpectBlock, FluxDoc, StatusMatch};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// A two-column table (name, value).
    Table(Vec<(String, String)>),
    /// Sentences, one per line.
    Lines(Vec<String>),
    /// A code sample with its language (`json`, `xml`, `text`).
    Code { lang: String, text: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub content: Content,
}

/// The expect block in words.
pub fn expect_in_words(e: &ExpectBlock, doc: &FluxDoc) -> Vec<String> {
    let mut out = Vec::new();
    out.push(match e.status {
        StatusMatch::Exact(c) => format!("The status is {c}."),
        StatusMatch::Class(c) => format!("The status is in the {c}xx range."),
        StatusMatch::Any => "Any status is accepted.".to_string(),
    });
    for h in e.headers.iter().filter(|h| h.enabled) {
        out.push(format!("The `{}` header is `{}`.", h.name, h.value));
    }
    if let Body::Raw(s) = &e.body {
        let _ = doc;
        if !s.slice(&doc.source).trim().is_empty() {
            out.push("The body contains the fields of the sample.".to_string());
        }
    }
    out
}

/// Method and URL, parameters, headers, example body and expectations of `doc`.
pub fn generate(doc: &FluxDoc) -> Vec<Section> {
    let mut out = Vec::new();
    if let Some(r) = doc.request() {
        out.push(Section {
            title: "Request".into(),
            content: Content::Table(vec![
                ("Method".into(), r.method.clone()),
                ("URL".into(), r.target.clone()),
            ]),
        });
        let q: Vec<_> = r
            .query
            .iter()
            .filter(|q| q.enabled)
            .map(|q| (q.name.clone(), q.value.clone().unwrap_or_default()))
            .collect();
        if !q.is_empty() {
            out.push(Section {
                title: "Parameters".into(),
                content: Content::Table(q),
            });
        }
        let h: Vec<_> = r
            .headers
            .iter()
            .filter(|h| h.enabled)
            .map(|h| (h.name.clone(), h.value.clone()))
            .collect();
        if !h.is_empty() {
            out.push(Section {
                title: "Headers".into(),
                content: Content::Table(h),
            });
        }
        if let Body::Raw(s) = &r.body {
            let text = s.slice(&doc.source).trim_end().to_string();
            if !text.is_empty() {
                let ct = r
                    .headers
                    .iter()
                    .find(|h| h.name.eq_ignore_ascii_case("content-type"))
                    .map(|h| h.value.as_str())
                    .unwrap_or("");
                let lang = if ct.contains("json") || text.starts_with(['{', '[']) {
                    "json"
                } else if ct.contains("xml") {
                    "xml"
                } else {
                    "text"
                };
                out.push(Section {
                    title: "Example body".into(),
                    content: Content::Code {
                        lang: lang.into(),
                        text,
                    },
                });
            }
        }
    }
    for d in &doc.data {
        if let BlockData::Expect(e) = d {
            out.push(Section {
                title: "Expected response".into(),
                content: Content::Lines(expect_in_words(e, doc)),
            });
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_from_a_request() {
        let doc = fluxwire_format::parse(
            "---\nname: A\n---\n\n```http\nPOST {{baseUrl}}/v1/p\n    ?expand=customer\n    # &debug=true\nContent-Type: application/json\n# X-Off: 1\n\n{\"a\": 1}\n```\n\n```http expect\nHTTP 201\nContent-Type: application/json\n```\n",
        );
        let s = generate(&doc);
        let titles: Vec<_> = s.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Request",
                "Parameters",
                "Headers",
                "Example body",
                "Expected response"
            ]
        );
        assert_eq!(
            s[1].content,
            Content::Table(vec![("expand".into(), "customer".into())])
        );
        assert!(matches!(&s[3].content, Content::Code { lang, .. } if lang == "json"));
        let Content::Lines(l) = &s[4].content else {
            panic!()
        };
        assert_eq!(l[0], "The status is 201.");
        assert!(l[1].contains("Content-Type"));
    }

    #[test]
    fn no_request_no_sections() {
        assert!(generate(&fluxwire_format::parse("# Only prose\n")).is_empty());
    }
}
