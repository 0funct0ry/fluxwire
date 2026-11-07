---
title: The .flux.md format
description: The complete rules of the Literate HTTP format, from file anatomy to canonical formatting and diagnostics.
---

A request is a Markdown file whose name ends in `.flux.md`. It reads as a plain document on GitHub or in any editor,
and it is precise enough for Fluxwire, other tools, and AI agents to execute.

## Principles

1. **The container is CommonMark.** A file is YAML front matter followed by Markdown. Prose is documentation. Fenced
   code blocks with a recognized info string carry everything executable.
2. **Requests and expected responses are protocol messages.** For HTTP, the request is an HTTP message (method line,
   headers, blank line, body) and the expectation is an HTTP response message.
3. **Every block declares its job.** The info string names a language and a role (`ts test`, `http expect`). Nothing is
   inferred from position or content.
4. **The author's layout is kept.** Fluxwire never reorders blocks or re-renders prose. The app and the formatter edit
   only the spans they own, so hand edits, app edits, and agent edits all produce small diffs.
5. **Open for extension, safe by default.** New protocols, roles, and keys can be added without breaking existing
   files, and older versions refuse to run what they do not understand. See
   [Extending the format](../extending-the-format/).

Only files ending in `.flux.md` are requests. Other `.md` files, such as `README.md`, are shown as docs and never
executed.

## Example

````markdown
---
name: Create payment
operation: createPayment
tags: [smoke, payments]
auth: inherit
capture:
  paymentId: $.id
  location: header Location
---

# Create payment

Creates a payment. `Idempotency-Key` is generated on every send.

```http
POST {{baseUrl}}/v1/payments
    ?expand=customer
    # &debug=true
Content-Type: application/json
Idempotency-Key: {{idemKey}}
# X-Debug: 1

{ "amount": 4200, "currency": "INR", "customer": "{{customerId}}" }
```

```http expect
HTTP 201
Content-Type: application/json

{ "status": "requires_capture", "amount": 4200 }
```

```ts pre
fw.vars.set("idemKey", crypto.randomUUID());
```

```ts test
fw.test("has an id", () => fw.expect(fw.response.json()).toHaveProperty("id"));
```
````

## File anatomy

| Part | Rules |
|---|---|
| Front matter | Required. Starts the file, delimited by `---` lines. A restricted YAML subset. |
| Prose | Any CommonMark or GFM outside recognized fences. Rendered as docs. The first paragraph after the first heading (or the first paragraph when there is no heading) is the **description** used by OpenAPI sync. |
| Recognized fences | Fenced blocks (backticks or tildes, three or more) whose info string is recognized. Unrecognized fences are prose. |
| Encoding | UTF-8, LF on write (CRLF accepted on read), trailing newline. |

## Front matter

The front matter is a restricted YAML subset so it parses identically everywhere: block mappings and sequences, flow
sequences (`[a, b]`), plain, single-quoted, and double-quoted scalars, and comments. There are no anchors, aliases,
tags, or multi-document streams. Nesting is limited to one level below the top. Values are strings unless the key's
schema says otherwise, so `country: NO` stays the string `NO`. Anything that needs deeper structure goes in a
`yaml config` block.

| Key | Type | Applies to | Meaning |
|---|---|---|---|
| `requires` | list | all | Features this file needs to run. Maintained by Fluxwire. |
| `name` | string | request (required), folder, collection | Display name. |
| `kind` | string | all | `request` (default for `*.flux.md`), `folder`, or `collection`. Future kinds, such as `flow`, need `requires`. |
| `seq` | integer | request, folder | Ordering within the parent. |
| `tags` | list | request | Used by filters such as `--filter tag:smoke`. |
| `operation` | string | request | OpenAPI operationId or `METHOD /path`. |
| `deprecated` | bool | request | Mirrors OpenAPI `deprecated`. |
| `auth` | string or map | all | `inherit` (default), `none`, or a map with `type` and fields. |
| `path` | map | request | Values for `:name` path parameters; a parameter without an entry uses the variable of the same name. |
| `vars` | map | all | Variables set before the send. |
| `capture` | map | request | Variables set from the response: `$.json.path`, `header <Name>`, `cookie <name>`, `status`. |
| `settings` | map | all | `timeout_ms`, `follow_redirects`, `max_redirects`, `http_version`, `verify_tls`, and `run_mode` (folders). |
| `x-<name>` | any | all | Extension keys. Preserved, never interpreted by core. |

The generated [Front matter reference](../front-matter/) is produced from the published JSON Schema.

## Info strings and roles

```text
info-string = language [ SP role ] [ SP label ] *( SP attribute )
attribute   = key "=" value            ; value is a bare word or "quoted"
```

The **language** says how the content is written. The **role** says what Fluxwire does with it. A request language
with no role is the request itself; any other language with no role is prose.

| Info string | Content | Count | Meaning |
|---|---|---|---|
| `http` | HTTP request message | exactly one per request file | The request. |
| `ws` | HTTP upgrade request message | exactly one per request file | A WebSocket request. |
| `http expect` | HTTP response message | at most one | Expected response. |
| `http form` | `name: value` lines | at most one | `application/x-www-form-urlencoded` body. |
| `http multipart` | `name: value` and `name: @file(path)` lines | at most one | `multipart/form-data` body. |
| `http defaults` | header lines only | folder and collection files | Headers inherited by every request below. |
| `ts pre`, `js pre` | script | any number | Runs before the send, in file order. May carry a `title` attribute. |
| `ts post`, `js post` | script | any number | Runs after the response, in file order. May carry a `title` attribute. |
| `ts test`, `js test` | script | any number | Tests, in file order. |
| `<lang> message <label>` | message body | any number | Saved messages for session protocols such as WebSocket. |
| `yaml config` | YAML | at most one | Nested configuration for protocols and extensions. |
| `<lang> example [label]` | anything | any number | Documentation-only sample, never executed. |

Rules:

- Roles are a reserved vocabulary owned by Fluxwire. An unknown role produces warning `F201`. If the block is
  documentation, use `example` or remove the role.
- The roles `query`, `variables`, `arguments`, `schema`, `step`, `response`, `fixture`, `mock`, `stream`, `metadata`,
  `resource`, and `prompt` are **reserved** and must not be used for anything else.
- Script blocks accept an optional `title="..."` attribute, shown on the block's card in the Scripts tab, for example
  `` ```ts pre title="Store auth in header" ``. It is metadata only and never affects execution. A title cannot contain
  double quotes or line breaks.
- Extension roles start with `x-` (`yaml x-acme-lint`). They are preserved and ignored by core.

The generated [Roles and info strings](../roles/) page lists the exact tables for this version.

## The HTTP request block

```text
request-line     = method SP target [ SP "HTTP/" version ]
query-line       = 4SP ( "?" / "&" ) name [ "=" value ]       ; continuation of the target
header-line      = field-name ":" OWS field-value
comment-line     = "#" *char                                    ; in the head only
blank-line       ; separates head from body
body             = raw text | "< " path                         ; "< path" sends a file
```

- `method` is any HTTP token; `target` is a URL that may contain `{{variables}}` and `:name` path parameters.
- Query parameters can be written in the URL or as indented continuation lines, which keeps diffs to one line per
  parameter. The app keeps both forms in sync and writes continuation lines once there are two or more parameters.
- A comment line that would be a valid header or query line when uncommented (`# X-Debug: 1`, `# &debug=true`) is a
  **disabled entry**: shown as an unchecked row in the app and not sent. Other comment lines are kept as comments.
- The body is everything after the first blank line, byte for byte. `# ` lines in the body are body content.
- Multi-line header values (obsolete line folding) are not supported; a diagnostic suggests a single line.

## The expect block

An `http expect` block is an HTTP response message describing what the response must contain.

- **Status line:** `HTTP 201`, `HTTP 2xx` (class), or `HTTP *` (any). `HTTP/2 201` also checks the protocol version.
- **Headers:** each listed header must be present. The value must match exactly after interpolation; `{{*}}` matches any
  value; `~/regex/` matches a regular expression.
- **Body (optional):** a JSON body is a **partial deep match**. Every listed key must exist with an equal value, keys not
  listed are ignored, arrays match element by element with equal length, and `"{{*}}"` matches any value. A non-JSON
  body must equal the trimmed response body.
- Each check is reported as a test (`expect status`, `expect header Content-Type`, `expect body $.status`) beside
  `ts test` results.

Anything more complex belongs in a `ts test` block.

## Folders and collections

`folder.flux.md` inside a folder and `collection.flux.md` at the collection root use the same format, with `kind`
implied by the file name. They may contain front matter (`name`, `seq`, `auth`, `vars`, `settings`), one `http defaults`
block, `pre`, `post`, and `test` scripts, and prose. The prose is the folder's or collection's documentation.

````markdown
---
name: Payments
seq: 2
auth:
  type: bearer
  token: "{{accessToken}}"
settings:
  run_mode: sequential
---

# Payments

Card and UPI payments. Amounts are in the smallest currency unit.

```http defaults
X-Client: fluxwire
X-Merchant-Id: {{merchantId}}
```
````

`fluxwire.toml` at the collection root must declare `format = 1`; a higher number refuses to open.

## Canonical formatting

`fluxwire fmt` and the app's save path produce identical bytes. The formatter:

- Orders front matter keys: `requires`, `name`, `kind`, `seq`, `tags`, `operation`, `deprecated`, `auth`, `path`,
  `vars`, `capture`, `settings`, then all other keys in their original order. It quotes scalars only when the subset
  requires it.
- Normalizes the head of `http`, `ws`, `http expect`, `http form`, `http multipart`, and `http defaults` blocks: one
  space after `:`, a four-space indent for query continuation lines, and no trailing whitespace. Bodies, scripts,
  messages, and `yaml config` content are left byte for byte.
- Ensures one blank line before and after every fenced block, LF line endings, and a final newline.
- Never reorders blocks and never touches prose.

When the app adds a block a file does not have yet, it inserts it after the last existing block that precedes it in
this default order, or at the end: front matter, title, description, request block, `http expect`, `ts pre`,
`ts post`, `ts test`, `message` blocks, `yaml config`.

Guarantees, property-tested: `fmt(fmt(f)) == fmt(f)`, and parsing then serializing an unmodified document returns the
original bytes.

## Parser and diagnostics

Parsing has two layers. The outer layer uses `pulldown-cmark` with source offsets to find front matter and fenced
blocks without re-rendering anything. The inner layer has a hand-written, error-recovering parser for each block kind.
Edits are applied as span replacements on the original text, which is what makes the lossless guarantee hold.

Diagnostics carry a code, a span, a message, and a fix where one exists:

| Code | Example |
|---|---|
| `F104` | Header line has no colon. Fix: insert `:`. |
| `F201` | Unknown role. |
| `F301` | Two request blocks in one file. Fix: mark one as `example`. |
| `F401` | Feature in `requires` is not supported by this version. |
| `P101` | Unresolved `{{variable}}`. |

The app shows them inline and in the Problems panel; `fluxwire validate` prints them rustc-style.
